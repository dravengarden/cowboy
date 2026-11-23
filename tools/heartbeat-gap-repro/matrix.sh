#!/usr/bin/env bash
# Runs inside the reproduction guest as root. One case = a broker, a worker
# and its loads for $DUR seconds; results go to $OUT/summary.jsonl.
set -euo pipefail
# Guest only: this script changes slice properties and starts units.
if [ ! -e /etc/hbrepro/guest ] || ! systemd-detect-virt --vm --quiet; then
    echo "matrix.sh runs only inside the reproduction guest" >&2
    exit 1
fi
DUR=${DUR:-180}
OUT=${OUT:-/root/results}
CPUS=$(nproc)
CG=/sys/fs/cgroup/cowboy.slice
mkdir -p "$OUT"
: >"$OUT/summary.jsonl"

counter() { # file key -> value, or empty when unreadable or malformed
    local value
    case "$2" in
    some | full) value=$(awk -v k="$2" '$1 == k { for (i = 2; i <= NF; i++) if ($i ~ /^total=/) { sub("total=", "", $i); print $i } }' "$1" 2>/dev/null) || value= ;;
    *) value=$(awk -v k="$2" '$1 == k { print $2 }' "$1" 2>/dev/null) || value= ;;
    esac
    # Decimal, no leading zeros, below 10^18 so bash arithmetic is exact.
    [[ $value =~ ^(0|[1-9][0-9]{0,17})$ ]] && echo "$value"
    return 0
}

# Journal lines of the unit's current invocation only.
unit_log() {
    local id
    id=$(systemctl show -p InvocationID --value "$1")
    [ -n "$id" ] || return 1
    journalctl -o cat "_SYSTEMD_INVOCATION_ID=$id"
}

load_command() {
    case "$1" in
    # Loads outlive the measurement window; the case stops them.
    cpu) echo "stress-ng --cpu $((CPUS * 8)) --timeout $((DUR + 60))s" ;;
    mem) echo "stress-ng --vm 4 --vm-bytes 2900M --vm-keep --timeout $((DUR + 60))s" ;; # total, not per worker
    io) echo "stress-ng --hdd 4 --hdd-bytes 256M --timeout $((DUR + 60))s" ;;
    esac
}

# name worker_slice "loads" bulk_bytes bulk_every_ms broker_delay_us memory_high
run_case() {
    local name=$1 slice=$2 loads=$3 bulk=$4 every=$5 delay=$6 high=$7
    local sock=/run/hbrepro-$name.sock wcg="$CG/$slice/hb-worker-$name.service"
    systemctl set-property --runtime cowboy-agents.slice "MemoryHigh=$high"
    systemd-run --quiet --unit="hb-broker-$name" -p CPUWeight=1000 -p MemoryLow=256M \
        hbrepro broker "$sock" "$delay"
    systemd-run --quiet --unit="hb-worker-$name" --slice="$slice" -p MemoryAccounting=yes \
        hbrepro worker "$sock" 10000 "$bulk" "$every" 1048576
    local i=0
    for load in $loads; do
        i=$((i + 1))
        # shellcheck disable=SC2046 # word-split the command on purpose
        systemd-run --quiet --unit="hb-load-$name-$i" --slice=cowboy-agents.slice $(load_command "$load")
    done
    sleep 2
    # The loaded window starts after startup and the counter baseline, so
    # receive and pressure metrics cover the same interval.
    local start_ms high0 mem0 cpu0 io0
    high0=$(counter "$CG/cowboy-agents.slice/memory.events" high)
    mem0=$(counter "$wcg/memory.pressure" full)
    cpu0=$(counter "$wcg/cpu.pressure" some)
    io0=$(counter "$wcg/io.pressure" full)
    start_ms=$(date +%s%3N)
    sleep "$DUR"
    local high1 mem1 cpu1 io1
    high1=$(counter "$CG/cowboy-agents.slice/memory.events" high)
    mem1=$(counter "$wcg/memory.pressure" full)
    cpu1=$(counter "$wcg/cpu.pressure" some)
    io1=$(counter "$wcg/io.pressure" full)
    local end_ms result=unknown valid=true unit state units
    end_ms=$(date +%s%3N)
    units=("hb-worker-$name" "hb-broker-$name")
    for ((n = 1; n <= i; n++)); do units+=("hb-load-$name-$n"); done
    # A unit that died early, or a failed query, is a failed observation,
    # not a quiet one.
    for unit in "${units[@]}"; do
        if ! state=$(systemctl show -p ActiveState --value "$unit"); then
            state=unknown
        fi
        [ "$unit" = "hb-worker-$name" ] && result=$state
        [ "$state" = active ] || valid=false
    done
    if ! unit_log "hb-broker-$name" >"$OUT/$name.broker.log" || ! unit_log "hb-worker-$name" >"$OUT/$name.worker.log"; then
        valid=false
    fi
    systemctl stop "${units[@]}" 2>/dev/null || true
    for unit in "${units[@]}"; do
        if [ "$(systemctl show -p ActiveState --value "$unit" 2>/dev/null)" = active ]; then
            echo "unit $unit survived its case; aborting so later cases are not contaminated" >&2
            exit 1
        fi
    done
    systemctl reset-failed 2>/dev/null || true
    # Keep only records inside the measurement window; a parse failure
    # invalidates the case instead of leaving partial data.
    { grep '^{' "$OUT/$name.broker.log" || true; } \
        | jq -c --argjson start "$start_ms" --argjson end "$end_ms" 'select(.recv_ms >= $start and .recv_ms <= $end)' \
            >"$OUT/$name.broker.jsonl" || valid=false
    { grep '^{' "$OUT/$name.worker.log" || true; } \
        | jq -c --argjson start "$start_ms" --argjson end "$end_ms" 'select(.done_ms >= $start and .done_ms <= $end)' \
            >"$OUT/$name.worker.jsonl" || valid=false
    delta() { if [ -n "$1" ] && [ -n "$2" ]; then echo $(($2 - $1)); else echo null; fi; }
    jq -cs --arg name "$name" --arg slice "$slice" --arg loads "$loads" --arg high "$high" \
        --argjson bulk "$bulk" --argjson delay "$delay" \
        --argjson start_ms "$start_ms" --argjson end_ms "$end_ms" --arg worker_state "$result" \
        --argjson valid "$valid" \
        --argjson high_events "$(delta "$high0" "$high1")" --argjson mem_full_us "$(delta "$mem0" "$mem1")" \
        --argjson cpu_some_us "$(delta "$cpu0" "$cpu1")" --argjson io_full_us "$(delta "$io0" "$io1")" \
        --slurpfile sent "$OUT/$name.worker.jsonl" '
        (map(.gap_ms) | .[1:]) as $gaps
        # Silence before the first and after the last heartbeat counts too.
        | (if length > 0 then .[0].recv_ms - $start_ms else $end_ms - $start_ms end) as $leading
        | (if length > 0 then $end_ms - .[-1].recv_ms else 0 end) as $trailing
        | {case: $name, valid: $valid, worker_state: $worker_state,
           observed_ms: ($end_ms - $start_ms),
           leading_silence_ms: $leading, trailing_silence_ms: $trailing,
           max_silence_ms: ([$leading, $trailing] + $gaps | max), worker_slice: $slice, loads: $loads, memory_high: $high,
           bulk_bytes: $bulk, broker_delay_us_per_64k: $delay,
           heartbeats: length,
           max_gap_ms: ($gaps | max // null),
           # Nearest rank: ceil(0.95 n) - 1, zero-based.
           p95_gap_ms: ($gaps | sort | if length > 0 then .[((length * 95 / 100) | ceil) - 1] else null end),
           gaps_over_20s: ($gaps | map(select(. > 20000)) | length),
           gaps_over_45s: ($gaps | map(select(. > 45000)) | length),
           max_wake_late_ms: (map(.frame.late_ms | select(. != null)) | max // null),
           max_overdue_ms: (map(.frame.overdue_ms) | max // null),
           max_send_latency_ms: ($sent | map(.send_latency_ms) | max // null),
           max_alloc_ms: (map(.frame.alloc_ms) | max // null),
           max_lock_ms: (map(.frame.lock_ms) | max // null),
           max_write_ms: ($sent | map(.write_ms) | max // null),
           slice_memory_high_events: $high_events,
           worker_memory_full_us: $mem_full_us,
           worker_cpu_some_us: $cpu_some_us,
           worker_io_full_us: $io_full_us}' "$OUT/$name.broker.jsonl" >>"$OUT/summary.jsonl"
    tail -1 "$OUT/summary.jsonl"
}

#        name          worker slice          loads        bulk     every delay  high
run_case baseline      cowboy-agents.slice   ""           0        0     0      2G
run_case cpu           cowboy-agents.slice   "cpu"        0        0     0      2G
run_case memory-high   cowboy-agents.slice   "mem"        0        0     0      2G
run_case io            cowboy-agents.slice   "io"         0        0     0      2G
run_case combined      cowboy-agents.slice   "cpu mem io" 0        0     0      2G
run_case frame-block   cowboy-agents.slice   ""           33554000 1000  100000 2G
run_case split-memory  cowboy-control.slice  "mem"        0        0     0      2G
run_case max-only      cowboy-agents.slice   "mem"        0        0     0      infinity
