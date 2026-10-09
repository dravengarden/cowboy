#!/usr/bin/env python3
"""Which remote execution checks a change needs, from its Git diff.

Reads tools/remote_check_map.json and prints JSON with:
- "native_changed": native sets (pinned CLIs and adapters, executor, Node) whose
  pins changed; a pin is a JSON value in a manifest or lock file, compared
  between the two revisions, so other components in a shared lock do not count;
- "suites": each selected suite with its command, kind (unit, packaged, native)
  and why it was selected. "claude-worker" also carries "phases" for the
  conformance input's "phases" field ("all" omits the field; [] runs the base
  turn alone) and "native_probes" to re-run: diff each fresh receipt with its
  baseline and add the phases the map lists for any probe that differs. A
  native change also runs every phase outside "native_covered";
- "unmapped": changed remote files no packaged or native suite, pin, manifest
  or inert entry names (a unit gate's directory does not count). They select
  every suite until the map is extended in the same change.

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


def impact(changed, check_map, native_changed=(), manifests_changed=(), closure=imports, deleted=()):
    native_changed = set(native_changed)
    for name, spec in check_map["native"].items():
        if any(path in spec.get("inputs", []) for path in changed):
            native_changed.add(name)
    suites = check_map["suites"]
    known = set(check_map["manifests"]) | set(check_map["inert"])
    for spec in check_map["native"].values():
        known |= set(spec.get("inputs", [])) | {pin["file"] for pin in spec.get("pins", [])}
    for spec in suites.values():
        known |= set(spec.get("paths", [])) if spec["kind"] != "unit" else set()
        known |= closure(spec["entry"]) if "entry" in spec else set()
        for entries in spec.get("phases", {}).values():
            known |= set(entries)
        for probe, probe_spec in spec.get("native_probes", {}).items():
            known |= {probe, probe_spec["baseline"]}
    # A deleted file the map no longer names needs no mapping; one it still
    # names selects its suites as any change does.
    unmapped = sorted(path for path in changed if path not in deleted and matches(path, check_map["lanes"])
                      and not matches(path, known)
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
                # Probes measure only the "native_covered" phases; the others
                # rely on native behavior no probe captures, so they run.
                probes = list(spec["native_probes"])
                phases.update(set(spec["phases"]) - set(spec["native_covered"]))
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


def value_at(document, keys):
    for key in keys:
        document = document.get(key, {}) if isinstance(document, dict) else {}
    return document


def pinned_changes(changed, check_map, before, after):
    """Native sets whose pins differ, and manifests whose non-inert, unpinned keys differ.

    before/after load a file's JSON at the base and head revisions."""
    native = sorted(name for name, spec in check_map["native"].items() if any(
        pin["file"] in changed and value_at(before(pin["file"]), pin["path"]) != value_at(after(pin["file"]), pin["path"])
        for pin in spec.get("pins", [])))

    def relevant(load, path):
        value = {key: json.loads(json.dumps(item)) for key, item in load(path).items()
                 if key not in check_map["manifests"][path]["inert"]}
        for spec in check_map["native"].values():
            for pin in spec.get("pins", []):
                if pin["file"] == path:
                    parent = value_at(value, pin["path"][:-1])
                    if isinstance(parent, dict):
                        parent.pop(pin["path"][-1], None)
        return value
    manifests = sorted(path for path in check_map["manifests"] if path in changed and
                       relevant(before, path) != relevant(after, path))
    return native, manifests


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args()
    changed = subprocess.run(["git", "diff", "--name-only", f"{args.base}...{args.head}"], cwd=ROOT, check=True,
                             capture_output=True, text=True).stdout.split()
    check_map = json.loads(CHECK_MAP.read_text())
    deleted = set(subprocess.run(["git", "diff", "--name-only", "--diff-filter=D", f"{args.base}...{args.head}"],
                                 cwd=ROOT, check=True, capture_output=True, text=True).stdout.split())

    def loader(rev):
        def load(path):
            shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=ROOT, capture_output=True, text=True)
            return json.loads(shown.stdout) if shown.returncode == 0 else {}
        return load
    native, manifests = pinned_changes(changed, check_map, loader(args.base), loader(args.head))
    print(json.dumps({"changed": changed, "manifests_changed": manifests,
                      **impact(changed, check_map, native, manifests, deleted=deleted)}, indent=1))


if __name__ == "__main__":
    main()
