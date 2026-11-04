#!/usr/bin/env python3
"""Validate the pinned Aeneas/Charon bridge toolchain record.

Usage:
    python tools/ci/check_bridge_toolchain.py [repository-root]
    python tools/ci/check_bridge_toolchain.py --check-environment
    python tools/ci/check_bridge_toolchain.py --require-generated

Checks:
  * `lean/bridge-toolchain.toml` parses and uses the expected schema/profile;
  * the Lean and Rust pins match `lean/lean-toolchain` and `rust-toolchain.toml`,
    and `semantics_version` matches the verification registry;
  * every bridge source exists and matches its recorded SHA-256;
  * when a generated model declared in the record is checked in, its header
    names the pinned Aeneas/Charon versions, the source path, and the source
    SHA-256;
  * with `--require-generated`, each declared generated model must be present;
  * with `--check-environment`, the installed Lean and Rust versions match the
    pins, and the Aeneas/Charon binaries are present (`AENEAS` and `CHARON`
    may override the PATH lookup). Their version output is reported, not
    parsed strictly because the pinned builds do not have a recorded format.

This is the drift guard for the manual/nightly `bridge-drift` workflow. The
fast CI job never installs Aeneas or Mathlib and does not run this script. Only
the Python standard library is required; `--check-environment` also needs the
pinned tools on PATH.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

SCHEMA_VERSION = 1
PROFILE = "stc-2013-oop-utf8-v1"
BRIDGE_RECORD = Path("lean/bridge-toolchain.toml")
LEAN_PIN = Path("lean/lean-toolchain")
RUST_PIN = Path("rust-toolchain.toml")
REGISTRY = Path("lean/verification-status.toml")
REQUIRED_TARGETS = ("exact", "keys", "wire")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
GENERATED_HEADER_LINES = 80


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_table(
    data: dict, key: str, fields: tuple[str, ...], errors: list[str]
) -> dict | None:
    table = data.get(key)
    if not isinstance(table, dict):
        errors.append(f"{key} must be a table")
        return None
    for field in fields:
        value = table.get(field)
        if not isinstance(value, str) or not value.strip():
            errors.append(f"{key}.{field} must be a non-empty string")
    return table


def check_pins(root: Path, data: dict, errors: list[str]) -> None:
    if data.get("schema_version") != SCHEMA_VERSION:
        errors.append(f"schema_version must be {SCHEMA_VERSION}")
    if data.get("profile") != PROFILE:
        errors.append(f"profile must be {PROFILE!r}")
    if data.get("semantics_version") != "1.0":
        errors.append("semantics_version must be '1.0'")

    lean = require_table(data, "lean", ("toolchain", "recorded_in"), errors)
    if lean is not None:
        lean_pin = root / LEAN_PIN
        if not lean_pin.is_file():
            errors.append(f"{LEAN_PIN.as_posix()} is missing")
        elif lean.get("toolchain") != read_text(lean_pin).strip():
            errors.append(
                f"lean.toolchain {lean.get('toolchain')!r} does not match "
                f"{LEAN_PIN.as_posix()} {read_text(lean_pin).strip()!r}"
            )

    rust = require_table(data, "rust", ("channel", "recorded_in"), errors)
    if rust is not None:
        rust_pin = root / RUST_PIN
        if not rust_pin.is_file():
            errors.append(f"{RUST_PIN.as_posix()} is missing")
        else:
            try:
                pin = tomllib.loads(read_text(rust_pin))
            except tomllib.TOMLDecodeError as error:
                errors.append(f"cannot parse {RUST_PIN.as_posix()}: {error}")
            else:
                channel = pin.get("toolchain", {}).get("channel")
                if channel != rust.get("channel"):
                    errors.append(
                        f"rust.channel {rust.get('channel')!r} does not match "
                        f"{RUST_PIN.as_posix()} {channel!r}"
                    )

    mathlib = require_table(data, "mathlib", ("version", "role", "package"), errors)
    if mathlib is not None:
        if mathlib.get("role") != "proof-only":
            errors.append("mathlib.role must be 'proof-only'")
        if mathlib.get("package") != "lean/aeneas":
            errors.append("mathlib.package must be 'lean/aeneas'")

    aeneas = require_table(
        data, "aeneas", ("version", "commit", "backend"), errors
    )
    if aeneas is not None and aeneas.get("backend") != "lean":
        errors.append("aeneas.backend must be 'lean'")

    charon = require_table(data, "charon", ("version", "commit", "preset"), errors)
    if charon is not None and charon.get("preset") != "aeneas":
        errors.append("charon.preset must be 'aeneas'")

    require_table(data, "ocaml", ("opam", "compiler", "gmp"), errors)

    layout = require_table(
        data,
        "layout",
        (
            "default_package",
            "bridge_package",
            "generated_root",
            "handwritten_bridge_root",
            "handwritten_proof_root",
        ),
        errors,
    )
    if layout is not None:
        if layout.get("default_package") != "lean":
            errors.append("layout.default_package must be 'lean'")
        if layout.get("bridge_package") != "lean/aeneas":
            errors.append("layout.bridge_package must be 'lean/aeneas'")

    generation = require_table(data, "generation", ("boundary",), errors)
    if generation is not None and "never in fast CI" not in generation.get(
        "boundary", ""
    ):
        errors.append("generation.boundary must state that regeneration never runs in fast CI")

    registry_path = root / REGISTRY
    if registry_path.is_file():
        try:
            registry = tomllib.loads(read_text(registry_path))
        except tomllib.TOMLDecodeError as error:
            errors.append(f"cannot parse {REGISTRY.as_posix()}: {error}")
        else:
            if registry.get("semantics_version") != data.get("semantics_version"):
                errors.append(
                    "semantics_version does not match the verification registry"
                )


def check_sources(
    root: Path,
    data: dict,
    require_generated: bool,
    errors: list[str],
    notices: list[str],
) -> tuple[int, int]:
    sources = data.get("sources")
    if not isinstance(sources, dict) or not sources:
        errors.append("sources must be a non-empty table")
        return (0, 0)

    missing_targets = [target for target in REQUIRED_TARGETS if target not in sources]
    if missing_targets:
        errors.append(
            "sources is missing target(s): " + ", ".join(missing_targets)
        )

    checked = 0
    checked_in = 0
    for target, source in sources.items():
        entry = f"sources.{target}"
        if not isinstance(source, dict):
            errors.append(f"{entry} must be a table")
            continue

        path_value = source.get("path")
        hash_value = source.get("sha256")
        generated = source.get("generated")
        generated_path = source.get("generated_path", "")

        if not isinstance(path_value, str) or not path_value.strip():
            errors.append(f"{entry}.path must be a non-empty string")
            continue
        if not isinstance(hash_value, str) or not SHA256_RE.fullmatch(hash_value):
            errors.append(f"{entry}.sha256 must be 64 lowercase hex characters")
            continue
        if not isinstance(generated, bool):
            errors.append(f"{entry}.generated must be a boolean")
            continue

        source_path = root / path_value
        if not source_path.is_file():
            errors.append(f"{entry}: source is missing: {path_value}")
            continue
        actual = sha256_file(source_path)
        if actual != hash_value:
            errors.append(
                f"{entry}: source hash drift: {path_value} is {actual}, "
                f"record says {hash_value}"
            )
            continue
        checked += 1

        if not generated:
            continue
        if not isinstance(generated_path, str) or not generated_path.strip():
            errors.append(f"{entry}.generated_path must be a non-empty string")
            continue
        generated_path = Path(generated_path)
        if generated_path.is_absolute() or ".." in generated_path.parts:
            errors.append(f"{entry}.generated_path must stay inside the repository")
            continue
        generated_file = root / generated_path
        if not generated_file.is_file():
            message = (
                f"{entry}: generated model is not checked in: "
                f"{generated_path.as_posix()}"
            )
            if require_generated:
                errors.append(message)
            else:
                notices.append(message)
            continue
        checked_in += 1

        with generated_file.open("r", encoding="utf-8-sig") as handle:
            header = "\n".join(
                handle.read().splitlines()[:GENERATED_HEADER_LINES]
            )
        required = [
            path_value,
            hash_value,
            data.get("aeneas", {}).get("version", ""),
            data.get("charon", {}).get("version", ""),
        ]
        absent = [item for item in required if item and item not in header]
        if absent:
            errors.append(
                f"{entry}: generated header for {generated_path.as_posix()} "
                f"does not name: {', '.join(absent)}"
            )
        elif not re.search(r"generated", header, re.IGNORECASE):
            errors.append(
                f"{entry}: generated header for {generated_path.as_posix()} "
                "does not mark the file as generated"
            )
    return (checked, checked_in)


def find_tool(name: str, override: str | None) -> str | None:
    if override:
        return override
    return shutil.which(name)


def tool_output(tool: str, *arguments: str) -> str:
    completed = subprocess.run(
        [tool, *arguments],
        capture_output=True,
        text=True,
        check=False,
    )
    return (completed.stdout + completed.stderr).strip()


def check_environment(
    data: dict, errors: list[str], notices: list[str]
) -> None:
    lean_pin = data.get("lean", {}).get("toolchain", "")
    lean_version = lean_pin.rsplit(":", 1)[-1].lstrip("v")
    rust_channel = data.get("rust", {}).get("channel", "")

    lean = find_tool("lean", None)
    if lean is None:
        errors.append("lean is not on PATH; cannot check the pinned environment")
    else:
        output = tool_output(lean, "--version")
        if lean_version not in output:
            errors.append(
                f"lean --version does not report pinned {lean_version}: {output!r}"
            )

    lake = find_tool("lake", None)
    if lake is None:
        errors.append("lake is not on PATH; cannot check the pinned environment")

    rustc = find_tool("rustc", None)
    if rustc is None:
        errors.append("rustc is not on PATH; cannot check the pinned environment")
    else:
        output = tool_output(rustc, "--version")
        if rust_channel not in output:
            errors.append(
                f"rustc --version does not report pinned {rust_channel}: {output!r}"
            )

    for name, override, pin_key in (
        ("aeneas", os.environ.get("AENEAS"), "aeneas"),
        ("charon", os.environ.get("CHARON"), "charon"),
    ):
        tool = find_tool(name, override)
        if tool is None:
            errors.append(
                f"{name} is not on PATH (set {name.upper()} to override); "
                "cannot check the pinned environment"
            )
            continue
        output = tool_output(tool, "--version")
        pinned = data.get(pin_key, {})
        markers = [pinned.get("version", ""), pinned.get("commit", "")]
        confirmed = any(marker and marker in output for marker in markers)
        if confirmed:
            notices.append(f"{name} reports the pinned build: {output!r}")
        else:
            notices.append(
                f"{name} version output does not name the pinned "
                f"{pinned.get('version')}/{pinned.get('commit')}; confirm the "
                f"installed build manually: {output!r}"
            )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repository root (default: the repository containing this script)",
    )
    parser.add_argument(
        "--check-environment",
        action="store_true",
        help="also compare the installed Lean/Rust/Aeneas/Charon tools to the pins",
    )
    parser.add_argument(
        "--require-generated",
        action="store_true",
        help="fail when a declared generated model is not checked in",
    )
    args = parser.parse_args(argv[1:])
    root = args.root.resolve()

    record_path = root / BRIDGE_RECORD
    if not record_path.is_file():
        print(
            f"check_bridge_toolchain: missing record: {BRIDGE_RECORD.as_posix()}",
            file=sys.stderr,
        )
        return 2
    try:
        data = tomllib.loads(read_text(record_path))
    except tomllib.TOMLDecodeError as error:
        print(
            f"check_bridge_toolchain: cannot parse "
            f"{BRIDGE_RECORD.as_posix()}: {error}",
            file=sys.stderr,
        )
        return 2

    errors: list[str] = []
    notices: list[str] = []
    check_pins(root, data, errors)
    checked, checked_in = check_sources(
        root, data, args.require_generated, errors, notices
    )
    if args.check_environment:
        check_environment(data, errors, notices)

    for notice in notices:
        print(f"  note: {notice}")
    if errors:
        print(f"check_bridge_toolchain: {len(errors)} problem(s)")
        for error in errors:
            print(f"  {error}")
        return 1

    print(
        f"check_bridge_toolchain: OK ({checked} source pin(s) match; "
        f"{checked_in} generated model(s) checked in; "
        f"Aeneas {data.get('aeneas', {}).get('version')}, "
        f"Charon {data.get('charon', {}).get('version')})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
