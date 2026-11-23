#!/usr/bin/env bash
# Run from the repository root: nix develop -c just test-postgres
set -euo pipefail

if (($# != 0)); then
  echo "usage: just test-postgres (no external database arguments)" >&2
  exit 2
fi
if ((EUID == 0)); then
  echo "PostgreSQL fixtures must run as an unprivileged user" >&2
  exit 2
fi
for fixture_tool in cargo jq initdb pg_ctl createdb; do
  command -v "$fixture_tool" >/dev/null || {
    echo "missing $fixture_tool; run nix develop -c just test-postgres" >&2
    exit 2
  }
done

# libpq/SQLx must not discover a service, password file, socket, or database from
# the invoking shell. No ambient database address is ever used, even on failure.
for fixture_variable in "${!PG@}" COWBOY_TEST_POSTGRES_URL COWBOY_PROVIDER_PACKAGE_PATH; do
  unset "$fixture_variable"
done
export PGPASSFILE=/dev/null PGSERVICEFILE=/dev/null

# Compile once, before starting the server. Use Cargo's artifact path instead of
# assuming a target directory or choosing a stale executable from a glob. The
# Cowboy library owns most storage contracts; the Controller crate owns the
# store writer, whose transcript-anchor contract also runs on both databases.
# `just test-postgres` passes the `just test` selection: the same package and
# feature graph reuses those test executables instead of recompiling the
# workspace under a different dependency feature unification.
read -r -a fixture_cargo_args <<< "${COWBOY_RUST_TEST_ARGS:---all-features --locked -p cowboy -p cowboy-store -p cowboy-server-auth -p cowboy-server-runtime -p cowboy-server --lib}"
if [[ -n "${COWBOY_TEST_GRAPH_ARTIFACTS:-}" && -r "$COWBOY_TEST_GRAPH_ARTIFACTS" ]]; then
  fixture_artifacts="$(cat "$COWBOY_TEST_GRAPH_ARTIFACTS")"
else
  fixture_artifacts="$(cargo test "${fixture_cargo_args[@]}" --no-run \
    --message-format=json-render-diagnostics)"
fi
fixture_binary_for() {
  local target="$1"
  jq -r --arg target "$target" 'select(.reason == "compiler-artifact" and .target.name == $target
    and .profile.test and (.target.kind | index("lib"))) | .executable // empty' <<< "$fixture_artifacts"
}
fixture_binaries=()
fixture_tests=()
for fixture_crate in cowboy:cowboy cowboy-store:cowboy_store cowboy-server-auth:cowboy_server_auth cowboy-server-runtime:cowboy_server_runtime cowboy-server:cowboy_server; do
  fixture_binary="$(fixture_binary_for "${fixture_crate#*:}")"
  if [[ ! -f "$fixture_binary" || ! -x "$fixture_binary" ]]; then
    echo "Cargo did not produce exactly one ${fixture_crate%%:*} library test executable" >&2
    exit 1
  fi
  fixture_listing="$("$fixture_binary" --ignored --list)"
  while IFS= read -r fixture_test; do
    case "$fixture_test" in
      *::postgres_*": test")
        fixture_binaries+=("$fixture_binary")
        fixture_tests+=("${fixture_test%: test}");;
    esac
  done <<< "$fixture_listing"
done
if ((${#fixture_tests[@]} == 0)); then
  echo "no ignored postgres_ tests found; refusing an empty database gate" >&2
  exit 1
fi

umask 077
fixture_root="$(mktemp -d /tmp/cowboy-postgres-test.XXXXXX)"
cleanup() {
  local fixture_status=$?
  trap - EXIT
  # Only delete the exact directory allocated above, and only after its server
  # has stopped. A failed stop retains the fixture for diagnosis, never wipes it.
  case "$fixture_root" in
    /tmp/cowboy-postgres-test.??????) ;;
    *) echo "refusing unexpected PostgreSQL fixture path" >&2; exit 1;;
  esac
  if [[ ! -d "$fixture_root" || -L "$fixture_root" || ! -O "$fixture_root" ]]; then
    echo "PostgreSQL fixture ownership changed; refusing cleanup" >&2
    exit 1
  fi
  if [[ -f "$fixture_root/data/postmaster.pid" ]]; then
    if ! pg_ctl -D "$fixture_root/data" -m immediate -w -t 15 stop; then
      echo "could not stop fixture; retained $fixture_root" >&2
      exit 1
    fi
  fi
  # Signed Plugin generations are deliberately read-only. Unlock only owned
  # fixture directories; never follow their presentation symlinks.
  find -P "$fixture_root" -type d -exec chmod u+w -- {} +
  rm -r -- "$fixture_root"
  exit "$fixture_status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir "$fixture_root/socket" "$fixture_root/tmp"
export PGPASSFILE="$fixture_root/pgpass" PGSERVICEFILE="$fixture_root/pg-service"
touch "$PGPASSFILE" "$PGSERVICEFILE"
initdb -D "$fixture_root/data" -U cowboy_test --auth-local=trust \
  --auth-host=reject --no-locale --encoding=UTF8 --no-sync \
  >"$fixture_root/initdb.log" 2>&1 || {
    sed -n '1,160p' "$fixture_root/initdb.log" >&2
    exit 1
  }
pg_ctl -D "$fixture_root/data" -l "$fixture_root/postgres.log" \
  -o "-F -c port=5432 -c listen_addresses='' -c unix_socket_directories='$fixture_root/socket' -c unix_socket_permissions=0700 -c max_connections=400" \
  -w -t 30 start || {
    sed -n '1,160p' "$fixture_root/postgres.log" >&2
    exit 1
  }

# Each test owns an empty database and a process, so tests run concurrently.
# A test's output is printed whole when it finishes, never interleaved.
fixture_jobs="${COWBOY_POSTGRES_TEST_JOBS:-8}"
if [[ ! "$fixture_jobs" =~ ^[1-9][0-9]*$ ]]; then
  echo "COWBOY_POSTGRES_TEST_JOBS must be a positive integer" >&2
  exit 2
fi
mkdir "$fixture_root/logs"
run_fixture() {
  local fixture_index="$1" fixture_binary="$2" fixture_test="$3"
  local fixture_database="cowboy_test_$fixture_index"
  createdb --host="$fixture_root/socket" --port=5432 --username=cowboy_test \
    --maintenance-db=postgres --template=template0 "$fixture_database"
  TMPDIR="$fixture_root/tmp" \
    COWBOY_TEST_POSTGRES_URL="postgresql://cowboy_test@localhost/$fixture_database?host=$fixture_root/socket&sslmode=disable" \
    "$fixture_binary" "$fixture_test" --exact --ignored --nocapture
}
declare -A fixture_running=()
fixture_failed=()
reap_fixture() {
  local fixture_pid fixture_status=0
  wait -n -p fixture_pid || fixture_status=$?
  local fixture_index="${fixture_running[$fixture_pid]}"
  unset "fixture_running[$fixture_pid]"
  cat "$fixture_root/logs/$fixture_index.log"
  if ((fixture_status != 0)); then
    fixture_failed+=("${fixture_tests[fixture_index - 1]}")
  fi
}

echo "Running ${#fixture_tests[@]} PostgreSQL tests, each in an isolated empty database, $fixture_jobs at a time"
for fixture_position in "${!fixture_tests[@]}"; do
  while ((${#fixture_running[@]} >= fixture_jobs)); do
    reap_fixture
  done
  fixture_index=$((fixture_position + 1))
  run_fixture "$fixture_index" "${fixture_binaries[$fixture_position]}" "${fixture_tests[$fixture_position]}" \
    >"$fixture_root/logs/$fixture_index.log" 2>&1 &
  fixture_running[$!]="$fixture_index"
done
while ((${#fixture_running[@]} > 0)); do
  reap_fixture
done
if ((${#fixture_failed[@]} != 0)); then
  printf 'failed PostgreSQL test: %s\n' "${fixture_failed[@]}" >&2
  exit 1
fi
