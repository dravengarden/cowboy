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
# assuming a target directory or choosing a stale executable from a glob.
fixture_binary="$(
  cargo test -p cowboy --lib --all-features --locked --no-run \
    --message-format=json-render-diagnostics |
    jq -r 'select(.reason == "compiler-artifact" and .target.name == "cowboy"
      and .profile.test and (.target.kind | index("lib"))) | .executable // empty'
)"
if [[ ! -f "$fixture_binary" || ! -x "$fixture_binary" ]]; then
  echo "Cargo did not produce exactly one Cowboy library test executable" >&2
  exit 1
fi
fixture_listing="$("$fixture_binary" --ignored --list)"
fixture_tests=()
while IFS= read -r fixture_test; do
  case "$fixture_test" in
    *::postgres_*": test") fixture_tests+=("${fixture_test%: test}");;
  esac
done <<< "$fixture_listing"
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
  -o "-F -c port=5432 -c listen_addresses='' -c unix_socket_directories='$fixture_root/socket' -c unix_socket_permissions=0700" \
  -w -t 30 start || {
    sed -n '1,160p' "$fixture_root/postgres.log" >&2
    exit 1
  }

echo "Running ${#fixture_tests[@]} PostgreSQL tests, each in an isolated empty database"
fixture_index=0
for fixture_test in "${fixture_tests[@]}"; do
  fixture_index=$((fixture_index + 1))
  fixture_database="cowboy_test_$fixture_index"
  createdb --host="$fixture_root/socket" --port=5432 --username=cowboy_test \
    --maintenance-db=postgres --template=template0 "$fixture_database"
  TMPDIR="$fixture_root/tmp" \
    COWBOY_TEST_POSTGRES_URL="postgresql://cowboy_test@localhost/$fixture_database?host=$fixture_root/socket&sslmode=disable" \
    "$fixture_binary" "$fixture_test" --exact --ignored --nocapture
done
