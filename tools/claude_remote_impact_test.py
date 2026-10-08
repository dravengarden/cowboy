import json
from pathlib import Path
import subprocess
import unittest

from claude_native_behavior_probe import stable
from claude_remote_impact import impact
from execution_claude_worker_conformance import PHASES

ROOT = Path(__file__).resolve().parent.parent
CHECK_MAP = json.loads((ROOT / "tools/claude_remote_check_map.json").read_text())
RUNTIME = "plugins/claude-code/runtime/"


class ImpactTest(unittest.TestCase):
    def test_a_version_bump_needs_no_phase(self):
        result = impact(["plugins/claude-code/provider.json", "plugins/claude-code/plugin.json"], CHECK_MAP)
        self.assertEqual(result, {"native_probes": [], "phases": [], "unmapped": []})

    def test_a_native_bump_reruns_every_probe_but_no_phase(self):
        result = impact(["plugins/claude-code/provider.json"], CHECK_MAP, native_dependencies_changed=True)
        self.assertEqual(result["native_probes"], sorted(CHECK_MAP["native_probes"]))
        self.assertEqual(result["phases"], [])

    def test_a_changed_runtime_manifest_runs_everything(self):
        result = impact(["plugins/claude-code/provider.json"], CHECK_MAP, manifests_changed=True)
        self.assertEqual(result["phases"], "all")

    def test_a_feature_module_runs_its_phase(self):
        self.assertEqual(impact([RUNTIME + "skills.mjs"], CHECK_MAP)["phases"], ["skills"])
        self.assertEqual(impact([RUNTIME + "mcp-proxy.mjs"], CHECK_MAP)["phases"], ["mcp"])

    def test_a_core_module_runs_everything(self):
        self.assertEqual(impact([RUNTIME + "tools.mjs"], CHECK_MAP)["phases"], "all")

    def test_a_changed_baseline_reruns_its_probe_and_phases(self):
        result = impact(["tools/claude_skill_native_baseline.json"], CHECK_MAP)
        self.assertEqual(result["native_probes"], ["tools/claude_skill_native_probe.py"])
        self.assertEqual(result["phases"], ["skills"])

    def test_an_unknown_lane_file_runs_everything(self):
        result = impact([RUNTIME + "new-feature.mjs"], CHECK_MAP)
        self.assertEqual(result["phases"], "all")
        self.assertEqual(result["unmapped"], [RUNTIME + "new-feature.mjs"])

    def test_files_outside_the_lane_need_nothing(self):
        result = impact(["docs/remote-tools-coverage-audit-2026-10-05.md", "src/config.rs"], CHECK_MAP)
        self.assertEqual(result, {"native_probes": [], "phases": [], "unmapped": []})


class CheckMapTest(unittest.TestCase):
    def test_phases_match_the_conformance_harness(self):
        self.assertEqual(set(CHECK_MAP["phases"]), set(PHASES))
        for spec in CHECK_MAP["native_probes"].values():
            self.assertLessEqual(set(spec["phases"]), set(PHASES))

    def test_every_named_file_exists(self):
        named = list(CHECK_MAP["core"]) + list(CHECK_MAP["native_inputs"]) + list(CHECK_MAP["manifests"]) + \
            list(CHECK_MAP["inert"]) + [entry for entries in CHECK_MAP["phases"].values() for entry in entries]
        for probe, spec in CHECK_MAP["native_probes"].items():
            named += [probe, spec["baseline"]]
        for entry in named:
            self.assertTrue((ROOT / entry).exists() or list(ROOT.glob(entry + "*")), entry)

    def test_every_tracked_lane_file_is_mapped(self):
        tracked = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard",
                                  "plugins/claude-code", "tools"], cwd=ROOT, check=True,
                                 capture_output=True, text=True).stdout.split()
        tracked = [path for path in tracked if (ROOT / path).exists()]
        self.assertEqual(impact(tracked, CHECK_MAP)["unmapped"], [])


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
