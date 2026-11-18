#!/bin/sh
# Cowboy launcher for the unmodified official Antigravity CLI (`agy`).
#
# It prepares the account's private HOME and then hands control to the vendor
# binary; it never reads, writes or forwards credentials. Login, refresh and
# logout are performed by `agy` itself.
# - Auto-update is disabled: `agy` otherwise rewrites its own executable,
#   bypassing the pinned, signed release (only the value `true` works).
# - D-Bus points at a socket that cannot exist, so `agy` keeps its 0600 token
#   file under this HOME instead of the user's login keyring.
# - ACP runtime sessions get Cowboy's PreToolUse hooks, which deny unattended,
#   fan-out and background-subagent tools (`schedule`, `swarm`,
#   `run_workflow`, `invoke_subagent`, `define_subagent`, `manage_subagents`).
#
# The vendor binary is the sibling `provider_cli` component of the same signed
# Provider generation; the Machine installs every component of a generation
# under `runtime/<kind>-<slot>/content/`.
set -eu
case "$0" in */*) cowboy_dir=${0%/*} ;; *) cowboy_dir=. ;; esac
cowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)
hooks="$cowboy_root/share/hooks.json"
if [ "${1-}" = "--cowboy-probe" ]; then
  [ -f "$hooks" ] && exit 0
  exit 1
fi
# In an account-sandbox runtime archive (`cowboy-runtime.json` at its root)
# the vendor binary is `bin/agy`; this launcher serves only login, logout and
# probe units there (runtime units start the `agy-proxy` Node proxy), and the
# daemon binds the policy files read-only over the account home.
sandboxed=false
if [ -f "$cowboy_root/cowboy-runtime.json" ]; then
  sandboxed=true
  agy="$cowboy_root/bin/agy"
else
  agy="$cowboy_root/../../provider_cli-antigravity/content/antigravity"
fi
[ -x "$agy" ] || { echo "cowboy: Antigravity CLI component is missing" >&2; exit 127; }
: "${HOME:?HOME is required}"
umask 077
unset XDG_RUNTIME_DIR
DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/cowboy-antigravity-no-bus
AGY_CLI_DISABLE_AUTO_UPDATE=true
export DBUS_SESSION_BUS_ADDRESS AGY_CLI_DISABLE_AUTO_UPDATE
case "${1-}" in
  --cowboy-login)
    exec "$agy" acp --login --disable-telemetry
    ;;
  --cowboy-logout)
    # `agy` has no logout command; its ACP server removes its own token.
    token="$HOME/.gemini/antigravity-cli/antigravity-oauth-token"
    printf '%s\n' \
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}' \
      '{"jsonrpc":"2.0","id":2,"method":"logout","params":{}}' |
      "$agy" acp --disable-telemetry >/dev/null
    [ ! -e "$token" ]
    exit
    ;;
esac
# Only an ACP runtime session owns its account home. Version and other probes
# may run with the Machine user's real HOME and must never write to it.
if [ "${1-}" != "acp" ] || [ "$sandboxed" = true ]; then
  exec "$agy" "$@"
fi
config="$HOME/.gemini/config"
mkdir -p "$config"
cp "$hooks" "$config/.hooks.json.$$"
mv -f "$config/.hooks.json.$$" "$config/hooks.json"
exec "$agy" "$@"
