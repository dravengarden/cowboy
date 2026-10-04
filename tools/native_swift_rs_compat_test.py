import importlib.util
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch

source = Path(__file__).resolve().parents[1] / "apps/native-shell/apple/swift-rs-compat.py"
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("swift_rs_compat", source)
compat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compat)


def nm(kind="t", member="SwiftRs.o"):
    return member + ":\n" + "".join(f"0000000000000000 {kind} {name}\n" for name in compat.SYMBOLS)


class SwiftRuntimeCompatibilityTests(unittest.TestCase):
    def test_archive_member_formats_and_non_runtime_symbols(self):
        for header in ["SwiftRs.o", "/tmp/libTauri.a(SwiftRs.o)", "/tmp/libTauri.a:SwiftRs.o"]:
            actual = compat.symbols(nm(member=header) + "0000000000000000 t ___swift_helper\n")
            self.assertEqual(actual, {name: [("SwiftRs.o", "t")] for name in compat.SYMBOLS})

    def test_missing_duplicate_and_foreign_runtime_definitions_fail_closed(self):
        for output in [nm().replace(compat.SYMBOLS[0], "_missing"), nm() + nm(), nm(member="Tauri.o")]:
            with self.assertRaisesRegex(RuntimeError, "ambiguous"):
                compat.verify(compat.symbols(output), True)
        with self.assertRaisesRegex(RuntimeError, "duplicate Swift runtime owner"):
            compat.verify(compat.symbols(nm("T")), False)

    def test_only_one_archive_changes_and_repeated_compilation_is_safe(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "target"
            root.mkdir()
            archives = [root / name for name in compat.ARCHIVES]
            for archive in archives:
                archive.write_bytes(b"original")
            arguments = ["-L", f"native={root}"]
            promoted = False

            def inspect(command, **kwargs):
                return nm("T" if promoted and command[-1] == str(archives[0]) else "t")

            def promote(command, **kwargs):
                nonlocal promoted
                self.assertEqual(command[1:-1], [f"--globalize-symbol={name}" for name in compat.SYMBOLS])
                self.assertEqual(command[-1], str(archives[0]))
                archives[0].write_bytes(b"promoted")
                promoted = True

            with patch.object(compat.subprocess, "check_output", side_effect=inspect), \
                 patch.object(compat.subprocess, "run", side_effect=promote) as run:
                compat.repair(arguments, root, Path("objcopy"))
                compat.repair(arguments, root, Path("objcopy"))
                self.assertEqual(run.call_count, 1)
            self.assertEqual([p.read_bytes() for p in archives], [b"promoted", b"original", b"original"])

    def test_borrowed_archive_and_incomplete_inventory_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "target"
            root.mkdir()
            foreign = base / "foreign"
            foreign.mkdir()
            (foreign / "libTauri.a").write_bytes(b"foreign")
            with self.assertRaisesRegex(RuntimeError, "escapes this build"):
                compat.repair(["-L", f"native={foreign}"], root, Path("objcopy"))
            with self.assertRaisesRegex(RuntimeError, "incomplete"):
                compat.repair(["-L", f"native={root}"], root, Path("objcopy"))


if __name__ == "__main__":
    unittest.main()
