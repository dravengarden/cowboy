#!/usr/bin/env bash
# Losslessly migrate an OVH named Antigravity account from cowboy-account-sandbox
# (ext4 loop image `home.img` + `account_sandbox: true`) to direct worker-unit
# execution (`providers/antigravity/native/home` + `account_sandbox: false`).
#
# Dry-run by default. Pass `--apply` (as root during the OVH maintenance window
# with zero live Antigravity / `cga-*` units) to copy `.gemini/antigravity-cli/`,
# SHA-256 verify every file, and atomically update `profile.json`.
# `home.img` is mounted strictly read-only (`ro,loop,noload`) and left untouched
# for instant rollback.
set -euo pipefail

APPLY=0
FORCE_OVERWRITE=0
SERVICE_ID=""
ACCOUNT_ID=""
DAEMON_KEY=""
GENERATION=""
STATE_ROOT="/home/ubuntu/.local/state/cowboy-machine"
SANDBOX_ROOT="/var/lib/cowboy-account-sandbox"
OWNER="ubuntu:ubuntu"

usage() {
  cat <<'EOF'
Usage: ovh-antigravity-migrate-home.sh --service-id <id> --account-id <uuid> [options]

Options:
  --service-id <id>      Cowboy Service ID (e.g. svc-...)
  --account-id <uuid>    Named Provider account UUID
  --daemon-key <hex16>   Expected account-sandbox daemon key (derived & cross-checked if given)
  --generation <N>       Native profile generation number (default: latest numeric generation)
  --state-root <dir>     Machine state root (default: /home/ubuntu/.local/state/cowboy-machine)
  --sandbox-root <dir>   account-sandbox data root (default: /var/lib/cowboy-account-sandbox)
  --owner <user:group>   Ownership for native/home and profile.json (default: ubuntu:ubuntu)
  --apply                Perform the migration (default is dry-run with zero mutations)
  --force-overwrite      Move any existing native/home aside and re-copy from home.img
  -h, --help             Show this help
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    --force-overwrite) FORCE_OVERWRITE=1; shift ;;
    --service-id) [ "$#" -ge 2 ] || die "--service-id requires a value"; SERVICE_ID="$2"; shift 2 ;;
    --account-id) [ "$#" -ge 2 ] || die "--account-id requires a value"; ACCOUNT_ID="$2"; shift 2 ;;
    --daemon-key) [ "$#" -ge 2 ] || die "--daemon-key requires a value"; DAEMON_KEY="$2"; shift 2 ;;
    --generation) [ "$#" -ge 2 ] || die "--generation requires a value"; GENERATION="$2"; shift 2 ;;
    --state-root) [ "$#" -ge 2 ] || die "--state-root requires a value"; STATE_ROOT="$2"; shift 2 ;;
    --sandbox-root) [ "$#" -ge 2 ] || die "--sandbox-root requires a value"; SANDBOX_ROOT="$2"; shift 2 ;;
    --owner) [ "$#" -ge 2 ] || die "--owner requires a value"; OWNER="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$SERVICE_ID" ] || die "--service-id is required"
[ -n "$ACCOUNT_ID" ] || die "--account-id is required"
[[ "$SERVICE_ID" =~ ^[A-Za-z0-9._-]+$ ]] || die "unsafe --service-id: $SERVICE_ID"
[[ "$ACCOUNT_ID" =~ ^[0-9a-fA-F-]+$ ]] || die "unsafe --account-id: $ACCOUNT_ID"
[[ "$OWNER" =~ ^[A-Za-z0-9._-]+:[A-Za-z0-9._-]+$ ]] || die "unsafe --owner: $OWNER"

command -v python3 >/dev/null 2>&1 || die "python3 is required"

DERIVED_KEY="$(python3 - "$SERVICE_ID" "$ACCOUNT_ID" <<'PY'
import hashlib, sys
service_id, account_id = sys.argv[1], sys.argv[2]
h = hashlib.sha256()
h.update(b"cowboy-account-v1\x00")
h.update(service_id.encode("utf-8"))
h.update(b"\x00")
h.update(account_id.encode("utf-8"))
print(h.hexdigest()[:16])
PY
)"

if [ -n "$DAEMON_KEY" ]; then
  [[ "$DAEMON_KEY" =~ ^[0-9a-f]{16}$ ]] || die "unsafe --daemon-key: $DAEMON_KEY"
  if [ "$DAEMON_KEY" != "$DERIVED_KEY" ]; then
    die "--daemon-key ($DAEMON_KEY) does not match derived key ($DERIVED_KEY) for service=$SERVICE_ID account=$ACCOUNT_ID"
  fi
else
  DAEMON_KEY="$DERIVED_KEY"
fi

NATIVE_ROOT="${STATE_ROOT}/services/${SERVICE_ID}/provider-auth/accounts/${ACCOUNT_ID}/providers/antigravity/native"
GENS_DIR="${NATIVE_ROOT}/generations"
TARGET_HOME="${NATIVE_ROOT}/home"
HOME_IMG="${SANDBOX_ROOT}/accounts/${DAEMON_KEY}/home.img"

if [ -z "$GENERATION" ]; then
  [ -d "$GENS_DIR" ] || die "generations directory not found: $GENS_DIR"
  GENERATION="$(find "$GENS_DIR" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | grep -E '^[0-9]+$' | sort -n | tail -n 1 || true)"
  [ -n "$GENERATION" ] || die "no numeric generation directories found under $GENS_DIR"
fi
[[ "$GENERATION" =~ ^[0-9]+$ ]] || die "unsafe --generation: $GENERATION"

PROFILE_JSON="${GENS_DIR}/${GENERATION}/profile.json"
PROFILE_BACKUP="${GENS_DIR}/${GENERATION}/profile.json.pre-migration.bak"

echo "=== Antigravity home migration ==="
echo "Mode:          $([ "$APPLY" -eq 1 ] && echo APPLY || echo DRY-RUN)"
echo "Service ID:    $SERVICE_ID"
echo "Account ID:    $ACCOUNT_ID"
echo "Daemon key:    $DAEMON_KEY"
echo "Generation:    $GENERATION"
echo "Source image:  $HOME_IMG"
echo "Target home:   $TARGET_HOME"
echo "Profile JSON:  $PROFILE_JSON"
echo "Owner:         $OWNER"

[ -f "$HOME_IMG" ] || die "source image not found: $HOME_IMG"
[ -f "$PROFILE_JSON" ] || die "profile.json not found: $PROFILE_JSON"

CURRENT_SANDBOX_FLAG="$(python3 - "$PROFILE_JSON" <<'PY'
import json, sys
with open(sys.argv[1], "r", encoding="utf-8") as fh:
    data = json.load(fh)
print("true" if data.get("account_sandbox") else "false")
PY
)"
echo "profile.json account_sandbox: $CURRENT_SANDBOX_FLAG"

# Check for active cga-* units or Antigravity processes.
ACTIVE_UNITS=""
if command -v systemctl >/dev/null 2>&1; then
  ACTIVE_UNITS="$(systemctl list-units --type=service --state=running,activating --no-legend 'cga-*.service' 2>/dev/null | awk '{print $1}' || true)"
fi
ACTIVE_PROCS="$(pgrep -fa '(agy-proxy|provider_cli-antigravity/content/antigravity|cowboy-antigravity)' 2>/dev/null | grep -v 'ovh-antigravity-migrate-home\.sh' || true)"

if [ -n "$ACTIVE_UNITS" ]; then
  echo "WARNING: active cga-* units detected:" >&2
  echo "$ACTIVE_UNITS" >&2
  [ "$APPLY" -eq 0 ] || die "refusing --apply while cga-* units are active; stop Antigravity sessions first"
fi
if [ -n "$ACTIVE_PROCS" ]; then
  echo "WARNING: active Antigravity processes detected:" >&2
  echo "$ACTIVE_PROCS" >&2
  [ "$APPLY" -eq 0 ] || die "refusing --apply while Antigravity processes are running"
fi

ATTACHED_LOOPS=""
if command -v losetup >/dev/null 2>&1; then
  ATTACHED_LOOPS="$(losetup -j "$HOME_IMG" 2>/dev/null || true)"
fi
if [ -n "$ATTACHED_LOOPS" ]; then
  echo "WARNING: $HOME_IMG is currently loop-attached:" >&2
  echo "$ATTACHED_LOOPS" >&2
  [ "$APPLY" -eq 0 ] || die "refusing --apply while $HOME_IMG is mounted or loop-attached"
fi

if [ "$APPLY" -eq 1 ] && [ "$(id -u)" -ne 0 ]; then
  die "--apply must run as root (needed to mount home.img read-only and set ownership)"
fi

MNT=""
STAGE=""
cleanup() {
  if [ -n "$MNT" ] && mountpoint -q "$MNT" 2>/dev/null; then
    umount "$MNT" || true
  fi
  if [ -n "$MNT" ] && [ -d "$MNT" ]; then
    rmdir "$MNT" || true
  fi
  if [ -n "$STAGE" ] && [ -d "$STAGE" ]; then
    rm -rf "$STAGE" || true
  fi
}
trap cleanup EXIT

if [ "$(id -u)" -ne 0 ]; then
  echo "Dry-run note: run with sudo to mount $HOME_IMG read-only and inspect inner files."
  ls -lh "$HOME_IMG" "$PROFILE_JSON"
  exit 0
fi

MNT="$(mktemp -d /tmp/cowboy-agy-migrate-mnt.XXXXXX)"
mount -o ro,loop,noload "$HOME_IMG" "$MNT"

SRC_CLI="${MNT}/.gemini/antigravity-cli"
[ -d "$SRC_CLI" ] || die "expected directory missing in home.img: $SRC_CLI"
[ -f "${SRC_CLI}/antigravity-oauth-token" ] || die "missing OAuth token in home.img: ${SRC_CLI}/antigravity-oauth-token"
[ -f "${SRC_CLI}/installation_id" ] || die "missing installation_id in home.img: ${SRC_CLI}/installation_id"

echo "Source summary:"
du -sh "$SRC_CLI"
echo "  conversations/*.db count: $(find "${SRC_CLI}/conversations" -maxdepth 1 -type f -name '*.db' 2>/dev/null | wc -l)"
echo "  brain/ entries:           $(find "${SRC_CLI}/brain" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)"
echo "  annotations/ files:       $(find "${SRC_CLI}/annotations" -maxdepth 1 -type f 2>/dev/null | wc -l)"

if [ "$APPLY" -eq 0 ]; then
  echo "Dry-run complete. Re-run with --apply to migrate into $TARGET_HOME and flip profile.json."
  exit 0
fi

if [ -e "${TARGET_HOME}/.gemini/antigravity-cli/antigravity-oauth-token" ] && [ "$FORCE_OVERWRITE" -eq 0 ]; then
  die "target already contains ${TARGET_HOME}/.gemini/antigravity-cli/antigravity-oauth-token (pass --force-overwrite to replace)"
fi

STAGE="${NATIVE_ROOT}/.home.migrating.$$"
mkdir -p "${STAGE}/.gemini/antigravity-cli"
chmod 0700 "$STAGE" "${STAGE}/.gemini" "${STAGE}/.gemini/antigravity-cli"
cp -a "${SRC_CLI}/." "${STAGE}/.gemini/antigravity-cli/"

chown -R "$OWNER" "$STAGE"
chmod 0700 "$STAGE" "${STAGE}/.gemini" "${STAGE}/.gemini/antigravity-cli"
chmod 0600 "${STAGE}/.gemini/antigravity-cli/antigravity-oauth-token"

echo "Verifying SHA-256 parity across every regular file in .gemini/antigravity-cli ..."
python3 - "$SRC_CLI" "${STAGE}/.gemini/antigravity-cli" <<'PY'
import hashlib, os, sys

src_root, dst_root = sys.argv[1], sys.argv[2]

def walk_files(root):
    result = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        filenames.sort()
        for name in filenames:
            full = os.path.join(dirpath, name)
            if os.path.islink(full):
                result.append((os.path.relpath(full, root), ("symlink", os.readlink(full))))
            elif os.path.isfile(full):
                h = hashlib.sha256()
                with open(full, "rb") as fh:
                    while True:
                        chunk = fh.read(1024 * 1024)
                        if not chunk:
                            break
                        h.update(chunk)
                result.append((os.path.relpath(full, root), ("file", h.hexdigest())))
    return result

src_entries = walk_files(src_root)
dst_entries = walk_files(dst_root)
if src_entries != dst_entries:
    src_map = dict(src_entries)
    dst_map = dict(dst_entries)
    missing = sorted(set(src_map) - set(dst_map))
    extra = sorted(set(dst_map) - set(src_map))
    diff = sorted(k for k in set(src_map) & set(dst_map) if src_map[k] != dst_map[k])
    raise SystemExit(
        f"SHA-256 verification failed: missing={missing[:5]} extra={extra[:5]} diff={diff[:5]}"
    )

file_count = sum(1 for _, (kind, _) in src_entries if kind == "file")
db_count = sum(1 for rel, (kind, _) in src_entries if kind == "file" and rel.startswith("conversations/") and rel.endswith(".db"))
print(f"SHA-256 verified: {file_count} files (including {db_count} conversations/*.db, antigravity-oauth-token, installation_id)")
PY

if [ -e "$TARGET_HOME" ]; then
  BACKUP_HOME="${NATIVE_ROOT}/home.pre-migration.$(date -u +%Y%m%dT%H%M%SZ)"
  mv "$TARGET_HOME" "$BACKUP_HOME"
  echo "Existing target home preserved at: $BACKUP_HOME"
fi
mv "$STAGE" "$TARGET_HOME"
STAGE=""

if [ ! -e "$PROFILE_BACKUP" ]; then
  cp -a "$PROFILE_JSON" "$PROFILE_BACKUP"
  chown "$OWNER" "$PROFILE_BACKUP"
  chmod 0600 "$PROFILE_BACKUP"
fi

TMP_PROFILE="${GENS_DIR}/${GENERATION}/.profile.json.migrating.$$"
python3 - "$PROFILE_JSON" "$TMP_PROFILE" <<'PY'
import json, sys
with open(sys.argv[1], "r", encoding="utf-8") as fh:
    data = json.load(fh)
data["account_sandbox"] = False
with open(sys.argv[2], "w", encoding="utf-8") as fh:
    json.dump(data, fh, indent=2)
    fh.write("\n")
PY
chown "$OWNER" "$TMP_PROFILE"
chmod 0600 "$TMP_PROFILE"
mv -f "$TMP_PROFILE" "$PROFILE_JSON"

echo "Migration complete!"
echo "  Migrated home:  $TARGET_HOME"
echo "  Updated profile: $PROFILE_JSON (account_sandbox=false)"
echo "  Profile backup: $PROFILE_BACKUP"
echo "  Untouched image: $HOME_IMG"
echo "Rollback command (if ever needed):"
echo "  mv -f '$PROFILE_BACKUP' '$PROFILE_JSON'  # then reinstall antigravity 0.2.43"
