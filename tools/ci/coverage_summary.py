#!/usr/bin/env python3
"""Render a Markdown coverage summary from a cargo-llvm-cov JSON report.

Usage:
    python tools/ci/coverage_summary.py target/summary.json [--top 10]

CI writes the report with:

    cargo llvm-cov report --json --summary-only --output-path target/summary.json

The output is GitHub-flavored Markdown and is appended to
`$GITHUB_STEP_SUMMARY`. Files are listed by ascending line coverage so the
least-covered sources are the ones a reviewer sees first.

Only the Python standard library is required.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

METRICS = (
    ("lines", "Lines"),
    ("regions", "Regions"),
    ("functions", "Functions"),
)


def percent(value: float) -> str:
    return f"{value:.1f}%"


def escape_cell(text: str) -> str:
    return text.replace("|", "\\|")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="JSON report from cargo llvm-cov")
    parser.add_argument(
        "--top",
        type=int,
        default=10,
        help="number of lowest-coverage files to list (default: 10)",
    )
    args = parser.parse_args(argv)

    try:
        document = json.loads(args.report.read_text(encoding="utf-8-sig"))
        payload = document["data"][0]
        totals = payload["totals"]
    except OSError as error:
        raise SystemExit(f"coverage_summary: cannot read {args.report}: {error}") from error
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"coverage_summary: {args.report} is not an llvm-cov JSON report") from error

    print("## Rust coverage")
    print()
    print("| Metric | Covered | Total | Percent |")
    print("| --- | ---: | ---: | ---: |")
    for key, label in METRICS:
        metric = totals.get(key) or {}
        count = metric.get("count", 0)
        if count == 0:
            continue
        print(
            f"| {label} | {metric.get('covered', 0)} | {count} "
            f"| {percent(metric.get('percent', 0.0))} |"
        )

    rows: list[tuple[float, str, int, int]] = []
    for entry in payload.get("files") or []:
        summary = (entry.get("summary") or {}).get("lines") or {}
        count = summary.get("count", 0)
        if count == 0:
            continue
        rows.append(
            (
                summary.get("percent", 0.0),
                entry.get("filename", ""),
                summary.get("covered", 0),
                count,
            )
        )
    rows.sort(key=lambda row: (row[0], row[1]))

    if args.top > 0 and rows:
        root = Path.cwd().resolve()
        print()
        print(f"Lowest line coverage ({min(args.top, len(rows))} of {len(rows)} files)")
        print()
        print("| File | Lines | Percent |")
        print("| --- | ---: | ---: |")
        for value, filename, covered, count in rows[: args.top]:
            path = Path(filename)
            try:
                display = path.resolve().relative_to(root).as_posix()
            except (OSError, ValueError):
                display = filename
            print(f"| `{escape_cell(display)}` | {covered}/{count} | {percent(value)} |")

    return 0


if __name__ == "__main__":
    sys.exit(main())
