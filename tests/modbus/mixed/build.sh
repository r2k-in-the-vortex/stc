#!/usr/bin/env bash
# Builds the mixed DIEWU EtherCAT + Modbus TCP bench fixture with the deployed
# stc package. Do not rebuild stc or the runtime on ecat-host.
#
# Extra arguments are passed to `stc build`. The packaged stc finds the
# sibling lib/libstc_runtime.a automatically.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}

if ! command -v "$STC" >/dev/null 2>&1; then
  echo "build.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi

exec "$STC" build --manifest-path "$here/stc.toml" "$@"
