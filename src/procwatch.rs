//! Detect agent-spawned **background processes** still alive *between* turns.
//!
//! ## The gap this closes
//!
//! When the agent backgrounds work (`run_in_background` bash, a build it polls
//! for) it ends the ACP turn (`stop_reason: EndTurn`) and yields — the process
//! keeps running, and the harness re-invokes the agent when it finishes. Between
//! those turns cowboy sees the session as non-busy with no in-flight ACP tool,
//! so the UI shows "Waiting for your reply" / "Queue paused" even though real
//! work is still running. ACP carries no signal for this (verified against the
//! spec + the just-released 1.0 SDK: no background/activity state, `StopReason`
//! has no "waiting"). The agent's own `run_in_background` task is invisible to
//! the protocol — but it IS in the agent's cgroup. The cgroup is the only
//! ground truth, so we read it directly.
//!
//! ## Why a heuristic, and why this one
//!
//! The cgroup also holds the agent's OWN infrastructure — the ACP launch chain
//! (`npm → node → claude`) plus any persistent MCP servers (`chrome-devtools-mcp`,
//! …). Naively counting processes is unreliable: that baseline varies per agent
//! (claude vs codex vs future) and per MCP config (servers come and go, lazily).
//! So we don't count — we **classify**, and learn the baseline from the running
//! system rather than hardcoding any agent/MCP knowledge:
//!
//! 1. **Seed** `infra` from what's alive while the session is idle and BEFORE
//!    its first turn: everything then is infrastructure — agent chain + startup
//!    MCP. No per-agent list. Crucially this is NOT a single-instant snapshot:
//!    a background task can only be spawned by a tool *during a turn* (see the
//!    gap above), so until the session has been Busy even once we keep folding
//!    every live pid into the baseline. This absorbs the startup storm — Claude
//!    Code lazily spawns MCP servers (`chrome-devtools-mcp`, …) that appear
//!    AFTER the first tick and burn CPU/I/O while they initialize; a one-shot
//!    seed would miss them and flag them as a "background task" on a session
//!    that has never run a turn.
//! 2. A cgroup process not in `infra` is a **candidate** (spawned by a tool).
//! 3. **Promotion** absorbs config drift: a candidate that has been *quiescent*
//!    — no CPU growth (≈0 jiffies) AND no I/O growth (`rchar+wchar`) — *since we
//!    first saw it*, for [`PROMOTE_QUIESCENT`], is folded into `infra`: a
//!    lazily-started MCP / idle helper just sits there, so it self-classifies as
//!    infrastructure. The "since we first saw it" is load-bearing: a candidate
//!    that has EVER shown activity is a genuine spawned task and is NEVER
//!    promoted, even after it later goes quiescent — otherwise a background task
//!    that works in bursts with lulls (e.g. narration synthesizing chapters
//!    batch-by-batch) gets mis-promoted into `infra` during a >10s gap and then
//!    never flags again when the next burst starts, so the UI shows "Task
//!    complete" while real work is still running. I/O is in the liveness test
//!    because the tasks that end a turn are
//!    often I/O-bound, not CPU-bound (an upload, a download, a device install):
//!    they'd read as idle on CPU alone, but their `read()`/`write()` byte
//!    counters — which include sockets, not just disk — keep climbing.
//! 4. **A session is flagged** when a non-promoted candidate has outlived the
//!    [`SETTLE`] delay (which lets a tool's transient children exit first).
//!
//! Reliability details learned the hard way on hawk:
//! - Process **identity is `(pid, starttime)`**, not the bare pid — pids are
//!   reused; the start-time (an opaque, immutable token here) disambiguates.
//! - `/proc/<pid>/stat` MUST be parsed by splitting **after the last `)`** — the
//!   `comm` field is `(name)` and can contain spaces/parens (`(npm exec @agent)`),
//!   so a naive whitespace split shifts every later field. This bit us in
//!   testing (a bogus start-time of `11`).
//! - Detached strays are reaped by the agent at the next turn boundary, so the
//!   only things that survive between turns are infra + genuine harness-managed
//!   background tasks — which keeps the candidate set clean.
//!
//! Known corners (documented, accepted for v1): a task that produces NO local
//! signal at all — neither CPU nor I/O — for [`PROMOTE_QUIESCENT`] can be
//! mis-promoted. The residual case after adding the I/O test is a task whose
//! work is entirely on another host behind a silent connection (an `ssh` to a
//! remote build that streams nothing back): the local process genuinely does
//! nothing, and is indistinguishable from an idle held connection — no local
//! observer can tell them apart. On a daemon restart a task already running is
//! baked into the fresh seed. Both self-heal (the task exits) and are rare.
//! Thread-level stalls (a hung thread inside the agent's own process) are out of
//! scope — `cgroup.procs` is per-process.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cgroup;
use crate::core::{Hub, Status};

/// Poll cadence. Cheap (one small file read per live session) so a tight-ish
/// interval keeps the widget responsive without meaningful cost.
const TICK: Duration = Duration::from_secs(2);
/// A candidate must persist this long before it counts — lets the short-lived
/// children a tool spawns (and zombies mid-reap) drain out, killing flicker.
const SETTLE: Duration = Duration::from_secs(2);
/// Sustained CPU-quiescence after which a candidate is deemed infrastructure
/// (a lazy MCP / idle helper) and promoted. Short enough that a lazy MCP only
/// briefly mis-flags; long enough that a working build doesn't dip under it.
const PROMOTE_QUIESCENT: Duration = Duration::from_secs(10);

/// One cgroup process we're tracking as not-yet-classified-as-infra.
struct Candidate {
    /// Identity tiebreaker (see module docs): if a pid reappears with a
    /// different start-time it's a different process — we reset.
    starttime: u64,
    /// First time we saw this exact `(pid, starttime)` — gates [`SETTLE`].
    first_seen: Instant,
    /// Last CPU sample (utime+stime jiffies) for the quiescence test.
    last_cpu: u64,
    /// Last I/O sample (`rchar+wchar` bytes) for the quiescence test. Catches
    /// the I/O-bound task that burns ~0 CPU (upload / download / device install)
    /// but keeps moving bytes. `None` if `/proc/<pid>/io` is unreadable — then
    /// the test falls back to CPU alone.
    last_io: Option<u64>,
    /// Has this candidate EVER shown activity (CPU or I/O growth) since we first
    /// saw it? A candidate that has worked at all is a genuine spawned task and
    /// must NEVER be promoted to infra — otherwise a bursty task (narration
    /// synthesizing chapters with lulls between batches) gets folded into infra
    /// during a >[`PROMOTE_QUIESCENT`] gap and never flags again on the next
    /// burst. Only a candidate quiescent *since first seen* (a lazy MCP that just
    /// sits there) self-classifies as infrastructure.
    ever_active: bool,
    /// When activity (CPU *or* I/O growth) first ceased in an unbroken run;
    /// reset to `None` on any activity. `Some(t)` past [`PROMOTE_QUIESCENT`] →
    /// promote (only while `!ever_active`).
    quiescent_since: Option<Instant>,
}

/// Per-session classifier state. Lives entirely in the watcher task — the Hub
/// only ever receives the resulting boolean.
#[derive(Default)]
struct SessionWatch {
    /// Known infrastructure, keyed `pid → starttime`. Seeded when empty (first
    /// observation, and again after a revival prunes it bare), grows by
    /// promotion, pruned as entries exit.
    infra: HashMap<u32, u64>,
    candidates: HashMap<u32, Candidate>,
    /// Has this session ever run a turn (been Busy)? A background task can only
    /// be spawned by a tool DURING a turn, so before the first one there is
    /// nothing to detect — we keep folding the still-spawning launch chain +
    /// lazy MCP servers into `infra` and never flag. Set once, never cleared.
    seen_turn: bool,
    /// Last value pushed to the Hub — avoids redundant broadcasts.
    flagged: bool,
}

/// A `/proc/<pid>/stat` sample: the two fields we need, parsed robustly.
struct Stat {
    starttime: u64,
    /// utime + stime, in clock ticks (jiffies). Monotonic per process.
    cpu: u64,
}

/// Parse `(starttime, utime+stime)` from `/proc/<pid>/stat`. Returns None if the
/// process is gone or the line is malformed. Splits AFTER the last `)` so a
/// `comm` containing spaces/parens can't shift the field offsets (see module
/// docs). Post-`comm` fields (1-based): state=1, ppid=2, …, utime=12, stime=13,
/// …, starttime=20 — i.e. the kernel's stat field N maps to N-2 here.
fn read_stat(pid: u32) -> Option<Stat> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &raw[raw.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // Need through starttime (index 19, 0-based).
    if f.len() <= 19 {
        return None;
    }
    let utime: u64 = f[11].parse().ok()?;
    let stime: u64 = f[12].parse().ok()?;
    let starttime: u64 = f[19].parse().ok()?;
    Some(Stat {
        starttime,
        cpu: utime + stime,
    })
}

/// Sum `rchar + wchar` from `/proc/<pid>/io` — total bytes the process has read
/// and written across ALL fds, sockets included (not just block I/O, so a
/// network transfer counts). Monotonic per process. Returns None when the file
/// is unreadable (process gone, or — for a differing-cred target — permission
/// denied); the agent's tool children share our uid, so in practice it reads.
/// A None just drops this tick's I/O signal, leaving the CPU test intact.
fn read_io(pid: u32) -> Option<u64> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let mut sum = 0u64;
    for line in raw.lines() {
        if let Some(v) = line
            .strip_prefix("rchar:")
            .or_else(|| line.strip_prefix("wchar:"))
        {
            sum += v.trim().parse::<u64>().ok()?;
        }
    }
    Some(sum)
}

/// Background-process watcher. Spawned once at startup; runs for the daemon's
/// life. Mirrors `run_scheduler`'s shape (a `Hub` clone + a loop).
pub async fn run_proc_watch(hub: Hub) {
    let mut watches: HashMap<String, SessionWatch> = HashMap::new();
    let mut ticker = tokio::time::interval(TICK);
    loop {
        ticker.tick().await;
        let now = Instant::now();
        let live = hub.session_list();
        // Drop watches for sessions that have gone away entirely.
        let alive: std::collections::HashSet<&str> =
            live.iter().map(|m| m.id.as_str()).collect();
        watches.retain(|id, _| alive.contains(id.as_str()));

        for meta in &live {
            // A turn in flight (Busy) or a dead agent (Exited/Crashed/…) owns the
            // status slot already — the spinner / terminal overlay shows. We only
            // care about the BETWEEN-turns window, so clear any stale flag and
            // keep the classifier state warm for the next idle period.
            if meta.status != Status::Running {
                // Busy is the turn-in-flight state: seeing it once means this
                // session has run a turn, so the classifier may start looking
                // for background tasks in the next idle window (see `seen_turn`).
                if meta.status == Status::Busy {
                    watches.entry(meta.id.clone()).or_default().seen_turn = true;
                }
                if let Some(w) = watches.get_mut(&meta.id) {
                    if w.flagged {
                        w.flagged = false;
                        hub.set_background_task(&meta.id, false);
                    }
                }
                continue;
            }
            let Some(dir) = cgroup::agent_dir(&meta.id) else {
                continue; // cgroup-v1 host / unresolved — fail-open, no widget.
            };
            let pids = cgroup::read_procs(&dir);
            if pids.is_empty() {
                continue; // cgroup gone (teardown race) — leave state as-is.
            }
            let w = watches.entry(meta.id.clone()).or_default();
            let flag = w.classify(&pids, now);
            if flag != w.flagged {
                w.flagged = flag;
                hub.set_background_task(&meta.id, flag);
            }
        }
    }
}

impl SessionWatch {
    /// Run one classification pass over the cgroup's current pids; returns
    /// whether the session has a live background task.
    fn classify(&mut self, pids: &[u32], now: Instant) -> bool {
        // Snapshot stats once per pid this tick.
        let stats: HashMap<u32, Stat> = pids
            .iter()
            .filter_map(|&p| read_stat(p).map(|s| (p, s)))
            .collect();

        // Prune infra entries that exited or whose pid was reused (start-time
        // changed) — a reused pid is a different process and must re-classify.
        self.infra
            .retain(|p, st| stats.get(p).is_some_and(|s| s.starttime == *st));

        // Pre-first-turn: everything alive is infrastructure by definition (a
        // background task can only be spawned by a tool DURING a turn). Keep
        // folding the still-spawning launch chain + lazily-started MCP servers
        // into the baseline every tick — a one-shot seed misses the MCP servers
        // that appear after it and busily initialize, which then read as a bogus
        // background task on a session that has never run. Never flag here.
        if !self.seen_turn {
            for (&p, s) in &stats {
                self.infra.insert(p, s.starttime);
            }
            self.candidates.clear();
            return false;
        }

        // Seed when infra is bare: the FIRST idle observation (agent chain +
        // startup MCP, before any turn → no background task possible yet), AND
        // again after a session REVIVAL prunes the old chain away (the restarted
        // agent has fresh pids — without re-seeding they'd misread as background
        // tasks). `status == Running` with an empty infra can only mean "the agent
        // chain is here under new pids", since a dead agent isn't Running. Seed
        // and emit nothing this tick.
        if self.infra.is_empty() {
            self.infra = stats.iter().map(|(&p, s)| (p, s.starttime)).collect();
            self.candidates.clear();
            return false;
        }

        for (&pid, s) in &stats {
            if self.infra.get(&pid) == Some(&s.starttime) {
                continue; // known infrastructure.
            }
            // Candidate. Reset if the pid was reused under a new start-time.
            let c = match self.candidates.get_mut(&pid) {
                Some(c) if c.starttime == s.starttime => c,
                _ => {
                    self.candidates.insert(
                        pid,
                        Candidate {
                            starttime: s.starttime,
                            first_seen: now,
                            last_cpu: s.cpu,
                            last_io: read_io(pid),
                            ever_active: false,
                            quiescent_since: Some(now),
                        },
                    );
                    continue;
                }
            };
            // Quiescence track: CPU *or* I/O growth resets the idle clock. I/O is
            // only compared when both this and the last sample are readable — a
            // transient None must not masquerade as a byte drop (then a spurious
            // rise) and reset the clock on a truly idle process.
            let io = read_io(pid);
            let io_grew = matches!((io, c.last_io), (Some(now_io), Some(prev)) if now_io > prev);
            if s.cpu > c.last_cpu || io_grew {
                c.quiescent_since = None;
                c.ever_active = true;
            } else if c.quiescent_since.is_none() {
                c.quiescent_since = Some(now);
            }
            c.last_cpu = s.cpu;
            if io.is_some() {
                c.last_io = io;
            }
            // Promote a sustained-quiescent candidate to infrastructure — but
            // ONLY if it has been idle since we first saw it (`!ever_active`). A
            // candidate that has ever worked is a real task and stays tracked, so
            // a bursty task in a between-batch lull keeps flagging instead of
            // being mis-folded into infra and going silent forever.
            if !c.ever_active
                && c
                    .quiescent_since
                    .is_some_and(|t| now.duration_since(t) >= PROMOTE_QUIESCENT)
            {
                self.infra.insert(pid, s.starttime);
                self.candidates.remove(&pid);
            }
        }

        // Drop candidates that have exited.
        self.candidates
            .retain(|p, c| stats.get(p).is_some_and(|s| s.starttime == c.starttime));

        // Flagged iff a settled, non-promoted candidate remains.
        self.candidates
            .values()
            .any(|c| now.duration_since(c.first_seen) >= SETTLE)
    }
}
