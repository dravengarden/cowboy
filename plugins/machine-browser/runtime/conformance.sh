#!/usr/bin/env bash
# Run the built runtime tree and its launcher in a pristine ubuntu:26.04
# rootless Docker container as an unprivileged user, without AppArmor, and
# verify the launcher contract from outside the instance: no listeners, the
# self-only uid map, empty capability sets and no_new_privs on every sealed
# process, the artifact as /usr and /opt, the read-only socket export and
# masks, an RFB handshake with clipboard forwarding, native touch
# (touch_probe.py, with and without -AcceptTouchEvents, and revoking pointer
# input while contacts are down), navigate, resize,
# status, snapshot, a graceful stop with a snapshot, and a restore from that
# snapshot. Prints the JSON summary written by conformance_probe.py.
#
# Usage: plugins/machine-browser/runtime/conformance.sh [TREE_DIR|ARCHIVE.tar.gz] [OUT_DIR]
# Default input: dist/plugins/machine-browser/runtime-linux-x86_64.tar.gz
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${DOCKER_HOST:=unix:///run/user/$(id -u)/docker.sock}"
export DOCKER_HOST
image=${MACHINE_BROWSER_CONFORMANCE_IMAGE:-ubuntu:26.04}
input=${1:-dist/plugins/machine-browser/runtime-linux-x86_64.tar.gz}
out=${2:-dist/plugins/machine-browser/conformance}
work=$(mktemp -d /tmp/cowboy-browser-conformance.XXXXXX)
trap 'rm -rf "$work"' EXIT
if [ -d "$input" ]; then
  tree=$(cd "$input" && pwd)
else
  mkdir "$work/tree"
  tar -xzf "$input" -C "$work/tree"
  tree="$work/tree"
fi
test -x "$tree/usr/libexec/cowboy-browser-launcher"
mkdir -p "$out"
out=$(cd "$out" && pwd)
# One run per output directory: the probe appends to its logs.
rm -rf "$out"/launcher-*.log "$out"/instance-* "$out"/summary.json "$out"/xclip.log "$out"/touch-off* "$out"/touch-revoke*

# systempaths=unconfined lets an unprivileged user mount a fresh proc for the
# instance's pid namespace (Docker otherwise masks /proc paths, which the
# kernel then refuses to over-mount). SYS_ADMIN is only used by the container's
# root to hand the test user a writable cgroup, as systemd's Delegate=yes does
# for the Machine user; SYS_PTRACE lets the root probe read the test user's
# /proc entries (namespaces, /proc/<pid>/root) the way the Machine reads its
# own user's. Neither reaches the launcher, which runs as the test user.
docker run --rm \
  -v "$tree:/runtime:ro" \
  -v "$here/conformance_probe.py:/conformance/probe.py:ro" \
  -v "$here/touch_probe.py:/conformance/touch_probe.py:ro" \
  -v "$out:/out" \
  --security-opt seccomp=unconfined \
  --security-opt apparmor=unconfined \
  --security-opt systempaths=unconfined \
  --cap-add SYS_ADMIN \
  --cap-add SYS_PTRACE \
  --shm-size 512m \
  --memory 3g \
  "$image" bash -euo pipefail -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends python3 iproute2 procps util-linux xclip xinput >/dev/null
    useradd -u 1500 -m tester
    mount -o remount,bind,rw /sys/fs/cgroup
    mkdir /sys/fs/cgroup/browser-unit
    chown -R tester /sys/fs/cgroup/browser-unit
    install -d -m 0700 -o tester -g tester /run/cowboy-browser /run/user/1500 /var/lib/cowboy-state /var/lib/cowboy-state/staging
    # xclip for the clipboard check, run inside the instance where /usr is the
    # artifact: carry the X toolkit libraries the artifact may not ship.
    mkdir -p /conformance/lib
    cp /usr/bin/xclip /conformance/xclip
    for lib in libXmu.so.6 libXt.so.6 libSM.so.6 libICE.so.6 libXext.so.6 libX11.so.6 libxcb.so.1 libXau.so.6 libXdmcp.so.6 libbsd.so.0 libmd.so.0 libuuid.so.1; do
      if [ -e "/usr/lib/x86_64-linux-gnu/$lib" ]; then cp -L "/usr/lib/x86_64-linux-gnu/$lib" /conformance/lib/; fi
    done
    python3 -I /conformance/probe.py \
      --launcher /runtime/usr/libexec/cowboy-browser-launcher \
      --runtime /runtime \
      --control /run/cowboy-browser \
      --staging /var/lib/cowboy-state/staging \
      --cgroup /sys/fs/cgroup/browser-unit \
      --display :7 --geometry 1280x800 --user tester \
      --xclip-dir /conformance \
      --mask /var/lib/cowboy-state \
      --out /out || true
    # The same display server without -AcceptTouchEvents must look stock:
    # no acknowledgement, and message 193 is an unknown message. Its own
    # mount namespace sees the artifact as /usr, as an instance does.
    mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix
    unshare -m sh -c "mount --bind /runtime/usr /usr && exec /usr/bin/Xtigervnc :9 -nolisten tcp -nolisten local -rfbport -1 -rfbunixpath /tmp/touch-off.sock -SecurityTypes None -geometry 800x600 -depth 24" >/out/touch-off-xvnc.log 2>&1 &
    off=$!
    for _ in $(seq 50); do [ -S /tmp/touch-off.sock ] && break; sleep 0.2; done
    python3 -I /conformance/touch_probe.py /tmp/touch-off.sock --off >/out/touch-off.json || true
    kill $off || true
    # With the flag, turning AcceptPointerEvents off through the VNC
    # extension ends active contacts and frees their slots (XI2 events seen
    # by xinput from outside the namespace).
    unshare -m sh -c "mount --bind /runtime/usr /usr && exec /usr/bin/Xtigervnc :10 -nolisten tcp -nolisten local -rfbport -1 -rfbunixpath /tmp/touch-revoke.sock -SecurityTypes None -geometry 800x600 -depth 24 -AcceptTouchEvents=1" >/out/touch-revoke-xvnc.log 2>&1 &
    revoke=$!
    for _ in $(seq 50); do [ -S /tmp/touch-revoke.sock ] && break; sleep 0.2; done
    python3 -I /conformance/touch_probe.py /tmp/touch-revoke.sock --revoke :10 >/out/touch-revoke.json || true
    kill $revoke || true
    chmod -R a+rwX /out
  '
cat "$out/summary.json"
cat "$out/touch-off.json" "$out/touch-revoke.json"
python3 -I -c '
import json, sys
ok = all(json.load(open(path))["ok"] for path in sys.argv[1:])
sys.exit(0 if ok else 1)
' "$out/summary.json" "$out/touch-off.json" "$out/touch-revoke.json"
