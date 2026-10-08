import json
from pathlib import Path
import re
import subprocess
import unittest

from claude_native_behavior_probe import stable
from execution_claude_worker_conformance import PHASES
from remote_impact import imports, impact

ROOT = Path(__file__).resolve().parent.parent
CHECK_MAP = json.loads((ROOT / "tools/remote_check_map.json").read_text())
CLAUDE = "plugins/claude-code/runtime/"


def suites(*args, **kwargs):
    return impact(*args, check_map=CHECK_MAP, **kwargs)["suites"]


class ImpactTest(unittest.TestCase):
    def test_version_bumps_select_nothing(self):
        changed = ["plugins/claude-code/provider.json", "plugins/claude-code/plugin.json",
                   "plugins/codex/provider.json", "plugins/codex/plugin.json"]
        self.assertEqual(impact(changed, CHECK_MAP), {"native_changed": [], "suites": {}, "unmapped": []})

    def test_a_claude_native_bump_selects_only_claude_suites(self):
        selected = suites(["plugins/claude-code/provider.json"], native_changed=["claude"])
        self.assertEqual(set(selected), {"claude-remote-check", "claude-worker", "claude-task-stop"})
        self.assertEqual(selected["claude-worker"]["phases"], [])
        self.assertEqual(selected["claude-worker"]["native_probes"],
                         sorted(CHECK_MAP["suites"]["claude-worker"]["native_probes"]))

    def test_a_codex_native_bump_selects_only_codex_suites(self):
        selected = suites(["plugins/codex/provider.json"], native_changed=["codex"])
        self.assertEqual(set(selected), {"codex-adapter-check", "codex-worker", "codex-turn", "child-stop",
                                         "codex-hooks"})

    def test_an_executor_bump_runs_every_executor_suite_in_full(self):
        selected = suites(["components/execution-runtime/lock.json"])
        self.assertEqual(set(selected), {"claude-worker", "codex-worker", "session", "keeper", "claude-task-stop",
                                         "lifetime", "codex-turn", "child-stop", "codex-hooks", "handshake-recovery"})
        self.assertEqual(selected["claude-worker"]["phases"], "all")

    def test_a_claude_feature_module_runs_its_phase(self):
        selected = suites([CLAUDE + "skills.mjs"])
        self.assertEqual(set(selected), {"claude-remote-check", "claude-worker"})
        self.assertEqual(selected["claude-worker"]["phases"], ["skills"])

    def test_a_claude_core_module_runs_every_phase(self):
        selected = suites([CLAUDE + "tools.mjs"])
        self.assertEqual(set(selected), {"claude-remote-check", "claude-worker"})
        self.assertEqual(selected["claude-worker"]["phases"], "all")

    def test_a_codex_launcher_change_stays_in_its_lane(self):
        self.assertEqual(set(suites(["plugins/codex/runtime/launch.mjs"])), {"codex-adapter-check", "codex-worker"})

    def test_the_execution_transport_runs_every_suite_built_on_it(self):
        selected = suites(["src/execution_host/ledger.rs"])
        self.assertEqual(set(selected), {"execution-rust-tests", "claude-worker", "codex-worker", "session",
                                         "keeper", "claude-task-stop"})
        self.assertEqual(selected["claude-worker"]["phases"], "all")

    def test_machine_control_runs_its_session_gate(self):
        self.assertEqual(set(suites(["src/machine_broker.rs"])), {"execution-rust-tests", "session"})

    def test_the_execution_session_api_runs_its_session_gate(self):
        self.assertEqual(set(suites(["src/server/execution/sessions.rs"])), {"execution-rust-tests", "session"})

    def test_the_shared_memory_client_runs_both_worker_suites(self):
        selected = suites(["components/memory-client/index.mjs"])
        self.assertEqual(set(selected), {"claude-remote-check", "claude-worker", "codex-worker"})

    def test_the_keeper_binary_runs_every_keeper_suite(self):
        self.assertEqual(set(suites(["src/bin/cowboy-execution-host.rs"])), {
            "execution-rust-tests", "claude-worker", "codex-worker", "session", "keeper", "claude-task-stop"})

    def test_a_shared_harness_module_runs_every_suite_importing_it(self):
        shared = "tools/execution_environment_probe.py"
        expected = {name for name, spec in CHECK_MAP["suites"].items() if "entry" in spec and
                    shared in imports(spec["entry"])}
        self.assertIn("codex-turn", expected)
        self.assertEqual(set(suites([shared])), expected)

    def test_a_changed_baseline_reruns_its_probe_and_phase(self):
        selected = suites(["tools/claude_skill_native_baseline.json"])
        self.assertEqual(set(selected), {"claude-worker"})
        self.assertEqual(selected["claude-worker"]["native_probes"], ["tools/claude_skill_native_probe.py"])
        self.assertEqual(selected["claude-worker"]["phases"], ["skills"])

    def test_an_unknown_remote_file_runs_everything(self):
        result = impact(["src/execution_new.rs"], CHECK_MAP)
        self.assertEqual(result["unmapped"], ["src/execution_new.rs"])
        self.assertEqual(set(result["suites"]), set(CHECK_MAP["suites"]))
        self.assertEqual(result["suites"]["claude-worker"]["phases"], "all")

    def test_files_outside_remote_execution_select_nothing(self):
        changed = ["docs/remote-tools-coverage-audit-2026-10-05.md", "src/config.rs", "web/src/App.tsx"]
        self.assertEqual(impact(changed, CHECK_MAP), {"native_changed": [], "suites": {}, "unmapped": []})


class CheckMapTest(unittest.TestCase):
    def test_claude_phases_match_the_conformance_harness(self):
        claude = CHECK_MAP["suites"]["claude-worker"]
        self.assertEqual(set(claude["phases"]), set(PHASES))
        for spec in claude["native_probes"].values():
            self.assertLessEqual(set(spec["phases"]), set(PHASES))

    def test_every_named_file_exists(self):
        named = list(CHECK_MAP["manifests"]) + list(CHECK_MAP["inert"])
        for spec in CHECK_MAP["native"].values():
            named += spec.get("inputs", []) + ([spec["manifest"]] if "manifest" in spec else [])
        for spec in CHECK_MAP["suites"].values():
            named += spec.get("paths", []) + spec.get("manifests", []) + ([spec["entry"]] if "entry" in spec else [])
            named += [entry for entries in spec.get("phases", {}).values() for entry in entries]
            for probe, probe_spec in spec.get("native_probes", {}).items():
                named += [probe, probe_spec["baseline"]]
        for entry in named:
            self.assertTrue((ROOT / entry).exists() or list(ROOT.glob(entry + "*")), entry)

    def test_every_command_names_an_existing_recipe(self):
        recipes = set(re.findall(r"^([a-z][a-z0-9-]*)[^:\n]*:", (ROOT / "justfile").read_text(), re.M))
        for name, spec in CHECK_MAP["suites"].items():
            command = spec["command"]
            if command.startswith("just "):
                self.assertIn(command.split()[1], recipes, name)

    def test_every_remote_file_is_mapped(self):
        files = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard"], cwd=ROOT,
                               check=True, capture_output=True, text=True).stdout.split()
        files = [path for path in files if (ROOT / path).exists()]
        self.assertEqual(impact(files, CHECK_MAP)["unmapped"], [])


class StableReceiptTest(unittest.TestCase):
    def test_receipts_of_two_runs_compare_equal(self):
        def run(root, session, first, second, use):
            return stable(f"(ID: {first}) {root}/home/-{root[1:].replace('/', '-')}-project/{session}/tasks/"
                          f"{first}.output <task-id>{second}</task-id> {use}", root)
        one = run("/tmp/cowboy-native-bash-ab12", "ce41a9d4-6659-416a-bd43-6489e606117b", "bq9loql5h", "b7wnwlqif",
                  "toolu_c8b6")
        two = run("/tmp/cowboy-native-bash-zz99", "0f0f0f0f-1111-2222-3333-444444444444", "bxxxxxxx1", "byyyyyyy2",
                  "toolu_ffff")
        self.assertEqual(one, two)
        self.assertEqual(one, "(ID: task1) <ROOT>/home/-ROOT-project/SESSION/tasks/task1.output "
                              "<task-id>task2</task-id> toolu_1")


if __name__ == "__main__":
    unittest.main()
