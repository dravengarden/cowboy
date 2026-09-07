import io
import json
import os
from pathlib import Path
import socket
import struct
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from plugin_runtime_conformance import (
    SessionStartupRejected, Worker, cleanup_workers, closed_environment, digest, extract_archive, fixture_session_id, read_frame,
    main, require_worker_isolation, run_conformance, validate_receipt_paths,
    write_frame, write_receipt,
)


class ConformanceHarnessTests(unittest.TestCase):
    def test_only_terminal_pre_native_events_are_startup_rejections(self):
        worker = Worker.__new__(Worker)
        worker.receive = Mock(return_value={"type": "worker_event", "event": {"event": "status", "state": "crashed"}})
        with self.assertRaises(SessionStartupRejected):
            worker.wait_ready()

    def test_native_allocation_cannot_be_forgotten_before_failure(self):
        worker = Worker.__new__(Worker)
        worker.candidate = SimpleNamespace(release={"artifact_digest": "sha256:fixture"})
        worker.receive = Mock(side_effect=[
            {"type": "worker_event", "event": {"event": "agent_session_id", "agent_session_id": "native-retained"}},
            {"type": "snapshot", "worker": {
                "launch": {"provider_generation_digest": "sha256:fixture"}, "state": "starting"}},
            {"type": "worker_event", "event": {"event": "status", "state": "crashed"}},
        ])
        with self.assertRaisesRegex(RuntimeError, "after native allocation") as error:
            worker.wait_ready()
        self.assertNotIsInstance(error.exception, SessionStartupRejected)

    def test_cleanup_rejects_live_descendants_and_still_closes_fixture_handles(self):
        worker = Worker.__new__(Worker)
        worker.process = Mock(pid=123)
        worker.child_pids = {456}
        worker.connection, worker.listener, worker.log = Mock(), Mock(), Mock()
        with patch("plugin_runtime_conformance.descendants", return_value={456}), \
             patch("plugin_runtime_conformance.is_running", return_value=True), \
             patch("plugin_runtime_conformance.os.kill") as kill, \
             patch("plugin_runtime_conformance.stop_group"), \
             patch("plugin_runtime_conformance.time.monotonic", side_effect=[0, 10]):
            with self.assertRaisesRegex(RuntimeError, "cleanup left a running descendant"):
                worker.cleanup()
            self.assertEqual(kill.call_args.args[0], 456)
        worker.connection.close.assert_called_once()
        worker.listener.close.assert_called_once()
        worker.log.close.assert_called_once()

    def test_cleanup_attempts_every_generation_even_when_one_fails(self):
        current, previous = Mock(), Mock()
        failure = RuntimeError("previous cleanup failed")
        previous.cleanup.side_effect = failure
        with self.assertRaises(RuntimeError) as error:
            cleanup_workers([current, previous])
        self.assertIs(error.exception, failure)
        previous.cleanup.assert_called_once()
        current.cleanup.assert_called_once()

    def test_receipts_are_complete_private_and_create_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "receipt.json"
            value = {"status": "passed", "plugin_id": "fixture"}
            write_receipt(path, value)
            self.assertEqual(json.loads(path.read_text()), value)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                write_receipt(path, {"status": "failed"})
            self.assertEqual(json.loads(path.read_text()), value)
            self.assertEqual(list(root.iterdir()), [path])

    def test_receipts_reject_collisions_and_dangling_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "receipt.json"
            validate_receipt_paths(path, root / "failure.json")
            with self.assertRaisesRegex(RuntimeError, "different paths"):
                validate_receipt_paths(path, root / "missing/../receipt.json")
            path.symlink_to(root / "absent.json")
            with self.assertRaisesRegex(RuntimeError, "already exists"):
                validate_receipt_paths(path, None)
            with self.assertRaises(FileExistsError):
                write_receipt(path, {"status": "failed"})
            self.assertTrue(path.is_symlink())
            self.assertFalse((root / "absent.json").exists())
            self.assertEqual(list(root.iterdir()), [path])

    def test_existing_receipt_stops_before_starting_any_runtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "accepted.json"
            write_receipt(path, {"accepted": True})
            with patch("sys.argv", ["conformance", "release", "artifacts", "--receipt", str(path)]), \
                 patch("plugin_runtime_conformance.Candidate") as candidate:
                with self.assertRaisesRegex(RuntimeError, "already exists"):
                    main()
                candidate.assert_not_called()
            self.assertEqual(json.loads(path.read_text()), {"accepted": True})

    def test_failure_receipt_is_separate_redacted_and_does_not_suppress_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            accepted, failed = root / "accepted.json", root / "failed.json"
            def fail(_args, progress):
                progress.update(phase="previous_worker_startup", previous={
                    "plugin_id": "fixture", "plugin_version": "1.0.0", "artifact_digest": "sha256:" + "a" * 64})
                raise RuntimeError("sensitive exception, argv, or worker log must not be copied")
            with patch("sys.argv", ["conformance", "release", "artifacts", "--receipt", str(accepted),
                                    "--failure-receipt", str(failed)]), \
                 patch("plugin_runtime_conformance.run_conformance", side_effect=fail):
                with self.assertRaisesRegex(RuntimeError, "sensitive exception"):
                    main()
            self.assertFalse(accepted.exists())
            report = json.loads(failed.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["phase"], "previous_worker_startup")
            self.assertEqual(report["exception_type"], "RuntimeError")
            self.assertFalse(report["acceptance_receipt_created"])
            self.assertFalse(report["uses_service_credentials"])
            self.assertNotIn("sensitive", failed.read_text())

    def test_success_does_not_create_failure_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            accepted, failed = root / "accepted.json", root / "failed.json"
            expected = {"schema_version": 1, "plugin_id": "fixture", "probes": []}
            with patch("sys.argv", ["conformance", "release", "artifacts", "--receipt", str(accepted),
                                    "--failure-receipt", str(failed)]), \
                 patch("plugin_runtime_conformance.run_conformance", return_value=expected), \
                 patch("sys.stdout", new_callable=io.StringIO) as stdout:
                main()
            self.assertEqual(json.loads(stdout.getvalue()), expected)
            self.assertEqual(json.loads(accepted.read_text()), expected)
            self.assertFalse(failed.exists())

    def test_missing_previous_artifacts_is_a_failure_not_runtime_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            failed = Path(temporary) / "failed.json"
            with patch("sys.argv", ["conformance", "release", "artifacts", "--previous", "old",
                                    "--failure-receipt", str(failed)]), \
                 patch("plugin_runtime_conformance.run_conformance") as run:
                with self.assertRaisesRegex(RuntimeError, "previous release needs"):
                    main()
                run.assert_not_called()
            self.assertEqual(json.loads(failed.read_text())["phase"], "input_validation")

    def test_failed_previous_startup_never_starts_the_candidate_worker(self):
        args = SimpleNamespace(release=Path("new.release.json"), artifacts=Path("new-artifacts"),
                               previous=Path("old.release.json"), previous_artifacts=Path("old-artifacts"),
                               worker=Path("/fixture-worker"))
        def candidate(path, _artifacts, _root):
            old = path.name.startswith("old")
            return SimpleNamespace(id="fixture", version="1.0.0" if old else "2.0.0",
                                   release={"artifact_digest": "sha256:" + ("a" if old else "b") * 64},
                                   target={"os": "linux", "architecture": "x86_64"}, probes=[])
        progress = {"phase": "input_validation", "completed": []}
        with patch("plugin_runtime_conformance.Candidate", side_effect=candidate), \
             patch("plugin_runtime_conformance.Worker", side_effect=RuntimeError("authentication required")) as worker:
            with self.assertRaisesRegex(RuntimeError, "authentication required"):
                run_conformance(args, progress)
        self.assertEqual(worker.call_count, 1)
        self.assertEqual(worker.call_args.args[0].version, "1.0.0")
        self.assertEqual(progress["phase"], "previous_worker_startup")
        self.assertEqual(progress["candidate"]["plugin_version"], "2.0.0")
        self.assertEqual(progress["previous"]["plugin_version"], "1.0.0")
        self.assertEqual(progress["completed"], ["candidate_artifact_probes", "previous_artifact_probes"])

    def test_failed_candidate_startup_cleans_up_the_ready_previous_worker(self):
        args = SimpleNamespace(release=Path("new.release.json"), artifacts=Path("new-artifacts"),
                               previous=Path("old.release.json"), previous_artifacts=Path("old-artifacts"),
                               worker=Path("/fixture-worker"))
        def candidate(path, _artifacts, _root):
            old = path.name.startswith("old")
            return SimpleNamespace(id="fixture", version="1.0.0" if old else "2.0.0",
                                   release={"artifact_digest": "sha256:" + ("a" if old else "b") * 64},
                                   target={"os": "linux", "architecture": "x86_64"}, probes=[])
        previous = Mock()
        progress = {"phase": "input_validation", "completed": []}
        with patch("plugin_runtime_conformance.Candidate", side_effect=candidate), \
             patch("plugin_runtime_conformance.Worker", side_effect=[previous, RuntimeError("candidate failed")]):
            with self.assertRaisesRegex(RuntimeError, "candidate failed"):
                run_conformance(args, progress)
        previous.cleanup.assert_called_once()
        self.assertEqual(progress["phase"], "candidate_worker_startup")
        self.assertIn("previous_worker_ready", progress["completed"])
        self.assertNotIn("candidate_worker_ready", progress["completed"])
        self.assertNotIn("distinct_generation_coexistence", progress["completed"])

    def test_cgroup_identity_is_unique_across_plugins_runs_and_generations(self):
        identities = {fixture_session_id(plugin, Path("/tmp") / run / generation)
                      for plugin in ["codex", "grok"]
                      for run in ["cw-conformance-first", "cw-conformance-second"]
                      for generation in ["old", "new"]}
        self.assertEqual(len(identities), 8)

    def test_worker_requires_non_root_isolated_network(self):
        with patch("platform.system", return_value="Linux"), \
             patch("os.geteuid", return_value=1000), \
             patch("socket.if_nameindex", return_value=[(1, "lo")]):
            require_worker_isolation()
            with patch("os.geteuid", return_value=0):
                with self.assertRaisesRegex(RuntimeError, "non-root"):
                    require_worker_isolation()
            with patch("socket.if_nameindex", return_value=[(1, "lo"), (2, "eth0")]):
                with self.assertRaisesRegex(RuntimeError, "loopback-only"):
                    require_worker_isolation()

    def test_environment_is_closed_and_does_not_mutate_parent(self):
        with tempfile.TemporaryDirectory() as temporary:
            before = dict(os.environ)
            environment = closed_environment(Path(temporary) / "home")
            self.assertEqual(environment["HOME"], temporary + "/home")
            self.assertEqual(environment["CODEX_HOME"], temporary + "/home/codex")
            self.assertEqual(set(environment) - {"PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE"},
                             {"HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "TMPDIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR"})
            self.assertEqual(dict(os.environ), before)

    def test_framed_protocol_is_bounded_and_round_trips(self):
        first, second = socket.socketpair()
        with first, second:
            frame = {"type": "welcome", "protocol": 1, "controller_epoch": 1}
            write_frame(first, frame)
            self.assertEqual(read_frame(second), frame)
            first.sendall(struct.pack(">I", 4 * 1024**2 + 1))
            with self.assertRaisesRegex(RuntimeError, "frame size"):
                read_frame(second)

    def test_archive_rejects_traversal_links_and_duplicate_files(self):
        for name, kind, duplicate in [("../escape", tarfile.REGTYPE, False),
                                      ("/escape", tarfile.REGTYPE, False),
                                      ("link", tarfile.SYMTYPE, False),
                                      ("link", tarfile.LNKTYPE, False),
                                      ("bin/cli", tarfile.REGTYPE, True)]:
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive_path = root / "artifact.tar.gz"
                with tarfile.open(archive_path, "w:gz") as archive:
                    entry = tarfile.TarInfo(name)
                    entry.type = kind
                    entry.linkname = "outside"
                    archive.addfile(entry, io.BytesIO())
                    if duplicate:
                        archive.addfile(entry, io.BytesIO())
                destination = root / "extracted"
                destination.mkdir()
                with self.assertRaises((RuntimeError, FileExistsError)):
                    extract_archive(archive_path, destination)

    def test_archive_preserves_executable_and_digest_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "artifact.tar.gz"
            with tarfile.open(archive_path, "w:gz") as archive:
                entry = tarfile.TarInfo("bin/cli")
                entry.size = 3
                entry.mode = 0o755
                archive.addfile(entry, io.BytesIO(b"cli"))
            destination = root / "extracted"
            destination.mkdir()
            extract_archive(archive_path, destination)
            command = destination / "bin/cli"
            self.assertEqual(command.read_bytes(), b"cli")
            self.assertTrue(os.access(command, os.X_OK))
            self.assertTrue(digest(command).startswith("sha256:"))
            link = root / "link"
            link.symlink_to(command)
            with self.assertRaisesRegex(RuntimeError, "regular file"):
                digest(link)


if __name__ == "__main__":
    unittest.main()
