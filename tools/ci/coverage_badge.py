#!/usr/bin/env python3
"""Build a line-coverage badge from an LCOV report.

Usage:
    python tools/ci/coverage_badge.py target/lcov.info \
        --svg target/coverage.svg \
        --json target/coverage.json

The SVG is a self-contained shields-style flat badge, so the README badge does
not depend on an external badge service. CI publishes the generated files to
the `badges` branch; the README links to the raw SVG there.

Only line coverage (the LCOV `LF:`/`LH:` records) is used. The threshold colors
match the common coverage conventions: >= 90 brightgreen, >= 80 green,
>= 70 yellowgreen, >= 60 yellow, >= 50 orange, otherwise red.

Only the Python standard library is required.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from xml.sax.saxutils import escape

# (minimum percentage, shields color name, hex color)
THRESHOLDS = (
    (90.0, "brightgreen", "#4c1"),
    (80.0, "green", "#97ca00"),
    (70.0, "yellowgreen", "#a4a61d"),
    (60.0, "yellow", "#dfb317"),
    (50.0, "orange", "#fe7d37"),
)
FALLBACK = ("red", "#e05d44")

# Approximate advance width of Verdana at 11 px. Exact measurement is not
# needed; the value only sizes the badge boxes around the centered text.
CHAR_WIDTH = 7.0
PADDING = 10.0
HEIGHT = 20.0


def parse_lcov(path: Path) -> tuple[int, int]:
    """Return (lines found, lines hit) from an LCOV file."""
    try:
        text = path.read_text(encoding="utf-8-sig")
    except OSError as error:
        raise SystemExit(f"coverage_badge: cannot read {path}: {error}") from error

    found = 0
    hit = 0
    for line in text.splitlines():
        if line.startswith("LF:"):
            found += int(line[3:].strip())
        elif line.startswith("LH:"):
            hit += int(line[3:].strip())
    if found == 0:
        raise SystemExit(f"coverage_badge: {path} has no line coverage records")
    if hit > found:
        raise SystemExit(
            f"coverage_badge: {path} reports more hit lines than found "
            f"({hit} > {found})"
        )
    return found, hit


def color_for(percent: float) -> tuple[str, str]:
    for minimum, name, hex_color in THRESHOLDS:
        if percent >= minimum:
            return name, hex_color
    return FALLBACK


def text_width(text: str) -> float:
    return round(len(text) * CHAR_WIDTH + PADDING)


def render_svg(label: str, message: str, hex_color: str) -> str:
    label_width = text_width(label)
    message_width = text_width(message)
    total_width = label_width + message_width
    label_center = label_width / 2
    message_center = label_width + message_width / 2
    label = escape(label)
    message = escape(message)
    aria_label = escape(f"{label}: {message}", {'"': "&quot;"})
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{total_width:g}" '
        f'height="{HEIGHT:g}" role="img" '
        f'aria-label="{aria_label}">\n'
        '  <linearGradient id="s" x2="0" y2="100%">\n'
        '    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>\n'
        '    <stop offset="1" stop-opacity=".1"/>\n'
        "  </linearGradient>\n"
        f'  <clipPath id="r"><rect width="{total_width:g}" '
        f'height="{HEIGHT:g}" rx="3" fill="#fff"/></clipPath>\n'
        '  <g clip-path="url(#r)">\n'
        f'    <rect width="{label_width:g}" height="{HEIGHT:g}" fill="#555"/>\n'
        f'    <rect x="{label_width:g}" width="{message_width:g}" '
        f'height="{HEIGHT:g}" fill="{hex_color}"/>\n'
        f'    <rect width="{total_width:g}" height="{HEIGHT:g}" fill="url(#s)"/>\n'
        "  </g>\n"
        '  <g fill="#fff" text-anchor="middle" '
        'font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">\n'
        f'    <text x="{label_center:g}" y="15" fill="#010101" '
        f'fill-opacity=".3">{label}</text>\n'
        f'    <text x="{label_center:g}" y="14">{label}</text>\n'
        f'    <text x="{message_center:g}" y="15" fill="#010101" '
        f'fill-opacity=".3">{message}</text>\n'
        f'    <text x="{message_center:g}" y="14">{message}</text>\n'
        "  </g>\n"
        "</svg>\n"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lcov", type=Path, help="LCOV report from cargo llvm-cov")
    parser.add_argument("--svg", type=Path, help="write the badge SVG here")
    parser.add_argument("--json", type=Path, help="write the shields endpoint JSON here")
    parser.add_argument("--label", default="coverage", help="badge label (default: coverage)")
    args = parser.parse_args(argv)

    if args.svg is None and args.json is None:
        parser.error("at least one of --svg or --json is required")

    found, hit = parse_lcov(args.lcov)
    percent = 100.0 * hit / found
    message = f"{percent:.1f}%"
    bright, hex_color = color_for(percent)

    if args.svg is not None:
        args.svg.parent.mkdir(parents=True, exist_ok=True)
        args.svg.write_text(render_svg(args.label, message, hex_color), encoding="utf-8")
    if args.json is not None:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        endpoint = {
            "schemaVersion": 1,
            "label": args.label,
            "message": message,
            "color": bright,
        }
        args.json.write_text(json.dumps(endpoint, indent=2) + "\n", encoding="utf-8")

    print(f"{args.label}: {hit}/{found} lines ({message})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
