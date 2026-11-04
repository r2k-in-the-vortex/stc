#!/usr/bin/env bash
# Builds the Modbus TCP bench fixture with an existing stc binary.
#
# `stc build` compiles the ST program with clang and writes the module and
# metadata under target/. Extra arguments are passed to `stc build`.
#
# Required: a stc binary on PATH or STC=/path/to/stc. In the Modbus fixture
# this may be a local `cargo run -p stc --` build or the CI package's bin/stc.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}

if ! command -v "$STC" >/dev/null 2>&1; then
  echo "build.sh: stc not found; put stc on PATH or set STC" >&2
  exit 2
fi

exec "$STC" build --manifest-path "$here/stc.toml" "$@"
