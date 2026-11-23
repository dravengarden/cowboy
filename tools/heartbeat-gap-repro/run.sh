#!/usr/bin/env bash
# Runs the heartbeat-gap reproduction matrix in a disposable NixOS VM
# (qemu user networking, overlay disk). Nothing on the host changes; the
# Columbus nixos-test-vm isolation check, run after teardown on every path,
# proves it.
#   DUR=180 tools/heartbeat-gap-repro/run.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
if [ -z "${VM_DIR:-}" ]; then
    VM_DIR=$(mktemp -d "${TMPDIR:-/tmp}/hbrepro-vm.XXXXXX")
fi
[ -n "$VM_DIR" ] && [ "$VM_DIR" != / ] || { echo "invalid VM_DIR" >&2; exit 1; }
mkdir -p "$VM_DIR"
[ -z "$(ls -A "$VM_DIR")" ] || { echo "VM_DIR $VM_DIR must be fresh and empty" >&2; exit 1; }
chmod 700 "$VM_DIR"
if [ -z "${FLAKE:-}" ]; then
    FLAKE=$(realpath /home/draven/columbus/machines)
fi
[ -d "$FLAKE" ] || { echo "invalid FLAKE $FLAKE" >&2; exit 1; }
export VM_DIR FLAKE
export VM_PORT=${VM_PORT:-2241}
export VM_KEY=$VM_DIR/key
VM=${VM:-/home/draven/columbus/machines/hawk/nixos/.agents/skills/nixos-test-vm/helpers/vm.sh}
# A throwaway key for this guest only; its public half is the only part that
# reaches the Nix store.
ssh-keygen -q -t ed25519 -N '' -C hbrepro -f "$VM_KEY"
cat >"$VM_DIR/vm-config.nix" <<NIX
{ ... }: {
  imports = [ $here/vm.nix ];
  users.users.root.openssh.authorizedKeys.keys = [ "$(cat "$VM_KEY.pub")" ];
  virtualisation.forwardPorts = [ { from = "host"; host.port = $VM_PORT; guest.port = 22; } ];
}
NIX
"$VM" isolation save
finish() {
    local status=$?
    "$VM" down || status=1
    # Give qemu time to exit before comparing host state.
    sleep 5
    "$VM" isolation check || status=1
    exit "$status"
}
trap finish EXIT
"$VM" build
"$VM" up
"$VM" ssh "DUR=${DUR:-180} bash /etc/hbrepro/matrix.sh"
"$VM" ssh 'cat /root/results/summary.jsonl' >"$VM_DIR/summary.jsonl"
"$VM" ssh 'tar -C /root -cz results' >"$VM_DIR/results.tar.gz"
echo "results: $VM_DIR/summary.jsonl"
