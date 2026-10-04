#!/usr/bin/env python3
"""Cargo workspace wrapper for swift-rs 1.0.8's Xcode 27 runtime exports.

Registry sources stay untouched. Before compiling Cowboy's native library,
export the three C ABI functions from exactly one build-local Swift archive.
Remove this adapter when swift-rs exports one shared runtime upstream.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

SYMBOLS = ("_retain_object", "_release_object", "_string_from_bytes")
ARCHIVES = ("libTauri.a", "libtauri-plugin-haptics.a", "libtauri-plugin-opener.a")


def symbols(output):
    result = {name: [] for name in SYMBOLS}
    member = ""
    for line in output.splitlines():
        if line.rstrip().endswith(":"):
            member = line.rstrip().rstrip(":").rstrip(")").rsplit("(", 1)[-1]
            member = member.rsplit(":", 1)[-1]
        fields = line.split()
        if len(fields) == 3 and fields[2] in result and fields[1] in ("t", "T"):
            result[fields[2]].append((member, fields[1]))
    return result


def native_paths(arguments):
    paths = []
    for index, arg in enumerate(arguments):
        value = arguments[index + 1] if arg == "-L" else arg[2:] if arg.startswith("-L") else ""
        if value.startswith("native="):
            paths.append(Path(value[7:]).resolve())
    return paths


def verify(exports, owner):
    for name in SYMBOLS:
        entries = exports[name]
        if owner:
            if len(entries) != 1 or entries[0][0] != "SwiftRs.o":
                raise RuntimeError(f"ambiguous Swift runtime export {name}: {entries}")
        elif any(kind == "T" for _, kind in entries):
            raise RuntimeError(f"duplicate Swift runtime owner for {name}")


def repair(arguments, target_root, objcopy):
    archives = {}
    for directory in native_paths(arguments):
        for name in ARCHIVES:
            archive = directory / name
            if not archive.is_file():
                continue
            if not archive.resolve().is_relative_to(target_root.resolve()):
                raise RuntimeError(f"Swift archive escapes this build: {archive}")
            if name in archives and archives[name] != archive:
                raise RuntimeError(f"multiple Swift archives named {name}")
            archives[name] = archive
    if set(archives) != set(ARCHIVES):
        raise RuntimeError(f"incomplete Swift archive inventory: {sorted(archives)}")

    def inspect(archive):
        return symbols(subprocess.check_output(["xcrun", "nm", str(archive)], text=True))

    for name, archive in archives.items():
        verify(inspect(archive), name == "libTauri.a")
    owner = archives["libTauri.a"]
    before = hashlib.sha256(owner.read_bytes()).hexdigest()
    local = [name for name, entries in inspect(owner).items() if entries[0][1] == "t"]
    if local:
        subprocess.run([str(objcopy), *[f"--globalize-symbol={name}" for name in local], str(owner)], check=True)
    exports = inspect(owner)
    verify(exports, True)
    if any(entries[0][1] != "T" for entries in exports.values()):
        raise RuntimeError("Swift runtime promotion failed")
    report = dict(adapter="swift-rs-1.0.8-xcode27", owner=str(owner),
                  symbols=list(SYMBOLS), before_sha256=before,
                  after_sha256=hashlib.sha256(owner.read_bytes()).hexdigest(),
                  other_archives=[str(archives[name]) for name in ARCHIVES[1:]])
    (target_root.parent / "swift-rs-compat.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    compiler, *arguments = sys.argv[1:]
    # Cargo also probes rustc and compiles build.rs through a workspace wrapper.
    # Only the final iOS product needs the compatibility step.
    if "--crate-name" in arguments and arguments[arguments.index("--crate-name") + 1] == "cowboy_app_lib":
        target = arguments[arguments.index("--target") + 1] if "--target" in arguments else ""
        if target in ("aarch64-apple-ios", "aarch64-apple-ios-sim"):
            version = subprocess.check_output(["xcodebuild", "-version"], text=True)
            if int(re.search(r"^Xcode (\d+)", version)[1]) >= 27:
                root = Path(os.environ["CARGO_TARGET_DIR"])
                sysroot = subprocess.check_output([compiler, "--print", "sysroot"], text=True).strip()
                objcopy = Path(sysroot) / "lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"
                repair(arguments, root, objcopy)
    os.execv(compiler, [compiler, *arguments])


if __name__ == "__main__":
    main()
