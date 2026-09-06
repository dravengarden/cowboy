#!/usr/bin/env python3
"""Compare exact Controller readers using public legacy bytes and temporary signed fixtures."""
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


def unsigned_envelope(release):
    return {name: value for name, value in release.items() if name != "signature"}


def publication_reader_result(result, legacy, publication, allow_skip=False, code_payload_schema=None):
    if result.returncode != 0:
        return dict(status="rejected", exit_code=result.returncode, detail=result.stderr[-2000:])
    try:
        report = json.loads(result.stdout)
        require(isinstance(report, dict)
                and report.get("schema") == "dravengarden.cowboy.catalog-reader-preflight/v1"
                and report.get("status") == "readable", "Unexpected Catalog report")
        actual = report.get("releases")
        both = [immutable_identity(legacy), immutable_identity(publication)]
        if actual == both or actual == list(reversed(both)):
            return dict(status="visible")
        supported = report.get("supported_release_schema")
        if (allow_skip and type(supported) is int and supported > 0
                and publication["release_schema"] > supported
                and actual == [immutable_identity(legacy)]):
            return dict(status="skipped_future_envelope")
        supported_code = report.get("supported_code_payload_schema")
        # A missing identity is not evidence of compatibility. The bridge must
        # explicitly advertise its nested-format limit, and the caller supplies
        # the Code schema only after the exact package passes SDK verification.
        if (allow_skip and type(supported) is int and supported > 0
                and type(publication["release_schema"]) is int
                and 0 < publication["release_schema"] <= supported
                and publication.get("plugin_kind") == "code_intelligence"
                and type(code_payload_schema) is int
                and type(supported_code) is int and 0 < supported_code < code_payload_schema
                and actual == [immutable_identity(legacy)]):
            return dict(status="skipped_future_code_payload")
        return dict(status="unexpected_inventory")
    except (ValueError, RuntimeError, KeyError, TypeError):
        return dict(status="invalid_report")


def publication_host_arguments(executable, data, catalog, publication):
    arguments = reader_arguments(executable, data, catalog, candidate=True)
    if publication.get("host_bundle_digest"):
        # Storage hosts deliberately never become defaults from publication
        # alone. Validate the exact host using temporary read-only policy,
        # without granting any production migration/activation authority.
        policy = catalog.parent / "candidate-host-policy.json"
        with policy.open("x") as output:
            output.write(json.dumps(dict(schema="dravengarden.cowboy.plugin-host-activation/v1",
                                         source_policy="bootstrap", hosts=[immutable_identity(publication)])))
        policy.chmod(0o600)
        arguments.extend(["--plugin-host-config", policy])
    return arguments


def publication_preflight(index, envelope, root, pack, key, bridge, candidate,
                          legacy_package, legacy_release, legacy_key, legacy):
    require(envelope.name.endswith(".release.json"), "Publication input must be a release envelope")
    stem = envelope.name.removesuffix(".release.json")
    package = envelope.with_name(stem + ".cowboy-plugin")
    host = envelope.with_name(stem + ".hostbundle.json")
    require(envelope.is_file() and not envelope.is_symlink(), "Publication envelope must be a regular file")
    publication = json.loads(envelope.read_text())
    require(publication.get("publisher") != legacy["publisher"],
            "Choose a public legacy fixture with a different publisher; never replace its trust key")
    require(re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", publication.get("publisher", "")),
            "Invalid publication publisher")
    sources = [envelope, package]
    if publication.get("host_bundle_digest"):
        sources.append(host)
    else:
        require(not host.exists() and not host.is_symlink(), "Publication has an unbound host bundle")
    require(all(path.is_file() and not path.is_symlink() for path in sources),
            "Publication inputs must be regular files")
    original = {path: digest(path) for path in sources}

    # Each exact release has an independent Catalog. Keep the legacy signature
    # and trust key intact; sign only the candidate COPY with the disposable
    # fixture key. No manifest, URL, runtime matrix or composite digest changes.
    catalog = root / f"publication-{index}" / "catalog"
    trust = catalog / "trusted-publishers"
    trust.mkdir(parents=True)
    shutil.copyfile(legacy_package, catalog / "legacy.cowboy-plugin")
    shutil.copyfile(legacy_release, catalog / "legacy.release.json")
    shutil.copyfile(legacy_key, trust / legacy_key.name)
    fixture_package = catalog / "candidate.cowboy-plugin"
    fixture_release = catalog / "candidate.release.json"
    shutil.copyfile(package, fixture_package)
    shutil.copyfile(envelope, fixture_release)
    host_args = []
    if host in sources:
        fixture_host = catalog / "candidate.hostbundle.json"
        shutil.copyfile(host, fixture_host)
        host_args.append(fixture_host)
    command(pack, "sign", fixture_package, fixture_release, key, *host_args)
    command(pack, "verify", fixture_package, fixture_release, key.with_suffix(".pub"), *host_args)
    require(unsigned_envelope(json.loads(fixture_release.read_text())) == unsigned_envelope(publication),
            "Fixture signing changed the candidate release proof")
    code_payload_schema = None
    if publication["plugin_kind"] == "code_intelligence":
        payload = json.loads(fixture_package.read_text())["payload"]
        require(payload["kind"] == "code_intelligence", "Verified Code package has a different payload kind")
        code_payload_schema = payload["contract"]["schema_version"]
        require(type(code_payload_schema) is int and code_payload_schema > 0, "Invalid verified Code payload schema")
    shutil.copyfile(key.with_suffix(".pub"), trust / (publication["publisher"] + ".pub"))
    data = catalog.parent / "not-created-service"
    bridge_results = [publication_reader_result(
        command(*reader_arguments(bridge, data, catalog), success=False), legacy, publication,
        allow_skip=True, code_payload_schema=code_payload_schema)
        for _ in range(2)]
    candidate_result = publication_reader_result(
        command(*reader_arguments(candidate, data, catalog), success=False), legacy, publication)
    host_result = command(*publication_host_arguments(candidate, data, catalog, publication), success=False)
    host_valid = False
    if host_result.returncode == 0:
        try:
            host_report = json.loads(host_result.stdout)
            host_valid = (isinstance(host_report, dict)
                          and host_report.get("schema") == "dravengarden.cowboy.plugin-host-preflight/v1"
                          and host_report.get("status") == "configuration_valid"
                          and dict(release=immutable_identity(publication), has_host_bundle=bool(host_args))
                          in host_report.get("catalog_defaults", []))
        except (ValueError, TypeError):
            pass
    require(not data.exists(), "Publication reader inspection created Service state")
    require(all(digest(path) == value for path, value in original.items()), "Publication inputs changed during inspection")
    return dict(release=immutable_identity(publication), release_schema=publication["release_schema"],
                code_payload_schema=code_payload_schema,
                source_envelope_sha256=original[envelope], package_digest=original[package],
                host_bundle_digest=original.get(host), bridge_cold_reads=bridge_results,
                candidate=candidate_result, candidate_host_preflight=host_valid,
                candidate_host_policy="temporary_exact_pin" if host_args else "bootstrap_without_pin",
                candidate_host_error=None if host_valid else (host_result.stderr or host_result.stdout)[-2000:],
                reader_compatible=(all(result["status"] in ("visible", "skipped_future_envelope", "skipped_future_code_payload")
                                       for result in bridge_results)
                                   and candidate_result["status"] == "visible" and host_valid),
                production_signature_checked=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bridge_release", type=Path)
    parser.add_argument("baseline_release", type=Path)
    parser.add_argument("candidate_release", type=Path)
    parser.add_argument("pack", type=Path)
    parser.add_argument("legacy_package", type=Path, help="public signed legacy package; never private auth config")
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--publication", type=Path, action="append", default=[],
                        help="exact fully bound candidate envelope; repeat for each release, even with an old outer schema")
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

        publications = [publication_preflight(index, envelope, root, pack, key, bridge, candidate,
                                             legacy_package, legacy_release, legacy_key, legacy)
                        for index, envelope in enumerate(args.publication)]
        check("public legacy inputs remain byte-identical", all(digest(path) == value for path, value in original_digests.items()))
        report = dict(schema="dravengarden.cowboy.catalog-reader-conformance/v1",
                      ok=all(entry["reader_compatible"] for entry in publications),
                      acceptance_revision=revision, tests=tests,
                      baseline=baseline_record, bridge=bridge_record, candidate=candidate_record,
                      verifier=dict(path=str(pack), sha256=digest(pack)),
                      legacy=immutable_identity(legacy), future_fixture=immutable_identity(future),
                      publication_preflights=publications,
                      catalog="temporary_only", network="isolated_loopback_only",
                      production_signing=False, production_publication=False, activation=False,
                      not_checked=["active_and_rollback_profile_floor", "complete_production_catalog", "production_signatures",
                                   "runtime_execution", "host_policy_cutover", "database_rollback", "real_login"])
    # The temporary fixture publisher private key is already deleted here.
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    with args.receipt.open("x") as output:
        output.write(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if not report["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
