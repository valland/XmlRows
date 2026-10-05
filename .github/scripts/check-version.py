#!/usr/bin/env python3
"""Confirm every manifest agrees on the application version.

An attested build is only useful if the artifact's name and the version
inside it match the tag a reader verified. This runs before the long build
so a mismatch fails in seconds rather than after a release is published.

    python3 .github/scripts/check-version.py [--tag v0.1.3]

The agreed version goes to stdout; everything else goes to stderr.
"""

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent


def json_at(relative: str, *keys: str) -> str:
    data = json.loads((ROOT / relative).read_text())
    for key in keys:
        data = data[key]
    return data


def search(relative: str, pattern: str) -> str:
    text = (ROOT / relative).read_text()
    found = re.search(pattern, text)
    if not found:
        raise SystemExit(f"check-version: no version found in {relative}")
    return found.group(1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="release tag that must equal v<version>")
    args = parser.parse_args()

    found = {
        "package.json": json_at("package.json", "version"),
        "package-lock.json (root)": json_at("package-lock.json", "version"),
        "package-lock.json (packages)": json_at(
            "package-lock.json", "packages", "", "version"
        ),
        "src-tauri/Cargo.toml": search(
            "src-tauri/Cargo.toml", r'(?m)^version = "([^"]+)"'
        ),
        "src-tauri/Cargo.lock": search(
            "src-tauri/Cargo.lock",
            r'\[\[package\]\]\nname = "xmlrows"\nversion = "([^"]+)"',
        ),
        "src-tauri/tauri.conf.json": json_at("src-tauri/tauri.conf.json", "version"),
    }

    width = max(len(name) for name in found)
    for name, value in found.items():
        print(f"  {name.ljust(width)}  {value}", file=sys.stderr)

    version = found["package.json"]
    disagree = {name: v for name, v in found.items() if v != version}
    if disagree:
        for name, value in disagree.items():
            print(
                f"check-version: {name} is {value}, expected {version}",
                file=sys.stderr,
            )
        return 1

    if args.tag and args.tag != f"v{version}":
        print(
            f"check-version: tag {args.tag} does not match version {version} "
            f"(expected v{version})",
            file=sys.stderr,
        )
        return 1

    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
