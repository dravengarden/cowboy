import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from plugin_generation_failure_isolation import main, run
from plugin_runtime_conformance import Worker, WorkerStartupRejected


def inputs():
    return SimpleNamespace(release=Path("new.release.json"), artifacts=Path("new-artifacts"),
                           previous=Path("old.release.json"), previous_artifacts=Path("old-artifacts"),
                           worker=Path("/fixture-worker"))


def candidate(path, _artifacts, _root):
    old = path.name.startswith("old")
    return SimpleNamespace(id="fixture", version="1.0.0" if old else "2.0.0",
                           release={"artifact_digest": "sha256:" + ("a" if old else "b") * 64},
                           target={"os": "linux", "architecture": "x86_64"})


class FailureIsolationTests(unittest.TestCase):
    def test_explicit_rejection_is_observed_without_claiming_coexistence_or_release(self):
        current = Mock(spec=Worker)
        with patch("plugin_generation_failure_isolation.Candidate", side_effect=candidate), \
             patch("plugin_generation_failure_isolation.Worker", side_effect=[current, WorkerStartupRejected()]) as worker, \
             patch("plugin_generation_failure_isolation.digest", return_value="sha256:worker"):
            report = run(inputs())
        self.assertEqual([call.args[0].version for call in worker.call_args_list], ["2.0.0", "1.0.0"])
        self.assertEqual(current.assert_alive.call_count, 2)
        current.stop.assert_called_once()
        current.cleanup.assert_called_once()
        self.assertEqual(report["status"], "observed_failure_isolation")
        self.assertFalse(report["release_accepted"])
        self.assertEqual(report["distinct_generation_coexistence"], "not_proven")
        self.assertIn("existing_session_resume", report["not_checked"])
        self.assertTrue(all(report["checks"].values()))

    def test_timeout_handshake_probe_and_cleanup_errors_do_not_qualify(self):
        for error in [TimeoutError("startup"), ValueError("handshake"), RuntimeError("probe"),
                      RuntimeError("cleanup left a running descendant")]:
            with self.subTest(error=str(error)):
                current = Mock(spec=Worker)
                with patch("plugin_generation_failure_isolation.Candidate", side_effect=candidate), \
                     patch("plugin_generation_failure_isolation.Worker", side_effect=[current, error]):
                    with self.assertRaises(type(error)) as raised:
                        run(inputs())
                self.assertIs(raised.exception, error)
                current.cleanup.assert_called_once()
                current.stop.assert_not_called()

    def test_unexpectedly_ready_previous_requires_normal_coexistence_acceptance(self):
        current, previous = Mock(spec=Worker), Mock(spec=Worker)
        with patch("plugin_generation_failure_isolation.Candidate", side_effect=candidate), \
             patch("plugin_generation_failure_isolation.Worker", side_effect=[current, previous]):
            with self.assertRaisesRegex(RuntimeError, "normal coexistence gate"):
                run(inputs())
        current.cleanup.assert_called_once()
        previous.cleanup.assert_called_once()

    def test_candidate_failure_after_previous_cleanup_rejects_diagnostic(self):
        current = Mock(spec=Worker)
        current.assert_alive.side_effect = [None, RuntimeError("candidate died")]
        with patch("plugin_generation_failure_isolation.Candidate", side_effect=candidate), \
             patch("plugin_generation_failure_isolation.Worker", side_effect=[current, WorkerStartupRejected()]):
            with self.assertRaisesRegex(RuntimeError, "candidate died"):
                run(inputs())
        current.stop.assert_not_called()
        current.cleanup.assert_called_once()

    def test_same_generation_and_other_plugin_are_rejected_before_startup(self):
        for different_plugin in [False, True]:
            with self.subTest(different_plugin=different_plugin):
                new = candidate(Path("new"), None, None)
                old = candidate(Path("old"), None, None)
                if different_plugin:
                    old.id = "unrelated"
                else:
                    old.release = new.release
                with patch("plugin_generation_failure_isolation.Candidate", side_effect=[new, old]), \
                     patch("plugin_generation_failure_isolation.Worker") as worker:
                    with self.assertRaisesRegex(RuntimeError, "distinct exact releases"):
                        run(inputs())
                    worker.assert_not_called()

    def test_cli_never_writes_evidence_on_diagnostic_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = Path(temporary) / "diagnostic.json"
            argv = ["diagnostic", "new", "artifacts", "--worker", "/worker",
                    "--previous", "old", "--previous-artifacts", "old-artifacts", "--receipt", str(receipt)]
            with patch("sys.argv", argv), \
                 patch("plugin_generation_failure_isolation.require_worker_isolation"), \
                 patch("plugin_generation_failure_isolation.digest"), \
                 patch("plugin_generation_failure_isolation.run", side_effect=TimeoutError()):
                with self.assertRaises(TimeoutError):
                    main()
            self.assertFalse(receipt.exists())

    def test_cli_observation_is_private_create_only_and_never_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = Path(temporary) / "diagnostic.json"
            report = {"status": "observed_failure_isolation", "release_accepted": False}
            argv = ["diagnostic", "new", "artifacts", "--worker", "/worker",
                    "--previous", "old", "--previous-artifacts", "old-artifacts", "--receipt", str(receipt)]
            with patch("sys.argv", argv), \
                 patch("plugin_generation_failure_isolation.require_worker_isolation"), \
                 patch("plugin_generation_failure_isolation.digest"), \
                 patch("plugin_generation_failure_isolation.run", return_value=report) as diagnostic, \
                 patch("sys.stdout", new_callable=io.StringIO):
                main()
                with self.assertRaisesRegex(RuntimeError, "already exists"):
                    main()
                diagnostic.assert_called_once()
            self.assertEqual(json.loads(receipt.read_text()), report)
            self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
