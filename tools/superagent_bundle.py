#!/usr/bin/env python3
"""Build a reviewable, immutable source bundle from explicitly selected Markdown."""

import argparse
import json
from pathlib import Path
import subprocess
import tomllib


def read_within(root: Path, relative: str) -> str:
    path = Path(relative)
    if path.is_absolute() or any(part in ("", ".", "..") for part in relative.split("/")):
        raise ValueError("source paths must stay inside the selected root")
    resolved = (root / path).resolve(strict=True)
    if not resolved.is_relative_to(root) or not resolved.is_file():
        raise ValueError("source path escapes its root or is not a file")
    if resolved.stat().st_size > 32768:
        raise ValueError("source file exceeds 32 KiB")
    return resolved.read_text(encoding="utf-8")


def bundle(root_name: str, profile: str) -> dict:
    root = Path(root_name).resolve(strict=True)
    text = read_within(root, profile)
    if not text.startswith("+++\n"):
        raise ValueError("profile requires +++ TOML frontmatter")
    header = tomllib.loads(text[4:].split("\n+++\n", 1)[0])
    if header.get("schema") != "cowboy.pattern-profile/v1":
        raise ValueError("unsupported profile schema")
    files = {profile: text}
    for guidance in header.get("guidance", []):
        files[guidance] = read_within(root, guidance)
    patterns = list(header.get("patterns", {}).values())
    patterns.extend(item["file"] for item in header.get("replace", {}).values())
    for pattern in patterns:
        relative = str(Path(profile).parent / pattern)
        files[relative] = read_within(root, relative)
    revision = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    # Source revision is informational. The Controller hashes the exact closure;
    # neither a clean worktree nor an approved version is inferred from Git HEAD.
    return {"revision": revision, "profile": profile, "files": files}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", nargs=2, action="append", required=True,
                        metavar=("ROOT", "PROFILE"))
    args = parser.parse_args()
    sources = [bundle(root, profile) for root, profile in args.source]
    encoded = json.dumps(sources, ensure_ascii=False, indent=2)
    if len(encoded.encode("utf-8")) > 800000:
        raise ValueError("bundle exceeds the import limit")
    print(encoded)


if __name__ == "__main__":
    main()
