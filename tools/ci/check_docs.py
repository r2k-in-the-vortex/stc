#!/usr/bin/env python3
"""Repository checks for the specification tree.

Usage:
    python tools/ci/check_docs.py [repository-root]

The default root is the repository containing this script.

Checks:
  * every document required by the M0 specification baseline exists;
  * relative Markdown links resolve inside the repository;
  * every specification document names the profile identifier;
  * the Rust and Lean toolchain pins exist and are non-empty;
  * no PDF under Specification/Reference is tracked, and reference PDFs stay ignored;
  * no TODO/TBD/FIXME placeholders remain outside fenced/inline code
    (a line containing ``ci:allow-placeholder`` is exempt).

Only the Python standard library and Git are required.
"""

from __future__ import annotations

import re
import subprocess
import sys
import urllib.parse
from pathlib import Path

PROFILE = "stc-2013-oop-utf8-v1"

REQUIRED_DOCUMENTS = (
    "README.md",
    "Specification/00-Project-Plan.md",
    "Specification/01-Conformance-Profile.md",
    "Specification/Syntax/Tokenization.md",
    "Specification/Syntax/Grammar.md",
    "Specification/AST/AST.md",
    "Specification/Architecture/Architecture.md",
    "Specification/Architecture/Runtime-and-Execution.md",
    "Specification/Architecture/IO-and-Devices.md",
    "Specification/Architecture/State-and-Retain.md",
    "Specification/Architecture/CLI-and-Manifest.md",
    "Specification/Architecture/Testing-and-Verification.md",
    "Specification/Architecture/Online-Change-and-GOT.md",
    "Specification/Architecture/Debug-and-Monitoring.md",
    "Specification/Decisions.md",
    "Specification/UTF8-Strings.md",
)

# The reference-material README describes the local PDF only; it is not part of the profile.
PROFILE_EXEMPT = {"Specification/Reference/README.md"}

TOOLCHAIN_PINS = (
    "rust-toolchain.toml",
    "lean/lean-toolchain",
)

LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
INLINE_CODE_RE = re.compile(r"`[^`\n]*`")
PLACEHOLDER_RE = re.compile(r"\b(?:TODO|TBD|FIXME)\b")


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def mask_code(text: str) -> str:
    """Replace fenced and inline code with blank text, preserving line numbers."""
    lines: list[str] = []
    fence: str | None = None
    for line in text.splitlines():
        stripped = line.lstrip()
        if fence is None:
            if stripped.startswith("```") or stripped.startswith("~~~"):
                fence = stripped[:3]
                lines.append("")
            else:
                lines.append(line)
        else:
            if stripped.startswith(fence):
                fence = None
            lines.append("")
    return INLINE_CODE_RE.sub("", "\n".join(lines))


def relative_path(path: Path, root: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return str(path)


def check_required_documents(root: Path, errors: list[str]) -> int:
    count = 0
    for name in REQUIRED_DOCUMENTS:
        path = root / name
        if not path.is_file():
            errors.append(f"{name}: required document is missing")
        elif not read_text(path).strip():
            errors.append(f"{name}: required document is empty")
        else:
            count += 1
    return count


def check_links(root: Path, path: Path, text: str, errors: list[str]) -> int:
    rel = relative_path(path, root)
    masked = mask_code(text)
    count = 0
    for match in LINK_RE.finditer(masked):
        target = match.group(1).strip()
        if target.startswith("<") and ">" in target:
            target = target[1 : target.index(">")]
        else:
            target = target.split()[0] if target else ""
        if not target or target.lower().startswith(("#", "http://", "https://", "mailto:")):
            continue

        raw_path = target.split("#", 1)[0].split("?", 1)[0]
        if not raw_path:
            continue

        decoded = urllib.parse.unquote(raw_path)
        if decoded.startswith("/"):
            resolved = (root / decoded.lstrip("/")).resolve()
        else:
            resolved = (path.parent / decoded).resolve()

        line = masked.count("\n", 0, match.start()) + 1
        count += 1
        try:
            resolved.relative_to(root)
        except ValueError:
            errors.append(f"{rel}:{line}: link escapes the repository: {target}")
            continue
        if not resolved.exists():
            errors.append(
                f"{rel}:{line}: broken link: {target} "
                f"(resolved to {relative_path(resolved, root)})"
            )
    return count


def check_profile(root: Path, path: Path, text: str, errors: list[str]) -> None:
    rel = relative_path(path, root)
    if rel in PROFILE_EXEMPT:
        return
    if PROFILE not in text:
        errors.append(f"{rel}: missing profile identifier {PROFILE!r}")


def check_placeholders(root: Path, path: Path, text: str, errors: list[str]) -> None:
    rel = relative_path(path, root)
    for number, line in enumerate(mask_code(text).splitlines(), 1):
        if "ci:allow-placeholder" in line:
            continue
        if PLACEHOLDER_RE.search(line):
            errors.append(f"{rel}:{number}: placeholder marker: {line.strip()}")


def git(root: Path, *args: str) -> subprocess.CompletedProcess[str] | None:
    try:
        return subprocess.run(
            ["git", "-C", str(root), *args],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None


def check_toolchain_pins(root: Path, errors: list[str]) -> None:
    for name in TOOLCHAIN_PINS:
        path = root / name
        if not path.is_file():
            errors.append(f"{name}: toolchain pin is missing")
        elif not read_text(path).strip():
            errors.append(f"{name}: toolchain pin is empty")


def check_reference_pdf(root: Path, errors: list[str]) -> None:
    if not (root / ".git").exists():
        return

    tracked = git(root, "ls-files", "--", "Specification/Reference")
    if tracked is None:
        errors.append("git is unavailable; cannot verify tracked reference PDFs")
        return
    if tracked.returncode != 0:
        errors.append(f"git ls-files failed: {tracked.stderr.strip()}")
        return
    pdfs = [line for line in tracked.stdout.splitlines() if line.lower().endswith(".pdf")]
    if pdfs:
        errors.append("reference PDF is tracked by Git: " + ", ".join(pdfs))

    ignored = git(
        root,
        "check-ignore",
        "-q",
        "--no-index",
        "Specification/Reference/reference.pdf",
    )
    if ignored is None:
        errors.append("git is unavailable; cannot verify the reference PDF ignore rule")
    elif ignored.returncode != 0:
        errors.append("Specification/Reference/*.pdf is not ignored by .gitignore")


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print(f"usage: {argv[0]} [repository-root]", file=sys.stderr)
        return 2

    root = Path(argv[1]).resolve() if len(argv) == 2 else Path(__file__).resolve().parents[2]
    if not (root / "README.md").is_file():
        print(f"check_docs: {root} does not look like the stc repository", file=sys.stderr)
        return 2

    errors: list[str] = []
    document_count = check_required_documents(root, errors)
    link_count = 0

    documents = [root / "README.md", *sorted((root / "Specification").rglob("*.md"))]
    for path in documents:
        text = read_text(path)
        check_profile(root, path, text, errors)
        check_placeholders(root, path, text, errors)
        link_count += check_links(root, path, text, errors)

    check_toolchain_pins(root, errors)
    check_reference_pdf(root, errors)

    if errors:
        print(f"check_docs: {len(errors)} problem(s)")
        for error in errors:
            print(f"  {error}")
        return 1

    print(
        f"check_docs: OK ({document_count} required documents, "
        f"{link_count} relative links, profile {PROFILE})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
