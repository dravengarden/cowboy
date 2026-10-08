#!/usr/bin/env python3
"""Which remote Claude lane checks a change needs, from its Git diff.

Reads tools/claude_remote_check_map.json and prints JSON with:
- "native_probes": probes to re-run (when the pinned native CLI or ACP
  dependencies changed, or a probe or baseline did); compare each new
  receipt with its baseline, and re-run `just claude-remote-check`, whose
  baseline tests fail where the plugin's native assumptions no longer hold;
- "phases": packaged acceptance phases for the input JSON's "phases" field
  ("all" when a core file changed; [] means the base turn alone). After a
  native change, add the phases of each probe whose receipt differs from its
  baseline;
- "unmapped": changed lane files the map does not name (treat as core and
  extend the map).

A manifest under "manifests" counts as core only when a key outside its
"inert" list changed; its pinned native dependencies count as a native
change instead.

Usage: python3 tools/claude_remote_impact.py --base <git rev> [--head <git rev>]
"""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
LANE = ("plugins/claude-code/", "tools/claude", "tools/execution_claude", "components/provider-runtime/")


def impact(changed, check_map, native_dependencies_changed=False, manifests_changed=False):
    def matches(path, entries):
        return any(path == entry or (entry.endswith("/") and path.startswith(entry)) or
                   (not entry.endswith((".mjs", ".js", ".py", ".json", ".ts", ".rs")) and path.startswith(entry))
                   for entry in entries)
    native = native_dependencies_changed or any(matches(path, check_map["native_inputs"]) for path in changed)
    core = manifests_changed or any(matches(path, check_map["core"]) for path in changed)
    phases = set()
    for name, entries in check_map["phases"].items():
        if any(matches(path, entries) for path in changed):
            phases.add(name)
    probes = sorted(check_map["native_probes"]) if native else []
    for probe, spec in check_map["native_probes"].items():
        if any(path in (probe, spec["baseline"]) for path in changed):
            probes.append(probe)
            phases.update(spec["phases"])
    known = set(check_map["core"]) | {entry for entries in check_map["phases"].values() for entry in entries} | \
        set(check_map["native_probes"]) | {spec["baseline"] for spec in check_map["native_probes"].values()} | \
        set(check_map["native_inputs"]) | set(check_map["manifests"]) | set(check_map["inert"])
    unmapped = sorted(path for path in changed if path.startswith(LANE) and not matches(path, known) and
                      not path.endswith((".test.mjs", "_test.py", ".md")) and path != "tools/claude_remote_check_map.json")
    return {
        "native_probes": sorted(set(probes)),
        "phases": "all" if core or unmapped else sorted(phases),
        "unmapped": unmapped,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    changed = subprocess.run(["git", "diff", "--name-only", f"{args.base}...{args.head}"], cwd=ROOT, check=True,
                             capture_output=True, text=True).stdout.split()
    check_map = json.loads((ROOT / "tools/claude_remote_check_map.json").read_text())
    pinned = check_map["native_dependencies"]

    def manifest(rev, path):
        shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=ROOT, capture_output=True, text=True)
        return json.loads(shown.stdout) if shown.returncode == 0 else {}

    def dependencies(rev):
        value = manifest(rev, pinned["file"])
        for key in pinned["path"]:
            value = value.get(key, {}) if isinstance(value, dict) else {}
        return value

    def relevant(rev, path):
        value = {key: item for key, item in manifest(rev, path).items()
                 if key not in check_map["manifests"][path]["inert"]}
        if path == pinned["file"]:
            parent = value
            for key in pinned["path"][:-1]:
                parent = parent.get(key, {}) if isinstance(parent, dict) else {}
            if isinstance(parent, dict):
                parent.pop(pinned["path"][-1], None)
        return value
    native = dependencies(args.base) != dependencies(args.head)
    manifests = sorted(path for path in check_map["manifests"] if path in changed and
                       relevant(args.base, path) != relevant(args.head, path))
    print(json.dumps({"changed": changed, "native_dependencies_changed": native, "manifests_changed": manifests,
                      **impact(changed, check_map, native, bool(manifests))}, indent=1))


if __name__ == "__main__":
    main()
