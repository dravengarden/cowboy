#!/usr/bin/env bash
# Generate packages.lock.json: the exact Ubuntu 26.04 package closure of the
# Machine browser runtime (Google Chrome's .deb plus the display server, window
# manager, helpers, XKB data, fonts and glibc), with every .deb's pool URL,
# version, size and SHA-256, resolved inside a throwaway rootless Docker
# ubuntu:26.04 container the way the distribution's own resolver does it.
# `build.ts` then assembles the artifact from the lock alone, without Docker.
#
# Usage: plugins/machine-browser/runtime/lock.sh
# Rerun when a Chrome security release or an Ubuntu update must be picked up;
# the script also pins the Chrome version into contract.json.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
plugin=$(dirname "$here")
: "${DOCKER_HOST:=unix:///run/user/$(id -u)/docker.sock}"
export DOCKER_HOST
image=${MACHINE_BROWSER_LOCK_IMAGE:-ubuntu:26.04}
# build.ts reads the same cache, so the Chrome .deb is downloaded once.
cache=${MACHINE_BROWSER_DEB_CACHE:-dist/plugins/machine-browser/debs}
mkdir -p "$cache"
cache=$(cd "$cache" && pwd)
chrome_deb="$cache/google-chrome-stable_current_amd64.deb"
if [ ! -s "$chrome_deb" ]; then
  curl -fsSL --retry 6 --retry-all-errors --retry-delay 4 -o "$chrome_deb.part" \
    https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
  mv "$chrome_deb.part" "$chrome_deb"
fi
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# Runtime roots beside the browser's own dependency closure. tzdata gives the
# artifact's glibc the zone the TZ variable names; dash provides /bin/sh,
# through which the X server runs xkbcomp when it compiles the keymap (Xorg's
# Popen), so the display server cannot start without it.
roots="tigervnc-standalone-server tigervnc-common matchbox-window-manager xfonts-base xkb-data x11-xkb-utils xauth x11-xserver-utils fonts-dejavu-core fonts-liberation fonts-noto-cjk libgl1 libegl1 libgles2 libglx-mesa0 libgl1-mesa-dri libc6 libgcc-s1 libstdc++6 libc-bin tzdata dash"
# Packages in the closure that are not runtime files of a browser instance:
# package management, shells and core utilities, PAM and login, init systems,
# daemons, interpreters, documentation-only metapackages and host integration
# the launcher replaces (/etc is scaffolded, fonts.conf is generated).
deny='^(base-files|base-passwd|dpkg|debconf|perl-base|perl|perl-modules-[0-9.]+|libperl[0-9.]+t?64?|passwd|adduser|login|login\.defs|init-system-helpers|bash|coreutils|coreutils-from-gnu|coreutils-from-uutils|gnu-coreutils|rust-coreutils|sed|grep|tar|gzip|findutils|util-linux|mount|libpam-modules|libpam-modules-bin|libpam-runtime|libpam-systemd|sysvinit-utils|ucf|media-types|mailcap|xdg-utils|wget|curl|ca-certificates|openssl|debianutils|hostname|sensible-utils|readline-common|netbase|install-info|dmsetup|systemd.*|libsystemd-shared|udev|libu2f-udev|python3.*|x11-common|xfonts-utils|xfonts-encodings|fontconfig-config|fonts-dejavu-mono|cpp|cpp-[0-9]+|cpp-x86-64-linux-gnu|cpp-[0-9]+-x86-64-linux-gnu|bubblewrap|dbus|dbus-bin|dbus-broker|dbus-daemon|dbus-session-bus-common|dbus-system-bus-common|dbus-user-session|dconf-service|procps|cdebconf|libdebconfclient0|libdebian-installer4|libfile-readbackwards-perl|libnewt0\.52|libslang2|libtextwrap1|lsb-base|gtk-update-icon-cache|glycin-loaders|glycin-thumbnailers|fakeroot|libfakeroot)$'

docker run --rm -v "$out:/out" -v "$chrome_deb:/tmp/chrome.deb:ro" -e "ROOTS=$roots" -e "DENY=$deny" "$image" bash -euo pipefail -c '
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends ca-certificates apt-rdepends python3 >/dev/null
  # Let apt resolve Chrome'"'"'s alternatives and virtual packages the way the
  # distribution does; the history log names what it chose.
  apt-get install -y -qq --no-install-recommends /tmp/chrome.deb $ROOTS >/dev/null
  installed=$(grep -h "^Install:" /var/log/apt/history.log | tail -1 | sed "s/^Install: //" | tr "," "\n" | sed "s/ (.*//; s/^ *//; s/:amd64//" | grep -v -E "^(google-chrome-stable)?$" | tr "\n" " ")
  # apt-rdepends prints architecture-qualified names (perl:any) for some
  # dependencies; the binary package is the unqualified name.
  apt-rdepends --follow=Depends,PreDepends --show=Depends,PreDepends $installed $ROOTS 2>/dev/null | grep -v "^ " | sed "s/:any$//; s/:amd64$//" | sort -u > /tmp/closure.txt
  python3 -I - <<PY
import hashlib, json, os, re, subprocess, datetime
roots = os.environ["ROOTS"].split()
deny = re.compile(os.environ["DENY"])
def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout
def field(stanza, name):
    for line in stanza.splitlines():
        if line.startswith(name + ":"):
            return line.split(":", 1)[1].strip()
    return None
mirror = None
for path in ["/etc/apt/sources.list.d/ubuntu.sources", "/etc/apt/sources.list"]:
    if os.path.exists(path):
        for line in open(path):
            line = line.strip()
            if line.startswith("URIs:"):
                mirror = line.split(":", 1)[1].strip().split()[0]
                break
            if line.startswith("deb ") and "http" in line:
                mirror = line.split()[1]
                break
    if mirror:
        break
assert mirror, "no apt mirror found"
mirror = mirror.rstrip("/")
release = {}
for line in open("/etc/os-release"):
    if "=" in line:
        key, value = line.rstrip("\n").split("=", 1)
        release[key] = value.strip().strip(chr(34))
packages = []
excluded = []
# apt-rdepends also prints status lines on stdout; keep only Debian package names.
closure = [name for name in open("/tmp/closure.txt").read().split() if re.fullmatch(r"[a-z0-9][a-z0-9+.-]+", name)]
wanted = [name for name in closure if not deny.match(name)]
for name in closure:
    if deny.match(name):
        excluded.append({"name": name, "reason": "not a runtime file of a browser instance"})
# One policy query and one show query for every package instead of two per
# package: the candidate version, then the stanza carrying that version.
candidates = {}
current = None
for line in run("apt-cache", "policy", *wanted).splitlines():
    if line and not line.startswith(" ") and line.endswith(":"):
        current = line[:-1]
    elif line.strip().startswith("Candidate:") and current:
        candidates[current] = line.split(":", 1)[1].strip()
stanzas = {}
for block in run("apt-cache", "show", *wanted).split("\n\n"):
    package, version = field(block, "Package"), field(block, "Version")
    if package and version:
        stanzas[(package, version)] = block
for name in wanted:
    candidate = candidates.get(name)
    if not candidate or candidate == "(none)":
        excluded.append({"name": name, "reason": "virtual or uninstallable"})
        continue
    stanza = stanzas.get((name, candidate))
    assert stanza, f"no stanza for {name} {candidate}"
    filename = field(stanza, "Filename")
    packages.append({
        "name": name,
        "version": candidate,
        "filename": filename,
        "size": int(field(stanza, "Size")),
        "sha256": field(stanza, "SHA256"),
        "urls": [
            f"{mirror}/{filename}",
            "https://launchpad.net/ubuntu/+archive/primary/+files/" + os.path.basename(filename),
        ],
    })
packages.sort(key=lambda p: p["name"])
excluded.sort(key=lambda p: p["name"])
chrome_version = run("dpkg-deb", "-f", "/tmp/chrome.deb", "Version").strip()
browser_version = chrome_version.split("-")[0]
digest = hashlib.sha256(open("/tmp/chrome.deb", "rb").read()).hexdigest()
lock = {
    "schema_version": 1,
    "generated_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "distribution": {
        "id": release.get("ID", "ubuntu"),
        "version_id": release.get("VERSION_ID", ""),
        "codename": release.get("VERSION_CODENAME", ""),
        "architecture": "amd64",
        "mirror": mirror,
    },
    "browser": {
        "package": "google-chrome-stable",
        "version": chrome_version,
        "browser_version": browser_version,
        "size": os.path.getsize("/tmp/chrome.deb"),
        "sha256": digest,
        "urls": [
            f"https://dl.google.com/linux/chrome/deb/pool/main/g/google-chrome-stable/google-chrome-stable_{chrome_version}_amd64.deb",
            "https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb",
        ],
    },
    "requested": roots,
    "excluded": excluded,
    "packages": packages,
}
with open("/out/packages.lock.json", "w") as handle:
    json.dump(lock, handle, indent=2)
    handle.write("\n")
print(f"{len(packages)} packages, {len(excluded)} excluded, chrome {chrome_version}")
PY
  chmod -R a+rwX /out
'
cp "$out/packages.lock.json" "$here/packages.lock.json"
# File the Chrome .deb under its digest so build.ts finds it in the cache.
chrome_sha=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["browser"]["sha256"])' "$here/packages.lock.json")
if [ "$(sha256sum "$chrome_deb" | cut -d" " -f1)" = "$chrome_sha" ]; then
  cp "$chrome_deb" "$cache/$chrome_sha.deb"
fi
browser_version=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["browser"]["browser_version"])' "$here/packages.lock.json")
python3 -I - "$plugin/contract.json" "$browser_version" <<'PY'
import json, sys
path, version = sys.argv[1], sys.argv[2]
with open(path) as handle:
    contract = json.load(handle)
contract["runtime"]["browser_version"] = version
with open(path, "w") as handle:
    json.dump(contract, handle, indent=2)
    handle.write("\n")
PY
echo "pinned Chrome $browser_version into $plugin/contract.json and $here/packages.lock.json"
