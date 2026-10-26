#!/usr/bin/env python3
"""Run the repository checks that CI runs, locally.

The CI matrix separates concerns into independent jobs. Running each of them by
hand is easy to forget, and two of them are not covered by `cargo test` or
`cargo clippy --workspace`:

* `fuzz/` is a separate workspace. A change to a library API can break the
  fuzz targets without breaking the main workspace.
* Markdown lint is a separate docs job (`markdownlint-cli2`), not part of
  `tools/ci/check_docs.py`.

Usage:
    python tools/ci/check_local.py
    python tools/ci/check_local.py --fail-fast
    python tools/ci/check_local.py --skip-markdownlint --skip-fuzz

On Windows, run it from the repository root with any Python 3 interpreter.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MARKDOWNLINT = ["npx", "--yes", "markdownlint-cli2@0.24.0"]


def command_for_windows(command: list[str]) -> list[str]:
    """Runs batch wrappers such as `npx.cmd` through cmd.exe on Windows."""
    if os.name != "nt":
        return command
    executable = shutil.which(command[0])
    if executable and executable.lower().endswith((".cmd", ".bat")):
        return ["cmd", "/c", *command]
    return command


def preflight(require_markdownlint: bool) -> list[str]:
    problems: list[str] = []
    for tool in ["cargo", "clang"]:
        if shutil.which(tool) is None:
            problems.append(
                f"`{tool}` was not found on PATH; install it before running the local checks"
            )
    if require_markdownlint and shutil.which("npx") is None:
        problems.append(
            "`npx` was not found on PATH; install Node.js or pass --skip-markdownlint"
        )
    return problems


def run_step(
    name: str,
    command: list[str],
    *,
    cwd: Path = ROOT,
    env: dict[str, str] | None = None,
) -> str | None:
    print(f"\n==> {name}", flush=True)
    print(f"    {' '.join(command)}", flush=True)
    started = time.monotonic()
    try:
        completed = subprocess.run(
            command_for_windows(command),
            cwd=cwd,
            env=env,
            check=False,
        )
    except OSError as error:
        print(f"    cannot run: {error}", file=sys.stderr, flush=True)
        return name
    elapsed = time.monotonic() - started
    if completed.returncode != 0:
        print(
            f"    FAILED (exit {completed.returncode}, {elapsed:.1f}s)",
            file=sys.stderr,
            flush=True,
        )
        return name
    print(f"    ok ({elapsed:.1f}s)", flush=True)
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="stop at the first failing check (default: run every check)",
    )
    parser.add_argument(
        "--skip-markdownlint",
        action="store_true",
        help="skip the markdownlint job (for environments without Node.js)",
    )
    parser.add_argument(
        "--skip-fuzz",
        action="store_true",
        help="skip compiling the separate fuzz workspace",
    )
    args = parser.parse_args()

    problems = preflight(not args.skip_markdownlint)
    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 2

    # Match the CI test job: do not let missing clang silently skip execution
    # tests (the test harnesses otherwise return early).
    test_env = os.environ.copy()
    test_env["STC_REQUIRE_CLANG"] = "1"

    steps: list[tuple[str, list[str], Path, dict[str, str] | None]] = [
        (
            "Specification, links, and required documents",
            [sys.executable, str(ROOT / "tools/ci/check_docs.py")],
            ROOT,
            None,
        ),
    ]
    if not args.skip_markdownlint:
        steps.append(("Markdown lint", MARKDOWNLINT, ROOT, None))
    steps.append(("Rust formatting", ["cargo", "fmt", "--all", "--check"], ROOT, None))
    steps.append(
        (
            "Rust clippy (workspace, all targets)",
            [
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            ROOT,
            None,
        )
    )
    steps.append(
        (
            "Workspace tests (clang required)",
            ["cargo", "test", "--workspace", "--locked"],
            ROOT,
            test_env,
        )
    )
    if not args.skip_fuzz:
        steps.append(
            (
                "Fuzz targets compile (separate workspace)",
                [
                    "cargo",
                    "check",
                    "--manifest-path",
                    "fuzz/Cargo.toml",
                    "--bins",
                    "--locked",
                ],
                ROOT,
                None,
            )
        )

    failed: list[str] = []
    for name, command, cwd, env in steps:
        if run_step(name, command, cwd=cwd, env=env) is not None:
            failed.append(name)
            if args.fail_fast:
                break

    if failed:
        print("\nLocal checks FAILED:", file=sys.stderr)
        for name in failed:
            print(f"  - {name}", file=sys.stderr)
        return 1
    print("\nAll local checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
