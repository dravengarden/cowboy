import subprocess
import unittest

import ovh_machine_idle_sessions as idle


def row(id, status, machine="ovh", account="default", system=False):
    return {"id": id, "status": status, "machine_id": machine,
            "provider_account_id": account, "system": system}


class IdleSessionsTest(unittest.TestCase):
    def test_selects_every_idle_session_of_the_machine_including_named_accounts(self):
        rows = [
            row("s-default", "running"),
            row("s-named", "running", account="0b5e2b8e-4c5f-4d0e-9d1b-2f6c1d2a3b4c"),
            row("s-busy", "busy"),
            row("s-crashed", "crashed"),
            row("s-dormant", "exited"),
            row("s-hawk", "running", machine="hawk"),
            row("s-system", "running", system=True),
        ]
        self.assertEqual(idle.idle_sessions(rows, "ovh"), ["s-default", "s-named"])

    def test_refused_sessions_stay_retained_and_stops_are_awaited(self):
        state = {"s-a": "running", "s-b": "running", "s-busy": "busy"}
        calls = []
        polls = []

        def run(*args):
            calls.append(args)
            session = args[-1]
            if session == "s-b":
                return subprocess.CompletedProcess(args, 1, "", "http 409: wait for the current turn")
            return subprocess.CompletedProcess(args, 0, f"{session}\tok", "")

        def sessions():
            polls.append(dict(state))
            rows = [row(id, status) for id, status in state.items()]
            # The worker stop is observed one poll after hibernation.
            if calls and state["s-a"] == "running" and len(polls) > 2:
                state["s-a"] = "exited"
            return rows

        summary = idle.hibernate_idle("ovh", run=run, sessions=sessions, sleep=lambda _: None)
        self.assertEqual(calls, [("session", "hibernate", "s-a"), ("session", "hibernate", "s-b")])
        self.assertEqual(summary["hibernated"], ["s-a"])
        self.assertEqual(summary["retained"], [{"id": "s-b", "reason": "http 409: wait for the current turn"}])
        self.assertEqual(state["s-a"], "exited")

    def test_unobserved_stop_refuses_before_maintenance(self):
        def run(*args):
            return subprocess.CompletedProcess(args, 0, "", "")

        with self.assertRaisesRegex(RuntimeError, "did not stop in time: s-a"):
            idle.hibernate_idle("ovh", run=run, sessions=lambda: [row("s-a", "running")],
                                sleep=lambda _: None, settle_seconds=0)


if __name__ == "__main__":
    unittest.main()
