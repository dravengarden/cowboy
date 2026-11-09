#!/usr/bin/env bash
# Generate display-server.lock.json: everything `display-server.ts` needs to
# rebuild the runtime's patched Xtigervnc without Docker, from bytes pinned by
# SHA-256, in the exact build environment Ubuntu used for the binary the
# runtime lock ships:
#
# - the Launchpad .buildinfo of that amd64 build (its own SHA-256), whose
#   Installed-Build-Depends is the complete build root and whose Build-Date
#   names the snapshot.ubuntu.com instant to fetch it from;
# - every build-root package at exactly that version, by snapshot pool
#   path, size and SHA-256 (resolute, -updates, -security and -proposed,
#   since Launchpad builds the development series against -proposed);
# - the source package (.dsc, orig and debian tarballs).
#
# Resolution is metadata only (apt-cache inside a throwaway rootless Docker
# ubuntu:26.04 container); nothing of the build root is installed. The
# expected output digest is recorded by the first rebuild
# (`display-server.ts --record`); every later build must reproduce it, and
# `display-server.ts --verify-stock` proves the build root by rebuilding the
# unpatched source to Ubuntu's own binary byte for byte.
#
# Usage: plugins/machine-browser/runtime/display-server-lock.sh
# Rerun after lock.sh moved tigervnc, then review native-touch.patch (its
# version guard refuses to build until then) and rerun with --record.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${DOCKER_HOST:=unix:///run/user/$(id -u)/docker.sock}"
export DOCKER_HOST
image=${MACHINE_BROWSER_LOCK_IMAGE:-ubuntu:26.04}
# The source is published after its build starts, so it comes from the
# snapshot of the runtime lock instead.
read -r version source_snapshot < <(python3 -I -c '
import json, sys
lock = json.load(open(sys.argv[1]))
stamp = lock["generated_at"].replace("-", "").replace(":", "")
print(next(p["version"] for p in lock["packages"] if p["name"] == "tigervnc-standalone-server"), stamp)
' "$here/packages.lock.json")
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# The amd64 build of that source version and its .buildinfo.
api="https://api.launchpad.net/devel/ubuntu/+archive/primary"
encoded=$(python3 -I -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$version")
buildinfo_url=$(curl -fsSL "$api?ws.op=getPublishedSources&source_name=tigervnc&version=$encoded&exact_match=true" |
  python3 -I -c '
import json, sys, urllib.request
for source in json.load(sys.stdin)["entries"]:
    builds = json.load(urllib.request.urlopen(source["self_link"] + "?ws.op=getBuilds"))
    for build in builds["entries"]:
        if build["arch_tag"] == "amd64" and build.get("buildinfo_url"):
            print(build["buildinfo_url"])
            sys.exit(0)
sys.exit("no amd64 buildinfo")
')
curl -fsSL --retry 4 -o "$out/build.buildinfo" "$buildinfo_url"

# The image has no CA bundle and the snapshot service is HTTPS-only: borrow
# the host's bundle read-only. apt still verifies the archive signatures.
ca_bundle=$(readlink -f "${SSL_CERT_FILE:-/etc/ssl/certs/ca-certificates.crt}")
docker run --rm -v "$out:/out" -v "$ca_bundle:/etc/ssl/certs/ca-certificates.crt:ro" \
  -e "VERSION=$version" -e "BUILDINFO_URL=$buildinfo_url" -e "SOURCE_SNAPSHOT=$source_snapshot" \
  "$image" bash -euo pipefail -c '
  export DEBIAN_FRONTEND=noninteractive
  echo "Acquire::https::CaInfo \"/etc/ssl/certs/ca-certificates.crt\";" > /etc/apt/apt.conf.d/99ca-bundle
  date=$(sed -n "s/^Build-Date: //p" /out/build.buildinfo)
  snapshot=$(date -u -d "$date" +%Y%m%dT%H%M%SZ)
  codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
  cat > /etc/apt/sources.list.d/ubuntu.sources <<EOF
Types: deb
URIs: https://snapshot.ubuntu.com/ubuntu/$snapshot
Suites: $codename $codename-updates $codename-security $codename-proposed
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb-src
URIs: https://snapshot.ubuntu.com/ubuntu/$SOURCE_SNAPSHOT
Suites: $codename $codename-updates $codename-security
Components: main restricted universe multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
  apt-get update -qq
  apt-get source --print-uris -qq "tigervnc=$VERSION" > /tmp/source.txt
  # Installed-Build-Depends: " name (= version)," lines.
  sed -n "/^Installed-Build-Depends:/,/^[A-Z]/p" /out/build.buildinfo |
    sed -n "s/^ \([^ ]*\) (= \([^)]*\)),\?$/\1\t\2/p" > /tmp/root.txt
  : > /tmp/stanzas.txt
  while IFS="$(printf "\t")" read -r name version; do
    apt-cache show "$name=$version" | awk -v RS= "NR==1" >> /tmp/stanzas.txt
    echo >> /tmp/stanzas.txt
  done < /tmp/root.txt
  apt-get install -y -qq --no-install-recommends python3-minimal >/dev/null
  SNAPSHOT=$snapshot python3 -I - <<"PY"
import hashlib, json, os, re, urllib.parse
snapshot = os.environ["SNAPSHOT"]
source_snapshot = os.environ["SOURCE_SNAPSHOT"]
def field(stanza, name):
    for line in stanza.splitlines():
        if line.startswith(name + ":"):
            return line.split(":", 1)[1].strip()
    return None
wanted = [line.split("\t") for line in open("/tmp/root.txt").read().splitlines()]
stanzas = [block for block in open("/tmp/stanzas.txt").read().split("\n\n") if block.strip()]
found = {(field(block, "Package"), field(block, "Version")): block for block in stanzas}
packages = []
for name, version in wanted:
    stanza = found.get((name, version))
    assert stanza, f"{name} {version} is not in snapshot {snapshot}"
    packages.append({
        "name": name,
        "version": version,
        "architecture": field(stanza, "Architecture"),
        "filename": field(stanza, "Filename"),
        "size": int(field(stanza, "Size")),
        "sha256": field(stanza, "SHA256"),
    })
packages.sort(key=lambda p: p["name"])
files = []
for line in open("/tmp/source.txt").read().splitlines():
    match = re.match(r"^\x27([^\x27]+)\x27 (\S+) (\d+) SHA256:([0-9a-f]{64})$", line)
    assert match, line
    url, name, size, digest = match.groups()
    path = urllib.parse.unquote(url.split(f"/ubuntu/{source_snapshot}/", 1)[1])
    files.append({"name": name, "filename": path, "size": int(size), "sha256": digest})
files.sort(key=lambda f: f["name"])
buildinfo = open("/out/build.buildinfo", "rb").read()
lock = {
    "schema_version": 1,
    "snapshot": snapshot,
    "archive": f"https://snapshot.ubuntu.com/ubuntu/{snapshot}",
    "source": {
        "package": "tigervnc",
        "version": os.environ["VERSION"],
        "archive": f"https://snapshot.ubuntu.com/ubuntu/{source_snapshot}",
        "files": files,
    },
    "buildinfo": {
        "url": os.environ["BUILDINFO_URL"],
        "size": len(buildinfo),
        "sha256": hashlib.sha256(buildinfo).hexdigest(),
    },
    "patch": None,
    "build_root": packages,
    "output": None,
}
with open("/out/display-server.lock.json", "w") as handle:
    json.dump(lock, handle, indent=2)
    handle.write("\n")
print(f"{len(packages)} build-root packages from snapshot {snapshot}")
PY
  chmod -R a+rwX /out
'
cp "$out/display-server.lock.json" "$here/display-server.lock.json"
echo "wrote $here/display-server.lock.json; record its output with display-server.ts --record"
