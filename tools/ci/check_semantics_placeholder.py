#!/usr/bin/env python3
"""Check the fail-closed semantics-conformance gate (Plan-Phase0.md P0.6).

Usage:
    python tools/ci/check_semantics_placeholder.py [repository-root]

The Phase 0 harness must never report agreement for an empty corpus. While
`tests/verification/semantics` has no `case.json` descriptors, this check
requires `lake exe semantics-conformance` to exit nonzero. Once cases exist the
harness must run them and succeed, which is the P0.6 gate; a case added to the
tree fails CI rather than passing silently.

The conformance report is a committed artifact. On a non-empty corpus this
check regenerates it and compares bytes, so a corpus or harness change that is
not reflected in the committed report fails CI instead of drifting silently.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

CORPUS = Path("tests/verification/semantics")
REPORT = CORPUS / "conformance-report.json"


def _report_difference(committed: Path, generated: Path) -> str:
    """Describe the first stale parts of a byte-different report."""
    try:
        old = json.loads(committed.read_text(encoding="utf-8"))
        new = json.loads(generated.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return f"cannot compare report objects: {error}"
    keys = sorted(set(old) | set(new))
    changed = [key for key in keys if old.get(key) != new.get(key)]
    if changed:
        return "changed report members: " + ", ".join(changed)
    return "reports parse equal but differ in bytes"


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print(f"usage: {argv[0]} [repository-root]", file=sys.stderr)
        return 2
    root = Path(argv[1]).resolve() if len(argv) == 2 else Path(__file__).resolve().parents[2]
    lean = root / "lean"
    corpus = root / CORPUS
    report = root / REPORT
    cases = sorted(corpus.rglob("case.json")) if corpus.is_dir() else []

    with tempfile.TemporaryDirectory() as scratch:
        generated = Path(scratch) / "conformance-report.json"
        completed = subprocess.run(
            ["lake", "exe", "semantics-conformance", "--report", str(generated)],
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

        if not report.is_file():
            print(
                f"check_semantics_placeholder: FAIL: {REPORT.as_posix()} is missing",
                file=sys.stderr,
            )
            return 1
        if report.read_bytes() != generated.read_bytes():
            print(
                "check_semantics_placeholder: FAIL: the committed conformance report is "
                "stale; regenerate it with `lake exe semantics-conformance --report "
                f"{REPORT.as_posix()}`",
                file=sys.stderr,
            )
            print(_report_difference(report, generated), file=sys.stderr)
            return 1

    print(
        f"check_semantics_placeholder: OK ({len(cases)} corpus case(s) ran; "
        "report matches)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
