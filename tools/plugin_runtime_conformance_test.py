import io
import os
from pathlib import Path
import socket
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from plugin_runtime_conformance import (
    closed_environment, digest, extract_archive, fixture_session_id, read_frame,
    require_worker_isolation, write_frame,
)


class ConformanceHarnessTests(unittest.TestCase):
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
