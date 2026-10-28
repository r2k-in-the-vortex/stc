#!/usr/bin/env python3
"""Check the fail-closed semantics-conformance placeholder (Plan-Phase0.md P0.0).

Usage:
    python tools/ci/check_semantics_placeholder.py [repository-root]

The Phase 0 harness must never report agreement for an empty corpus. While
`tests/verification/semantics` has no `case.json` descriptors, this check
requires `lake exe semantics-conformance` to exit nonzero. Once cases exist the
harness must run them and succeed, which is the P0.6 gate; until then a case
added to the tree fails CI rather than passing silently.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

CORPUS = Path("tests/verification/semantics")


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print(f"usage: {argv[0]} [repository-root]", file=sys.stderr)
        return 2
    root = Path(argv[1]).resolve() if len(argv) == 2 else Path(__file__).resolve().parents[2]
    lean = root / "lean"
    corpus = root / CORPUS
    cases = sorted(corpus.rglob("case.json")) if corpus.is_dir() else []

    completed = subprocess.run(
        ["lake", "exe", "semantics-conformance"],
        cwd=lean,
        capture_output=True,
        text=True,
        check=False,
    )
    output = (completed.stdout + completed.stderr).strip()

    if not cases:
        if completed.returncode == 0:
            print(
                "check_semantics_placeholder: FAIL: the harness accepted an empty corpus",
                file=sys.stderr,
            )
            if output:
                print(output, file=sys.stderr)
            return 1
        print(
            "check_semantics_placeholder: OK (fail-closed placeholder, "
            f"exit {completed.returncode}, no corpus cases)"
        )
        return 0

    if completed.returncode != 0:
        print(
            "check_semantics_placeholder: FAIL: corpus cases exist but the harness "
            f"exited {completed.returncode}",
            file=sys.stderr,
        )
        if output:
            print(output, file=sys.stderr)
        return 1
    print(f"check_semantics_placeholder: OK ({len(cases)} corpus case(s) ran)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
