import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

from catalog_reader_conformance import candidate_reads_both, command, immutable_identity, reader_arguments


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


if __name__ == "__main__":
    unittest.main()
