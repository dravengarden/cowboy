import json
from pathlib import Path
import re
import subprocess
import unittest

from claude_native_behavior_probe import stable
from execution_claude_worker_conformance import PHASES
from remote_impact import imports, impact, pinned_changes

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
        claude = CHECK_MAP["suites"]["claude-worker"]
        self.assertEqual(selected["claude-worker"]["phases"],
                         sorted(set(claude["phases"]) - set(claude["native_covered"])))
        self.assertIn("hooks", selected["claude-worker"]["phases"])
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

    def test_the_codex_acp_launcher_runs_its_handshake_probe(self):
        selected = suites(["components/provider-runtime/packages/codex-acp/launch.mjs"])
        self.assertEqual(set(selected), {"codex-adapter-check", "codex-worker", "handshake-recovery"})
        self.assertIn("handshake-recovery", suites(["components/provider-runtime/lock.json"], native_changed=["node"]))

    def test_a_codex_runtime_manifest_change_runs_the_session_fixture_built_from_it(self):
        selected = suites(["plugins/codex/provider.json"], manifests_changed=["plugins/codex/provider.json"])
        self.assertEqual(set(selected), {"codex-adapter-check", "codex-worker", "session"})

    def test_a_new_runtime_module_is_unmapped_even_under_a_unit_gate_directory(self):
        for path in [CLAUDE + "new-module.mjs", "plugins/codex/runtime/new-module.mjs"]:
            result = impact([path], CHECK_MAP)
            self.assertEqual(result["unmapped"], [path])
            self.assertEqual(set(result["suites"]), set(CHECK_MAP["suites"]))

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

    def test_a_deleted_file_the_map_no_longer_names_is_not_unmapped(self):
        gone = "tools/claude_remote_impact.py"
        self.assertEqual(impact([gone], CHECK_MAP, deleted={gone}), {"native_changed": [], "suites": {}, "unmapped": []})
        self.assertEqual(impact([gone], CHECK_MAP)["unmapped"], [gone])
        # One the map still names selects its suites.
        removed = "plugins/claude-code/runtime/skills.mjs"
        self.assertEqual(impact([removed], CHECK_MAP, deleted={removed})["suites"]["claude-worker"]["phases"],
                         ["skills"])

    def test_files_outside_remote_execution_select_nothing(self):
        changed = ["docs/remote-tools-coverage-audit-2026-10-05.md", "src/config.rs", "web/src/App.tsx"]
        self.assertEqual(impact(changed, CHECK_MAP), {"native_changed": [], "suites": {}, "unmapped": []})


class PinnedChangesTest(unittest.TestCase):
    LOCK = "components/provider-runtime/lock.json"

    def compare(self, path, before, after):
        return pinned_changes([path], CHECK_MAP, lambda _: before, lambda _: after)

    def test_each_lock_component_marks_its_own_native_set(self):
        base = {"node": {"version": "1"}, "components": {"anthropic-claude-code": {"url": "a"},
                "openai-codex": {"url": "a"}, "google-gemini-cli": {"url": "a"}}}

        def changed(*keys):
            after = json.loads(json.dumps(base))
            target = after
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = {"url": "b"}
            return self.compare(self.LOCK, base, after)[0]
        self.assertEqual(changed("components", "anthropic-claude-code"), ["claude"])
        self.assertEqual(changed("components", "openai-codex"), ["codex"])
        self.assertEqual(changed("node"), ["node"])
        self.assertEqual(changed("components", "google-gemini-cli"), [])

    def test_manifest_pins_are_native_and_inert_keys_are_ignored(self):
        path = "plugins/codex/provider.json"
        base = {"version": "1", "runtime": {"dependencies": [1], "entrypoint": "a"}}
        self.assertEqual(self.compare(path, base, {"version": "2", "runtime": {"dependencies": [1], "entrypoint": "a"}}),
                         ([], []))
        self.assertEqual(self.compare(path, base, {"version": "1", "runtime": {"dependencies": [2], "entrypoint": "a"}}),
                         (["codex"], []))
        self.assertEqual(self.compare(path, base, {"version": "1", "runtime": {"dependencies": [1], "entrypoint": "b"}}),
                         ([], [path]))


class CheckMapTest(unittest.TestCase):
    def test_claude_phases_match_the_conformance_harness(self):
        claude = CHECK_MAP["suites"]["claude-worker"]
        self.assertEqual(set(claude["phases"]), set(PHASES))
        for spec in claude["native_probes"].values():
            self.assertLessEqual(set(spec["phases"]), set(PHASES))
        # A phase counts as covered only when some probe measures it.
        measured = {phase for spec in claude["native_probes"].values() for phase in spec["phases"]}
        self.assertLessEqual(set(claude["native_covered"]), measured)

    def test_every_named_file_exists(self):
        named = list(CHECK_MAP["manifests"]) + list(CHECK_MAP["inert"])
        for spec in CHECK_MAP["native"].values():
            named += spec.get("inputs", []) + [pin["file"] for pin in spec.get("pins", [])]
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
