#!/usr/bin/env bash
# Runs the heartbeat-gap reproduction matrix in a disposable NixOS VM
# (qemu user networking, overlay disk). Nothing on the host changes; the
# Columbus nixos-test-vm isolation check proves it.
#   DUR=180 tools/heartbeat-gap-repro/run.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export VM_DIR=${VM_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/hbrepro-vm.XXXXXX")}
export VM_PORT=${VM_PORT:-2241}
export VM_KEY=$VM_DIR/key
export FLAKE=${FLAKE:-$(realpath /home/draven/columbus/machines)}
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
"$VM" build
"$VM" up
trap '"$VM" down || true' EXIT
"$VM" ssh "DUR=${DUR:-180} bash /etc/hbrepro/matrix.sh"
"$VM" ssh 'cat /root/results/summary.jsonl' >"$VM_DIR/summary.jsonl"
"$VM" ssh 'tar -C /root -cz results' >"$VM_DIR/results.tar.gz"
"$VM" isolation check
echo "results: $VM_DIR/summary.jsonl"
