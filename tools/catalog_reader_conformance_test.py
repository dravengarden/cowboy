import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from catalog_reader_conformance import (candidate_reads_both, command, immutable_identity,
                                        publication_host_arguments, publication_reader_result,
                                        reader_arguments, unsigned_envelope)


class CatalogReaderHarnessTests(unittest.TestCase):
    def test_reader_arguments_are_read_only_and_explicit(self):
        for candidate, flag in [(False, "--check-plugin-catalog"), (True, "--check-plugin-hosts")]:
            args = reader_arguments(Path("/nix/store/fixture/bin/cowboy"), Path("/tmp/private"), Path("/tmp/catalog"), candidate)
            self.assertIn(flag, args)
            self.assertEqual(args[args.index("--bind") + 1], "127.0.0.1:0")
            self.assertEqual(args[args.index("--product-auth-enabled") + 1], "false")
            self.assertNotIn("--auth-config", args)
            self.assertNotIn("--database-url", args)

    def test_reader_children_never_inherit_service_credentials_or_ssh_agent(self):
        with patch.dict(os.environ, {"COWBOY_AUTH_CONFIG": "/private", "SSH_AUTH_SOCK": "/agent", "DEEPSEEK_API_KEY": "fixture"}), \
             patch("catalog_reader_conformance.subprocess.run", return_value=subprocess.CompletedProcess([], 0, "", "")) as run:
            command("fixture", "argument with spaces; not a shell")
            options = run.call_args.kwargs
            self.assertEqual(set(options["env"]), {"PATH", "LANG"})
            self.assertEqual(run.call_args.args[0], ["fixture", "argument with spaces; not a shell"])
            self.assertNotIn("shell", options)
            self.assertEqual(options["timeout"], 30)

    def test_public_receipt_identity_does_not_copy_other_release_fields(self):
        self.assertEqual(immutable_identity(dict(plugin_id="example", plugin_version="1.0.0", artifact_digest="sha256:fixture", signature="not-a-receipt-field")),
                         dict(plugin_id="example", plugin_version="1.0.0", artifact_digest="sha256:fixture"))

    def test_candidate_success_requires_both_exact_signed_identities(self):
        legacy = dict(plugin_id="legacy", plugin_version="1.0.0", artifact_digest="sha256:legacy")
        future = dict(plugin_id="future", plugin_version="2.0.0", artifact_digest="sha256:future")
        report = dict(schema="dravengarden.cowboy.plugin-host-preflight/v1", status="configuration_valid",
                      catalog_defaults=[dict(release=legacy, has_host_bundle=False), dict(release=future, has_host_bundle=True)])
        self.assertTrue(candidate_reads_both(report, legacy, future))
        report["catalog_defaults"][1]["has_host_bundle"] = False
        self.assertFalse(candidate_reads_both(report, legacy, future))
        report["catalog_defaults"].pop()
        self.assertFalse(candidate_reads_both(report, legacy, future))
        report["catalog_defaults"] = []
        self.assertFalse(candidate_reads_both(report, legacy, future))


class PublicationReaderTests(unittest.TestCase):
    def setUp(self):
        self.legacy = dict(plugin_id="legacy", plugin_version="1.0.0", artifact_digest="sha256:legacy")
        self.publication = dict(plugin_id="candidate", plugin_version="2.0.0", artifact_digest="sha256:candidate",
                                release_schema=2)

    def report(self, releases, supported=1):
        return subprocess.CompletedProcess([], 0, json.dumps(dict(
            schema="dravengarden.cowboy.catalog-reader-preflight/v1", status="readable",
            supported_release_schema=supported, releases=releases)), "")

    def inspect(self, result, allow_skip=True):
        return publication_reader_result(result, self.legacy, self.publication, allow_skip)["status"]

    def test_visible_requires_both_exact_identities_regardless_of_order(self):
        entries = [self.legacy, immutable_identity(self.publication)]
        self.assertEqual(self.inspect(self.report(entries)), "visible")
        self.assertEqual(self.inspect(self.report(list(reversed(entries)))), "visible")

    def test_only_a_future_outer_envelope_can_be_safely_skipped(self):
        self.assertEqual(self.inspect(self.report([self.legacy])), "skipped_future_envelope")
        self.assertEqual(self.inspect(self.report([self.legacy]), allow_skip=False), "unexpected_inventory")
        self.publication["release_schema"] = 1
        self.assertEqual(self.inspect(self.report([self.legacy])), "unexpected_inventory")

    def test_skipped_schema_needs_an_explicit_positive_integer_reader_limit(self):
        for supported in [None, "1", True, 0, -1, 2]:
            with self.subTest(supported=supported):
                self.assertEqual(self.inspect(self.report([self.legacy], supported)), "unexpected_inventory")

    def test_missing_extra_duplicate_or_changed_identity_is_not_compatibility(self):
        publication = immutable_identity(self.publication)
        for entries in [[], [publication], [self.legacy, publication, publication],
                        [self.legacy, dict(publication, artifact_digest="sha256:other")],
                        [dict(self.legacy, plugin_version="0.9.0"), publication],
                        [self.legacy, dict(publication, unsigned_extra=True)]]:
            with self.subTest(entries=entries):
                self.assertEqual(self.inspect(self.report(entries)), "unexpected_inventory")

    def test_unsupported_nested_payload_failure_is_not_a_safe_skip(self):
        result = subprocess.CompletedProcess([], 1, "", "unknown variant code_intelligence_server")
        report = publication_reader_result(result, self.legacy, self.publication, allow_skip=True)
        self.assertEqual(report["status"], "rejected")
        self.assertEqual(report["exit_code"], 1)
        self.assertIn("code_intelligence_server", report["detail"])

    def test_invalid_or_wrong_diagnostic_never_counts_as_reader_acceptance(self):
        for stdout in ["not JSON", "[]", "null", "{}", '{"status":"readable"}']:
            with self.subTest(stdout=stdout):
                self.assertEqual(self.inspect(subprocess.CompletedProcess([], 0, stdout, "")), "invalid_report")

    def test_fixture_signature_is_the_only_excluded_proof_field(self):
        release = dict(self.publication, artifact_url="https://fixture.invalid/exact", signature="production",
                       runtime_artifacts=[dict(components=[dict(artifact_digest="sha256:runtime")])])
        fixture = dict(release, signature="temporary")
        self.assertEqual(unsigned_envelope(release), unsigned_envelope(fixture))
        for name, value in [("artifact_digest", "sha256:changed"), ("artifact_url", "https://fixture.invalid/other"),
                            ("runtime_artifacts", []), ("host_bundle_digest", "sha256:new")]:
            with self.subTest(name=name):
                self.assertNotEqual(unsigned_envelope(release), unsigned_envelope(dict(fixture, **{name: value})))

    def test_bound_host_uses_exact_private_fixture_policy_without_initialization(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            publication = dict(self.publication, host_bundle_digest="sha256:host")
            args = publication_host_arguments(Path("/nix/store/fixture/bin/cowboy"), root / "data", root / "catalog", publication)
            policy = args[args.index("--plugin-host-config") + 1]
            self.assertEqual(policy.parent, root)
            self.assertEqual(policy.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(policy.read_text()), dict(
                schema="dravengarden.cowboy.plugin-host-activation/v1", source_policy="bootstrap",
                hosts=[immutable_identity(publication)]))
            self.assertIn("--check-plugin-hosts", args)
            self.assertFalse((root / "data").exists())
            self.assertFalse((root / "catalog").exists())
            with self.assertRaises(FileExistsError):
                publication_host_arguments(Path("/nix/store/fixture/bin/cowboy"), root / "data", root / "catalog", publication)

    def test_hostless_release_does_not_get_an_invalid_host_pin(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = publication_host_arguments(Path("/nix/store/fixture/bin/cowboy"), root / "data", root / "catalog", self.publication)
            self.assertNotIn("--plugin-host-config", args)
            self.assertEqual(list(root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
