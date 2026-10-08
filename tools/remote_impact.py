#!/usr/bin/env python3
"""Which remote execution checks a change needs, from its Git diff.

Reads tools/remote_check_map.json and prints JSON with:
- "native_changed": native sets (pinned CLIs, executor, Node) whose pins changed;
- "suites": each selected suite with its command, kind (unit, packaged, native)
  and why it was selected. "claude-worker" also carries "phases" for the
  conformance input's "phases" field ("all" omits the field; [] runs the base
  turn alone) and "native_probes" to re-run: diff each fresh receipt with its
  baseline and add the phases the map lists for any probe that differs;
- "unmapped": changed remote files the map does not name. They select every
  suite until the map is extended in the same change.

A manifest under "manifests" counts as changed only when a key outside its
"inert" list changed; a native set's pinned dependencies count as a native
change instead. Suites that are not selected keep their accepted receipts.

Usage: python3 tools/remote_impact.py --base <accepted revision> [--head <git rev>]
"""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
CHECK_MAP = ROOT / "tools/remote_check_map.json"


def matches(path, entries):
    return any(path == entry or path.startswith(entry) for entry in entries)


def imports(entry, tools=ROOT / "tools"):
    """The tools/ modules a Python entry imports, transitively, including itself."""
    seen, pending = set(), [entry]
    while pending:
        path = pending.pop()
        if path in seen or not (ROOT / path).is_file():
            continue
        seen.add(path)
        if path.endswith(".py"):
            for name in re.findall(r"^\s*(?:from|import) (\w+)", (ROOT / path).read_text(), re.M):
                if (tools / f"{name}.py").is_file():
                    pending.append(f"tools/{name}.py")
    return seen


def impact(changed, check_map, native_changed=(), manifests_changed=(), closure=imports):
    native_changed = set(native_changed)
    for name, spec in check_map["native"].items():
        if any(path in spec.get("inputs", []) for path in changed):
            native_changed.add(name)
    suites = check_map["suites"]
    known = set(check_map["manifests"]) | set(check_map["inert"])
    for spec in check_map["native"].values():
        known |= set(spec.get("inputs", []))
    for spec in suites.values():
        known |= set(spec.get("paths", []))
        known |= closure(spec["entry"]) if "entry" in spec else set()
        for entries in spec.get("phases", {}).values():
            known |= set(entries)
        for probe, probe_spec in spec.get("native_probes", {}).items():
            known |= {probe, probe_spec["baseline"]}
    unmapped = sorted(path for path in changed if matches(path, check_map["lanes"]) and not matches(path, known)
                      and not path.endswith((".test.mjs", "_test.py", "_test.mjs", ".md")))
    selected = {}
    for name, spec in suites.items():
        reasons = []
        harness = closure(spec["entry"]) if "entry" in spec else set()
        hits = sorted(path for path in changed if matches(path, spec.get("paths", [])) or path in harness)
        if hits:
            reasons.append({"files": hits})
        native = sorted(native_changed & set(spec.get("native", [])))
        if native:
            reasons.append({"native": native})
        manifests = sorted(set(manifests_changed) & set(spec.get("manifests", [])))
        if manifests:
            reasons.append({"manifests": manifests})
        if unmapped:
            reasons.append({"unmapped": unmapped})
        phase_reasons = []
        if "phases" in spec:
            phases, probes = set(), []
            for phase, entries in spec["phases"].items():
                if any(matches(path, entries) for path in changed):
                    phases.add(phase)
            for probe, probe_spec in spec["native_probes"].items():
                if any(path in (probe, probe_spec["baseline"]) for path in changed):
                    probes.append(probe)
                    phases.update(probe_spec["phases"])
            if "claude" in native:
                probes = list(spec["native_probes"])
            if phases or probes:
                phase_reasons.append({"phases": sorted(phases)})
            full = bool(hits and any(not any(matches(path, entries) for entries in spec["phases"].values()) and
                                     path not in spec["native_probes"] and
                                     path not in {item["baseline"] for item in spec["native_probes"].values()}
                                     for path in hits)) or bool(manifests) or bool(unmapped) or \
                bool(set(native) - {"claude"})
        if not reasons and not phase_reasons:
            continue
        entry = {"kind": spec["kind"], "command": spec["command"], "because": reasons + phase_reasons}
        if "phases" in spec:
            entry["phases"] = "all" if full else sorted(phases)
            entry["native_probes"] = sorted(set(probes))
        selected[name] = entry
    return {"native_changed": sorted(native_changed), "suites": selected, "unmapped": unmapped}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    changed = subprocess.run(["git", "diff", "--name-only", f"{args.base}...{args.head}"], cwd=ROOT, check=True,
                             capture_output=True, text=True).stdout.split()
    check_map = json.loads(CHECK_MAP.read_text())

    def manifest(rev, path):
        shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=ROOT, capture_output=True, text=True)
        return json.loads(shown.stdout) if shown.returncode == 0 else {}

    def pinned(rev, spec):
        value = manifest(rev, spec["manifest"])
        for key in spec["path"]:
            value = value.get(key, {}) if isinstance(value, dict) else {}
        return value

    def relevant(rev, path):
        value = {key: item for key, item in manifest(rev, path).items()
                 if key not in check_map["manifests"][path]["inert"]}
        for spec in check_map["native"].values():
            if spec.get("manifest") == path:
                parent = value
                for key in spec["path"][:-1]:
                    parent = parent.get(key, {}) if isinstance(parent, dict) else {}
                if isinstance(parent, dict):
                    parent.pop(spec["path"][-1], None)
        return value
    native = [name for name, spec in check_map["native"].items() if "manifest" in spec and
              spec["manifest"] in changed and pinned(args.base, spec) != pinned(args.head, spec)]
    manifests = [path for path in check_map["manifests"] if path in changed and
                 relevant(args.base, path) != relevant(args.head, path)]
    print(json.dumps({"changed": changed, "manifests_changed": manifests,
                      **impact(changed, check_map, native, manifests)}, indent=1))


if __name__ == "__main__":
    main()
