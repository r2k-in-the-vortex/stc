#!/usr/bin/env bash
# Runs the Modbus TCP bench fixture and serves the two profile binds.
#
# Default: build with stc if needed and run until interrupted at 1 ms. The
# script prints the verbose backend status on exit, which reports the
# per-server counters (`server0.*`) and per-listener state.
#
# Environment:
#   STC         path to the stc binary (default: stc from PATH)
#   CYCLES      cycle count, default 0 (run until interrupted)
#   CYCLE_TIME  cycle time, default from stc.toml (1ms)
#
# In another terminal, run tests/modbus/client.py against
# 127.0.0.1:15020 and 127.0.0.1:15021 (the two configured binds). To use
# different ports, edit io/bench.toml; the Rust end-to-end test does this
# automatically in a temporary copy.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}
cycles=${CYCLES:-0}

if ! command -v "$STC" >/dev/null 2>&1; then
  echo "run.sh: stc not found; put stc on PATH or set STC" >&2
  exit 2
fi

echo "stc: $("$STC" --version)" >&2
echo "cycles: $cycles" >&2

args=(run --manifest-path "$here/stc.toml" --cycles "$cycles" -v)
if [[ -n ${CYCLE_TIME:-} ]]; then
  args+=(--cycle-time "$CYCLE_TIME")
fi

exec "$STC" "${args[@]}"
