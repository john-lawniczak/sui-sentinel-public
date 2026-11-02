#!/usr/bin/env python3
"""Bound maintained files; generated/captured artifacts have explicit exceptions."""

import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
LIMIT = 400
# These data artifacts are reviewed via their generator or capture provenance.
EXEMPT_FILES = {"Cargo.lock", "deploy/grafana/dashboards/snapshot.json"}
EXEMPT_ROOTS = ("docs/evidence/", "tests/data/")


def main():
    paths = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
    ).decode().split("\0")
    failures = []
    checked = 0
    for name in sorted(set(paths) - {""}):
        path = ROOT / name
        if name in EXEMPT_FILES or name.startswith(EXEMPT_ROOTS) or not path.is_file():
            continue
        contents = path.read_bytes()
        if b"\0" in contents:  # Binary assets are not source text.
            continue
        checked += 1
        count = len(contents.splitlines())
        if count > LIMIT:
            failures.append(f"{name}: {count} lines (limit {LIMIT}); split by responsibility")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"Checked {checked} maintained text files: all within {LIMIT} lines.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
