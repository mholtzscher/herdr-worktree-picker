#!/usr/bin/env python3
"""Calculate the next conventional-commit release and update version files."""

from __future__ import annotations

import argparse
import re
import subprocess
import tomllib
from pathlib import Path

RELEASING_TYPES = {"fix": "patch", "perf": "patch", "revert": "patch", "feat": "minor"}
BUMP_ORDER = {"patch": 1, "minor": 2, "major": 3}


def run_git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True).strip()


def latest_tag() -> str:
    try:
        return run_git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*")
    except subprocess.CalledProcessError as error:
        raise SystemExit("No release tag found; create the initial release manually.") from error


def bump_for_message(message: str) -> str | None:
    subject = message.splitlines()[0] if message else ""
    match = re.match(r"^([a-z]+)(?:\([^)]*\))?(!)?:\s+", subject)
    if not match:
        return None
    if match.group(2) or re.search(r"^BREAKING[ -]CHANGE:\s+", message, re.MULTILINE):
        return "major"
    return RELEASING_TYPES.get(match.group(1))


def required_bump(messages: list[str]) -> str | None:
    bumps = [bump_for_message(message) for message in messages]
    bumps = [bump for bump in bumps if bump is not None]
    return max(bumps, key=BUMP_ORDER.get) if bumps else None


def bump_version(version: str, bump: str) -> str:
    major, minor, patch = map(int, version.split("."))
    if bump == "major":
        return f"{major + 1}.0.0"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def replace_one(path: Path, pattern: str, replacement: str) -> None:
    content = path.read_text()
    updated, count = re.subn(pattern, replacement, content, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"Could not update version in {path}")
    path.write_text(updated)


def update_versions(root: Path, current: str, new: str) -> None:
    replace_one(
        root / "Cargo.toml",
        rf'(^name = "herdr-worktree-picker"\nversion = "){re.escape(current)}("$)',
        rf"\g<1>{new}\2",
    )
    replace_one(
        root / "Cargo.lock",
        rf'(^name = "herdr-worktree-picker"\nversion = "){re.escape(current)}("$)',
        rf"\g<1>{new}\2",
    )
    replace_one(
        root / "herdr-plugin.toml",
        rf'(^version = "){re.escape(current)}("$)',
        rf"\g<1>{new}\2",
    )
    replace_one(
        root / "README.md",
        rf"(--ref v){re.escape(current)}\b",
        rf"\g<1>{new}",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    root = Path(__file__).resolve().parent.parent
    current = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    tag = latest_tag()
    if tag != f"v{current}":
        raise SystemExit(f"Cargo version {current} does not match latest tag {tag}")

    log = run_git("log", "--format=%B%x00", f"{tag}..HEAD")
    messages = [message.strip() for message in log.split("\0") if message.strip()]
    bump = required_bump(messages)
    values = {"release": "false"}
    if bump:
        version = bump_version(current, bump)
        values = {"release": "true", "version": version, "tag": f"v{version}", "bump": bump}
        if not args.dry_run:
            update_versions(root, current, version)
        print(f"{bump} release: v{current} -> v{version}")
    else:
        print(f"No releasable conventional commits since {tag}")

    if args.github_output:
        with args.github_output.open("a") as output:
            for key, value in values.items():
                output.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
