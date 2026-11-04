#!/usr/bin/env python3
"""Validate the verification registry (Semantics.md §7 and Phase0-Report.md).

Usage:
    python tools/ci/check_verification_registry.py [repository-root]

Checks:
  * the registry parses and uses the documented schema and versions;
  * ids, names, phases, levels, kinds, and statuses are valid and unique;
  * every specification reference resolves to an existing section anchor;
  * every referenced Lean module exists in the default `lean/` package or in
    the separate `lean/aeneas/` bridge package (generated-model records);
  * every proved entry resolves in the Lean environment and depends only on
    the standard Lean axioms (`propext`, `Classical.choice`, `Quot.sound`).
    The single owner-approved Phase 2 bridge exception (P2-T33) is exempt
    because the bridge package imports Aeneas/Mathlib and is not built in fast
    CI; a cheap textual declaration guard replaces its axiom audit. Every
    default-package proved record keeps the full audit;
  * every "descoped" entry records a reason;
  * Phase 1 records stay V0 evidence with status `tested` or `planned` (the
    Phase 1 policy: no refinement or proved bridge claim);
  * Phase 2 records stay at S or V0, with exactly one owner-approved
    exception: P2-T33, the bridge-only exact-kernel refinement theorem, may be
    kind `theorem`, status `proved`, level V1. Every other bridge-only Phase 2
    record must be V0 evidence with status `planned` or `tested`.

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
ALLOWED_PHASES = {f"P0.{index}" for index in range(8)} | {"P1", "P2"}
ALLOWED_AXIOMS = {"propext", "Classical.choice", "Quot.sound"}
# Phase 1 bridge records are evidence only: the spikes and checker interfaces
# are V0 until the refinement theorem and its external-axiom equations land.
P1_STATUSES = {"planned", "tested"}
# Phase 2 model theorems are proved in the default package at V0. The owner
# accepted Phase 2 on 2026-11-03 with exactly one bridge V1 claim: the exact
# scalar kernel bundle `Stc.Bridge.Exact.exact_kernel_refines`, relative to the
# TCB in Phase2-Report.md §8. Every other bridge-only record stays V0 evidence.
P2_LEVELS = {"S", "V0"}
P2_BRIDGE_STATUSES = {"planned", "tested"}
P2_V1_EXCEPTION = {
    "id": "P2-T33",
    "kind": "theorem",
    "name": "Stc.Bridge.Exact.exact_kernel_refines",
    "phase": "P2",
    "level": "V1",
    "module": "Bridge.Exact.Kernel",
    "status": "proved",
}
MODULE_ROOTS = (Path("lean"), Path("lean") / "aeneas")
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


def module_candidates(root: Path, module: str) -> list[Path]:
    """Return the default-package and bridge-package paths for a module."""
    relative = Path(*module.split(".")).with_suffix(".lean")
    return [root / base / relative for base in MODULE_ROOTS]


def check_module(root: Path, record: dict, errors: list[str]) -> None:
    entry = record["id"]
    candidates = module_candidates(root, record["module"])
    if any(path.is_file() for path in candidates):
        return
    looked = ", ".join(path.relative_to(root).as_posix() for path in candidates)
    errors.append(
        f"{entry}: Lean module is missing: {record['module']} "
        f"(looked for {looked})"
    )


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


def module_state(root: Path, record: dict) -> tuple[bool, bool]:
    """Return (default-package module exists, bridge-package module exists)."""
    default_module, bridge_module = (
        path.is_file() for path in module_candidates(root, record["module"])
    )
    return default_module, bridge_module


def is_p2_v1_exception(root: Path, record: dict) -> bool:
    """Whether this is the single owner-approved Phase 2 bridge V1 record.

    The match is exact on the allowlist fields and additionally requires the
    module to resolve only in the non-default bridge package, so a
    default-package record cannot inherit the exception.
    """
    if not isinstance(record, dict):
        return False
    default_module, bridge_module = module_state(root, record)
    if not (bridge_module and not default_module):
        return False
    return all(record.get(field) == value for field, value in P2_V1_EXCEPTION.items())


def check_bridge_v1_declaration(root: Path, record: dict, errors: list[str]) -> None:
    """Cheap textual guard for the bridge-only V1 theorem.

    Fast CI never builds the Aeneas/Mathlib package, so the default-package
    `#print axioms` audit cannot check this record. The full proof is checked
    by the pinned host build; here we only require the module to declare the
    recorded theorem.
    """
    path = next(
        (
            candidate
            for candidate in module_candidates(root, record["module"])[1:]
            if candidate.is_file()
        ),
        None,
    )
    if path is None:
        return  # check_module reports the missing module
    short_name = record["name"].rsplit(".", 1)[-1]
    pattern = re.compile(
        rf"^\s*(?:theorem|lemma)\s+{re.escape(short_name)}\b", re.MULTILINE
    )
    if not pattern.search(read_text(path)):
        errors.append(
            f"{record['id']}: bridge module "
            f"{path.relative_to(root).as_posix()} does not declare {short_name}"
        )


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
    default_module, bridge_module = module_state(root, record)
    bridge_only = bridge_module and not default_module
    p2_v1_exception = record["phase"] == "P2" and is_p2_v1_exception(root, record)
    if record["phase"] == "P2" and not p2_v1_exception:
        if record["level"] not in P2_LEVELS:
            errors.append(
                f"{entry}: Phase 2 records must stay at level S or V0, except the "
                f"owner-approved bridge theorem P2-T33"
            )
        if bridge_only:
            if record["kind"] != "evidence":
                errors.append(
                    f"{entry}: bridge-package Phase 2 records must have kind 'evidence', "
                    f"except the owner-approved theorem P2-T33"
                )
            if record["level"] != "V0":
                errors.append(
                    f"{entry}: bridge-package Phase 2 records must stay at level V0, "
                    f"except the owner-approved theorem P2-T33"
                )
            if record["status"] not in P2_BRIDGE_STATUSES:
                errors.append(
                    f"{entry}: bridge-package Phase 2 records must be 'planned' or "
                    f"'tested', not {record['status']!r}, except the owner-approved "
                    f"theorem P2-T33"
                )
    if p2_v1_exception:
        check_bridge_v1_declaration(root, record, errors)
    if record["phase"] == "P1":
        if record["kind"] != "evidence":
            errors.append(f"{entry}: Phase 1 records must have kind 'evidence'")
        if record["level"] != "V0":
            errors.append(f"{entry}: Phase 1 records must stay at level V0")
        if record["status"] not in P1_STATUSES:
            errors.append(
                f"{entry}: Phase 1 records must be 'planned' or 'tested', "
                f"not {record['status']!r}"
            )
    if record["status"] == "proved" and record["kind"] == "evidence":
        errors.append(f"{entry}: evidence cannot have status 'proved'; use 'tested'")
    if record["status"] == "tested" and record["kind"] != "evidence":
        errors.append(f"{entry}: status 'tested' is reserved for evidence entries")
    if record["status"] == "descoped" and not record.get("note"):
        errors.append(f"{entry}: status 'descoped' requires a note with the reason")
    check_module(root, record, errors)
    check_spec(root, record, errors)


def lean_audit(root: Path, records: list[dict], errors: list[str]) -> None:
    # The owner-approved bridge V1 record is exempt: the Aeneas/Mathlib package
    # is not part of the default environment that fast CI builds. Default-package
    # records, including every other proved Phase 2 record, keep the full audit.
    proved = [
        record
        for record in records
        if record["status"] == "proved" and not is_p2_v1_exception(root, record)
    ]
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
            encoding="utf-8",
            errors="replace",
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

    # Exactly one Phase 2 V1 record is accepted, and it must be the allowlisted
    # bridge-only exact-kernel theorem. This also catches a missing acceptance
    # record or a second V1 claim introduced around the per-record checks.
    p2_v1 = [
        record
        for record in records
        if isinstance(record, dict)
        and record.get("phase") == "P2"
        and record.get("level") == "V1"
    ]
    if len(p2_v1) != 1 or not is_p2_v1_exception(root, p2_v1[0]):
        errors.append(
            "Phase 2 must record exactly one V1 theorem: the owner-approved "
            "bridge exception P2-T33 (Stc.Bridge.Exact.exact_kernel_refines)"
        )

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
