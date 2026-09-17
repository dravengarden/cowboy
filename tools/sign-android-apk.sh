#!/usr/bin/env bash
# Sign one unsigned Android release APK recorded by build-native-shell.sh.
# Signing is deliberately separate from the build: the build entry never reads
# credentials, and this step accepts only an exact, receipt-verified artifact.
set -euo pipefail
if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
  echo "usage: bash tools/sign-android-apk.sh /absolute/build/receipt.json [--receipt-path /absolute/new.json]" >&2
  exit 2
fi
sign_build_receipt="$1"
sign_report_target=
if [ "${2:-}" = --receipt-path ]; then
  sign_report_target="${3:?--receipt-path requires an absolute path}"
  case "$sign_report_target" in /*) ;; *) echo "receipt path must be absolute" >&2; exit 2;; esac
  test ! -e "$sign_report_target" && test ! -L "$sign_report_target" || {
    echo "refusing to overwrite a signing receipt" >&2; exit 1;
  }
elif [ "$#" != 1 ]; then
  echo "unknown argument: $2" >&2; exit 2
fi
test "${COWBOY_NATIVE_ANDROID_SHELL:-}" = 1 || {
  echo "Run Android signing inside: nix develop .#native-android" >&2; exit 1;
}
: "${ANDROID_HOME:?Set ANDROID_HOME to the SDK Manager-owned Android SDK}"
# The keystore and its password file live outside every checkout. Name them
# explicitly; there is no default key.
: "${COWBOY_ANDROID_KEYSTORE:?Set COWBOY_ANDROID_KEYSTORE to the release keystore}"
: "${COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE:?Set COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE}"
sign_alias="${COWBOY_ANDROID_KEY_ALIAS:-cowboy}"
case "$COWBOY_ANDROID_KEYSTORE" in /*) ;; *) echo "keystore path must be absolute" >&2; exit 2;; esac
test -f "$COWBOY_ANDROID_KEYSTORE" && test -f "$COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE"

python3 - "$sign_build_receipt" "$sign_report_target" "$sign_alias" <<'PY'
import hashlib, json, os, pathlib, re, subprocess, sys, tempfile
receipt_path, report_target, alias = sys.argv[1:]
receipt_path = pathlib.Path(receipt_path)
if not receipt_path.is_absolute():
    raise SystemExit("build receipt path must be absolute")
build = json.loads(receipt_path.read_text())
if build.get("platform") != "android" or build.get("profile") != "release":
    raise SystemExit("only arm64 release builds from the android platform are signed")
if build.get("signing") != "unsigned":
    raise SystemExit("build receipt does not describe an unsigned APK")
apk = pathlib.Path(build["apk"])
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
if digest(apk) != build["apk_sha256"]:
    raise SystemExit("APK bytes differ from the build receipt")
tools = pathlib.Path(os.environ["ANDROID_HOME"]) / "build-tools" / build["toolchain"]["android"]["buildTools"]
out_dir = receipt_path.parent
signed = out_dir / f"Cowboy-{build['version_name']}-{build['version_code']}-{build['source_revision'][:12]}.apk"
if signed.exists():
    raise SystemExit("refusing to overwrite a signed APK: " + str(signed))
with tempfile.TemporaryDirectory(dir=out_dir) as work:
    aligned = pathlib.Path(work) / "aligned.apk"
    subprocess.run([str(tools / "zipalign"), "-P", "16", "-f", "4", str(apk), str(aligned)], check=True)
    # apksigner is a `#!/bin/bash` wrapper, which a NixOS host does not provide.
    subprocess.run(["bash", str(tools / "apksigner"), "sign",
        "--ks", os.environ["COWBOY_ANDROID_KEYSTORE"], "--ks-key-alias", alias,
        "--ks-pass", "file:" + os.environ["COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE"],
        "--key-pass", "file:" + os.environ["COWBOY_ANDROID_KEYSTORE_PASSWORD_FILE"],
        "--out", str(signed), str(aligned)], check=True)
verify = subprocess.run(["bash", str(tools / "apksigner"), "verify", "--verbose", "--print-certs", str(signed)],
    check=True, capture_output=True, text=True).stdout
certificate = re.search(r"certificate SHA-256 digest: ([0-9a-f]{64})", verify)
schemes = sorted(set(re.findall(r"Verified using (v\d(?:\.\d)?) scheme[^:]*: true", verify)))
if not certificate or not schemes:
    raise SystemExit("apksigner did not verify the signed APK")
report = dict(build_receipt=str(receipt_path), build_receipt_sha256=digest(receipt_path),
    source_revision=build["source_revision"], unsigned_apk_sha256=build["apk_sha256"],
    apk=str(signed), apk_sha256=digest(signed), application_id=build["application_id"],
    version_code=build["version_code"], version_name=build["version_name"],
    certificate_sha256=certificate.group(1), signature_schemes=schemes, key_alias=alias,
    installed=False, physical_device="not_checked")
(out_dir / "signing-receipt.json").write_text(json.dumps(report, indent=2) + "\n")
if report_target:
    with pathlib.Path(report_target).open("x") as receipt:
        receipt.write(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
PY
