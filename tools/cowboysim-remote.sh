#!/usr/bin/env bash
# Direct SSH transport to an explicitly selected Cowboy Git worktree on a Mac.
# No personal plugin, fixed home directory or source-copy deployment.
set -euo pipefail
remote_host="${COWBOY_SIM_MAC_HOST:-macbook-air}"
remote_worktree="${COWBOY_SIM_REMOTE_WORKTREE:?set an absolute Cowboy Git worktree path on the Mac}"
case "$remote_host" in ""|-*|*[!a-zA-Z0-9_.@-]*) echo "invalid SSH host alias" >&2; exit 2;; esac
case "$remote_worktree" in /*) ;; *) echo "remote worktree must be absolute" >&2; exit 2;; esac
quote() {
  # Avoid replacement-string quote rules that differ between Bash 3 and 5.
  local value="$1"
  printf "'"
  while [[ "$value" == *"'"* ]]; do
    printf '%s%s' "${value%%\'*}" "'\\''"
    value="${value#*\'}"
  done
  printf "%s'" "$value"
}
remote_command="cd $(quote "$remote_worktree") && test -f tools/cowboysim.sh && test \"\$(git rev-parse --show-toplevel)\" = \"\$(pwd -P)\" && exec env"
for name in COWBOY_SIM_UDID COWBOY_SIM_DEVPORT; do
  if [ -n "${!name:-}" ]; then remote_command+=" $(quote "$name=${!name}")"; fi
done
remote_command+=" bash tools/cowboysim.sh"
for arg in "$@"; do remote_command+=" $(quote "$arg")"; done
# The remote login shell may be fish. Pass a single safely-quoted POSIX body.
exec ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UpdateHostKeys=no \
  -o ConnectTimeout=8 -- "$remote_host" "/bin/sh -ec $(quote "$remote_command")"
