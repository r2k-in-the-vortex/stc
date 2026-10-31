#!/usr/bin/env bash
# Builds the DIEWU open-ended HIL fixture with the deployed stc package.
#
# Use the CI-built `stc` package (bin/stc); do not rebuild the compiler/runtime
# on the EtherCAT host. Extra arguments are passed to `stc build`.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}

if ! command -v "$STC" >/dev/null 2>&1; then
  echo "build.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi

exec "$STC" build --manifest-path "$here/stc.toml" "$@"
