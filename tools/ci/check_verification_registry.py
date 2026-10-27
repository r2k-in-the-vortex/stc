#!/usr/bin/env python3
"""Validate the Phase 0 verification registry (Plan-Phase0.md §7, §11).

Usage:
    python tools/ci/check_verification_registry.py [repository-root]

Checks:
  * the registry parses and uses the documented schema and versions;
  * ids, names, phases, levels, kinds, and statuses are valid and unique;
  * every specification reference resolves to an existing section anchor;
  * every referenced Lean module exists;
  * every proved entry resolves in the Lean environment and depends only on
    the standard Lean axioms (`propext`, `Classical.choice`, `Quot.sound`);
  * every "descoped" entry records a reason.

Only the Python standard library, a `tomllib`-capable Python (3.11 or later),
and `lake` on PATH are required. The Lean audit runs only when at least one
entry is marked `proved`.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

SCHEMA_VERSION = 1
PROFILE = "stc-2013-oop-utf8-v1"
REGISTRY = Path("lean/verification-status.toml")

REQUIRED_FIELDS = {
    "id",
    "kind",
    "name",
    "phase",
    "level",
    "spec",
    "module",
    "owner",
    "status",
    "tcb",
}
OPTIONAL_FIELDS = {"note"}
ALLOWED_KINDS = {"theorem", "evidence", "definition"}
ALLOWED_STATUSES = {"planned", "proved", "tested", "descoped"}
ALLOWED_LEVELS = {"S", "V0", "V1", "V2", "V3", "V4"}
ALLOWED_PHASES = {f"P0.{index}" for index in range(8)}
ALLOWED_AXIOMS = {"propext", "Classical.choice", "Quot.sound"}
UMBRELLA_IMPORTS = (
    "Stc.Syntax",
    "Stc.Grammar",
    "Stc.Hir",
    "Stc.Sem",
    "Stc.Codegen",
    "Verification.Semantics",
)
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
LINK_RE = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def github_slug(heading: str) -> str:
    """Approximate GitHub's Markdown heading anchor algorithm."""
    text = LINK_RE.sub(r"\1", heading.strip())
    text = text.replace("`", "").lower()
    text = re.sub(r"[^\w\s-]", "", text)
    text = re.sub(r"\s+", "-", text)
    return text.strip("-")


def heading_slugs(path: Path) -> set[str]:
    slugs: set[str] = set()
    for line in read_text(path).splitlines():
        match = HEADING_RE.match(line)
        if match:
            slugs.add(github_slug(match.group(2)))
    return slugs


def module_path(root: Path, module: str) -> Path:
    return root / "lean" / Path(*module.split(".")).with_suffix(".lean")


def check_spec(root: Path, record: dict, errors: list[str]) -> None:
    entry = record["id"]
    spec = record["spec"]
    raw_path, separator, anchor = spec.partition("#")
    if not separator or not raw_path or not anchor:
        errors.append(f"{entry}: spec must be 'path#section-anchor': {spec!r}")
        return
    target = (root / raw_path).resolve()
    try:
        target.relative_to(root)
    except ValueError:
        errors.append(f"{entry}: spec escapes the repository: {spec!r}")
        return
    if not target.is_file():
        errors.append(f"{entry}: spec file is missing: {raw_path}")
        return
    if anchor not in heading_slugs(target):
        errors.append(f"{entry}: spec section anchor not found: {raw_path}#{anchor}")


def check_record(root: Path, record: dict, errors: list[str]) -> None:
    if not isinstance(record, dict):
        errors.append("record is not a table")
        return
    entry = record.get("id", "<unknown>")
    missing = REQUIRED_FIELDS - record.keys()
    unknown = record.keys() - REQUIRED_FIELDS - OPTIONAL_FIELDS
    if missing:
        errors.append(f"{entry}: missing field(s): {', '.join(sorted(missing))}")
    if unknown:
        errors.append(f"{entry}: unknown field(s): {', '.join(sorted(unknown))}")
    if missing or unknown:
        return

    for field in ("id", "name", "owner", "tcb"):
        if not isinstance(record[field], str) or not record[field].strip():
            errors.append(f"{entry}: {field} must be a non-empty string")
    if record["kind"] not in ALLOWED_KINDS:
        errors.append(f"{entry}: kind must be one of {sorted(ALLOWED_KINDS)}")
    if record["status"] not in ALLOWED_STATUSES:
        errors.append(f"{entry}: status must be one of {sorted(ALLOWED_STATUSES)}")
    if record["level"] not in ALLOWED_LEVELS:
        errors.append(f"{entry}: level must be one of {sorted(ALLOWED_LEVELS)}")
    if record["phase"] not in ALLOWED_PHASES:
        errors.append(f"{entry}: phase must be one of {sorted(ALLOWED_PHASES)}")
    if record["status"] == "proved" and record["kind"] == "evidence":
        errors.append(f"{entry}: evidence cannot have status 'proved'; use 'tested'")
    if record["status"] == "tested" and record["kind"] != "evidence":
        errors.append(f"{entry}: status 'tested' is reserved for evidence entries")
    if record["status"] == "descoped" and not record.get("note"):
        errors.append(f"{entry}: status 'descoped' requires a note with the reason")
    if not module_path(root, record["module"]).is_file():
        errors.append(f"{entry}: Lean module is missing: {record['module']}")
    check_spec(root, record, errors)


def lean_audit(root: Path, records: list[dict], errors: list[str]) -> None:
    proved = [record for record in records if record["status"] == "proved"]
    if not proved:
        return
    lake = shutil.which("lake")
    if lake is None:
        errors.append("lake is not on PATH; cannot resolve proved theorems")
        return

    imports = set(UMBRELLA_IMPORTS)
    imports.update(record["module"] for record in proved)
    lines = [f"import {module}" for module in sorted(imports)]
    for record in proved:
        lines.append(f"#check {record['name']}")
        lines.append(f"#print axioms {record['name']}")

    with tempfile.TemporaryDirectory(prefix="stc-axiom-audit-") as directory:
        audit = Path(directory) / "Audit.lean"
        audit.write_text("\n".join(lines) + "\n", encoding="utf-8")
        completed = subprocess.run(
            [lake, "env", "lean", str(audit)],
            cwd=root / "lean",
            capture_output=True,
            text=True,
            check=False,
        )
    output = completed.stdout + completed.stderr
    if completed.returncode != 0:
        errors.append(f"Lean audit failed (exit {completed.returncode}):\n{output.strip()}")
        return

    for record in proved:
        entry = record["id"]
        name = record["name"]
        quoted = f"'{name}'"
        if quoted not in output:
            errors.append(f"{entry}: no #print axioms output for {name}")
            continue
        match = re.search(
            rf"'{re.escape(name)}' depends on axioms: \[(.*?)\]",
            output,
        )
        if match:
            axioms = {item.strip() for item in match.group(1).split(",") if item.strip()}
            extra = sorted(axioms - ALLOWED_AXIOMS)
            if extra:
                errors.append(
                    f"{entry}: {name} depends on non-standard axiom(s): {', '.join(extra)}"
                )
        elif f"{quoted} does not depend on any axioms" not in output:
            errors.append(f"{entry}: no axiom result found for {name}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repository root (default: the repository containing this script)",
    )
    args = parser.parse_args(argv[1:])
    root = args.root.resolve()

    path = root / REGISTRY
    if not path.is_file():
        print(f"check_verification_registry: missing registry: {REGISTRY}", file=sys.stderr)
        return 2
    try:
        data = tomllib.loads(read_text(path))
    except tomllib.TOMLDecodeError as error:
        print(f"check_verification_registry: cannot parse {REGISTRY}: {error}", file=sys.stderr)
        return 2

    errors: list[str] = []
    if data.get("schema_version") != SCHEMA_VERSION:
        errors.append(f"schema_version must be {SCHEMA_VERSION}")
    if data.get("profile") != PROFILE:
        errors.append(f"profile must be {PROFILE!r}")
    for field in ("semantics_version", "dump_format_version"):
        value = data.get(field)
        if not isinstance(value, str) or not value.strip():
            errors.append(f"{field} must be a non-empty string")

    records = data.get("records")
    if not isinstance(records, list) or not records:
        errors.append("records must be a non-empty array of tables")
        records = []

    ids: set[str] = set()
    names: set[str] = set()
    for record in records:
        check_record(root, record, errors)
        if isinstance(record, dict):
            entry = record.get("id")
            if isinstance(entry, str):
                if entry in ids:
                    errors.append(f"{entry}: duplicate id")
                ids.add(entry)
            name = record.get("name")
            if isinstance(name, str):
                if name in names:
                    errors.append(f"{name}: duplicate name")
                names.add(name)

    lean_audit(root, records, errors)

    if errors:
        print(f"check_verification_registry: {len(errors)} problem(s)")
        for error in errors:
            print(f"  {error}")
        return 1

    counts = {status: 0 for status in ALLOWED_STATUSES}
    for record in records:
        counts[record["status"]] += 1
    summary = ", ".join(f"{counts[status]} {status}" for status in sorted(ALLOWED_STATUSES))
    print(
        f"check_verification_registry: OK ({len(records)} records: {summary}; "
        f"semantics_version {data['semantics_version']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
