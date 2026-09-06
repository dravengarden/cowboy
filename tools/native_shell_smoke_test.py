import unittest
from unittest.mock import patch

from native_shell_smoke import REMOTE_ORIGIN, probe_script, valid_probe, wait_for_probe


def report(remote):
    return dict(tests=["check-" + str(i) for i in range(15 if remote else 7)],
                phase="remote-logged-out" if remote else "shell",
                origin=REMOTE_ORIGIN if remote else "tauri://localhost",
                user_agent="iPhone AppleWebKit")


class NativeSmokeTests(unittest.TestCase):
    def test_modes_have_distinct_receipt_contracts(self):
        self.assertTrue(valid_probe(report(False), False))
        self.assertTrue(valid_probe(report(True), True))
        self.assertFalse(valid_probe(report(False), True))
        self.assertFalse(valid_probe(report(True), False))
        remote = report(True)
        remote["origin"] = "tauri://localhost"
        self.assertFalse(valid_probe(remote, True))

    def test_malformed_or_duplicate_checks_fail_closed(self):
        for value in [None, [], "ok", {}, {**report(True), "tests": ["same"] * 15},
                      {**report(True), "tests": [{}] * 15},
                      {**report(True), "user_agent": None}]:
            with self.subTest(value=value):
                self.assertFalse(valid_probe(value, True))

    def test_remote_timeout_never_falls_back_to_a_local_success(self):
        import json
        with patch("native_shell_smoke.request", return_value=(200, json.dumps(report(False)))), \
             patch("native_shell_smoke.time.monotonic", side_effect=[0, 151]):
            with self.assertRaisesRegex(RuntimeError, "remote smoke failed"):
                wait_for_probe(12345, "exclusive-simulator", True)

    def test_bridge_error_is_a_failure_not_a_scope_receipt(self):
        with patch("native_shell_smoke.request", return_value=(200, "ERR: command not found")), \
             patch("native_shell_smoke.time.monotonic", side_effect=[0, 151]):
            with self.assertRaisesRegex(RuntimeError, "ERR: command not found"):
                wait_for_probe(12345, "exclusive-simulator", True)

    def test_remote_flag_is_passed_as_a_literal_not_a_navigation_override(self):
        self.assertTrue(probe_script(True).endswith("probeCowboyNativeShell(true));"))
        self.assertTrue(probe_script(False).endswith("probeCowboyNativeShell(false));"))
        self.assertNotIn("location.replace", probe_script(True))


if __name__ == "__main__":
    unittest.main()
