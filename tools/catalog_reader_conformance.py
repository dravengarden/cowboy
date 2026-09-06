#!/usr/bin/env python3
"""Compare exact Controller readers using public legacy bytes and a temporary signed fixture."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from plugin_runtime_conformance import require_worker_isolation

ROOT = Path(__file__).resolve().parent.parent


def require(value, message):
    if not value:
        raise RuntimeError(message)


def digest(path):
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def command(*args, success=True):
    # No inherited Service/Provider credentials, agent homes, SSH agent, DB URL
    # or private authentication configuration. The network namespace is closed.
    result = subprocess.run([str(arg) for arg in args], cwd=ROOT,
                            env={"PATH": os.environ["PATH"], "LANG": "C.UTF-8"},
                            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=30, check=False)
    if success:
        require(result.returncode == 0, "fixture command failed: " + result.stderr[-2000:])
    return result


def controller_release(path):
    require(path.is_absolute(), "Controller release must be absolute")
    path = path.resolve(strict=True)
    require(path.parent == Path("/nix/store"), "Controller must be an immutable Nix release")
    source = json.loads((path / "etc/cowboy-release/source.json").read_text())
    require(source.get("component") == "cowboy" and source.get("lane") == "controller"
            and source.get("dirty") is False
            and re.fullmatch(r"[a-f0-9]{40}", source.get("revision", "")),
            "Controller release lacks clean exact source provenance")
    executable = (path / "bin/cowboy").resolve(strict=True)
    return executable, dict(release=str(path), source_revision=source["revision"],
                            executable_sha256=digest(executable))


def reader_arguments(executable, data, catalog, candidate=False):
    return [executable, "serve", "--check-plugin-hosts" if candidate else "--check-plugin-catalog",
            "--data-dir", data, "--plugin-catalog-dir", catalog,
            "--workspace-root", data, "--web-root", data,
            "--product-auth-enabled", "false", "--bind", "127.0.0.1:0"]


def immutable_identity(release):
    return {name: release[name] for name in ("plugin_id", "plugin_version", "artifact_digest")}


def candidate_reads_both(report, legacy, future):
    defaults = report.get("catalog_defaults", [])
    return (report.get("schema") == "dravengarden.cowboy.plugin-host-preflight/v1"
            and report.get("status") == "configuration_valid"
            and all(dict(release=immutable_identity(release), has_host_bundle=has_host) in defaults
                    for release, has_host in ((legacy, False), (future, True))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bridge_release", type=Path)
    parser.add_argument("baseline_release", type=Path)
    parser.add_argument("candidate_release", type=Path)
    parser.add_argument("pack", type=Path)
    parser.add_argument("legacy_package", type=Path, help="public signed legacy package; never private auth config")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    require_worker_isolation()
    require(not command("git", "status", "--porcelain").stdout.strip(), "Conformance needs clean committed source")
    revision = command("git", "rev-parse", "HEAD").stdout.strip()
    bridge, bridge_record = controller_release(args.bridge_release)
    baseline, baseline_record = controller_release(args.baseline_release)
    candidate, candidate_record = controller_release(args.candidate_release)
    require(bridge_record["source_revision"] != baseline_record["source_revision"], "Bridge must differ from baseline")
    command("git", "merge-base", "--is-ancestor", baseline_record["source_revision"], bridge_record["source_revision"])
    require(args.pack.is_absolute(), "SDK pack tool must be absolute")
    pack = args.pack.resolve(strict=True)
    require(Path("/nix/store") in pack.parents and os.access(pack, os.X_OK), "SDK verifier must be an immutable executable")
    legacy_package = args.legacy_package.resolve(strict=True)
    require(legacy_package.suffix == ".cowboy-plugin", "Expected a public Plugin package")
    legacy_release = legacy_package.with_suffix(".release.json")
    legacy = json.loads(legacy_release.read_text())
    require(legacy.get("release_schema") == 1 and not legacy.get("host_bundle_digest"), "Expected a hostless legacy schema-1 fixture")
    require(re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", legacy.get("publisher", "")), "Invalid public publisher identity")
    legacy_key = legacy_package.parent / "trusted-publishers" / (legacy["publisher"] + ".pub")
    original_digests = {path: digest(path) for path in (legacy_package, legacy_release, legacy_key)}
    command(pack, "verify", legacy_package, legacy_release, legacy_key)
    tests = []

    def check(name, passed):
        require(passed, "Catalog reader conformance failed: " + name)
        tests.append(name)

    with tempfile.TemporaryDirectory(prefix="cowboy-catalog-readers-") as temporary:
        root = Path(temporary)
        catalog = root / "catalog"
        trust = catalog / "trusted-publishers"
        trust.mkdir(parents=True)
        old_package = catalog / "legacy.cowboy-plugin"
        old_release = old_package.with_suffix(".release.json")
        shutil.copyfile(legacy_package, old_package)
        shutil.copyfile(legacy_release, old_release)
        shutil.copyfile(legacy_key, trust / legacy_key.name)
        data = root / "not-created-service"

        def inspect_bridge():
            output = command(*reader_arguments(bridge, data, catalog))
            report = json.loads(output.stdout)
            require(report.get("schema") == "dravengarden.cowboy.catalog-reader-preflight/v1"
                    and report.get("status") == "readable", "Unexpected reader report")
            require(report.get("releases") == [immutable_identity(legacy)], "Reader changed the exact legacy selection")
            require(not data.exists(), "Read-only inspection created Service state")
            return report

        inspect_bridge()
        check("bridge reads exact public legacy release without initialization", True)
        package = root / "password.cowboy-plugin"
        envelope = package.with_suffix(".release.json")
        host = package.with_suffix(".hostbundle.json")
        command(pack, "build", ROOT / "examples/authentication/password", package)
        command(pack, "set-artifact-url", package, envelope,
                "https://fixtures.invalid/plugin-artifacts/" + digest(package)[7:] + "/password.cowboy-plugin")
        key = root / "fixture-publisher"
        command("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", key)
        command(pack, "sign", package, envelope, key, host)
        command(pack, "verify", package, envelope, key.with_suffix(".pub"), host)
        future = json.loads(envelope.read_text())
        check("schema-2 fixture independently verified", future["release_schema"] == 2)
        require(future["publisher"] != legacy["publisher"], "Fixture must not replace the legacy trust key")
        shutil.copyfile(key.with_suffix(".pub"), trust / (future["publisher"] + ".pub"))
        new_package = catalog / "future.cowboy-plugin"
        new_release = new_package.with_suffix(".release.json")
        shutil.copyfile(package, new_package)
        shutil.copyfile(host, new_package.with_suffix(".hostbundle.json"))
        inspect_bridge()
        check("uncommitted future package cannot break old reader", True)
        shutil.copyfile(envelope, new_release)
        inspect_bridge()
        check("mixed formats retain only supported signed legacy identity", True)
        inspect_bridge()
        check("cold reader restart keeps legacy identity", True)

        # The actual deployed predecessor must fail for the intended Catalog
        # incompatibility, before DB/listener/session initialization. It receives
        # only a new disposable data root and a loopback-only network namespace.
        baseline_data = root / "baseline-service"
        baseline_args = reader_arguments(baseline, baseline_data, catalog)
        baseline_args.remove("--check-plugin-catalog")
        rejected = command(*baseline_args, success=False)
        check("actual baseline rejects future package before service startup",
              rejected.returncode != 0 and "validating Plugin artifact" in rejected.stderr)
        latest = command(*reader_arguments(candidate, data, catalog, candidate=True))
        latest_report = json.loads(latest.stdout)
        check("candidate Controller reads both signed release formats",
              candidate_reads_both(latest_report, legacy, future) and not data.exists())

        # This only corrupts a disposable COPY, never a published signed release.
        bad = dict(legacy, signature="")
        old_release.write_text(json.dumps(bad))
        check("bad supported signature is rejected",
              command(*reader_arguments(bridge, data, catalog), success=False).returncode != 0)
        shutil.copyfile(legacy_release, old_release)
        for name in (".catalog-only-v1", "live/fixture/.catalog-authority-v1"):
            marker = data / "plugins" / name
            marker.parent.mkdir(parents=True, exist_ok=True)
            marker.write_text("fixture-authority\n")
            result = command(*reader_arguments(bridge, data, catalog), success=False)
            check("activated host authority blocks legacy reader: " + name,
                  result.returncode != 0 and "cannot run after Plugin host authority activation" in result.stderr)
            # Also exercise normal startup, before it can create Service state.
            startup = reader_arguments(bridge, data, catalog)
            startup.remove("--check-plugin-catalog")
            result = command(*startup, success=False)
            check("normal startup preserves host authority: " + name,
                  result.returncode != 0 and "cannot run after Plugin host authority activation" in result.stderr
                  and sorted(path.name for path in data.iterdir()) == ["plugins"])
            marker.unlink()

        check("public legacy inputs remain byte-identical", all(digest(path) == value for path, value in original_digests.items()))
        report = dict(schema="dravengarden.cowboy.catalog-reader-conformance/v1", ok=True,
                      acceptance_revision=revision, tests=tests,
                      baseline=baseline_record, bridge=bridge_record, candidate=candidate_record,
                      verifier=dict(path=str(pack), sha256=digest(pack)),
                      legacy=immutable_identity(legacy), future_fixture=immutable_identity(future),
                      catalog="temporary_only", network="isolated_loopback_only",
                      production_signing=False, production_publication=False, activation=False,
                      not_checked=["active_and_rollback_profile_floor", "host_policy_cutover", "database_rollback", "real_login"])
    # The temporary fixture publisher private key is already deleted here.
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x") as output:
        output.write(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
