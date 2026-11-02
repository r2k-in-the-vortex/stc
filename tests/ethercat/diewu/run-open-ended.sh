#!/usr/bin/env bash
# Runs the 10-minute, 1 ms open-ended I/O HIL procedure described in
# README.md. Frame-level success only: the run does not assert physical input
# values or measured outputs.
#
# Required environment:
#   STC_ETHERCAT_LIBRARY   path to the CI-built libstc_io_ethercat.so
# Optional environment:
#   STC                    path to the stc binary (default: stc from PATH)
#   CYCLES                 cycle count, default 600000 (10 min at 1 ms)
#   CYCLE_TIME             cycle time, default 1ms
#   LD_LIBRARY_PATH        include /usr/local/etherlab/lib when IgH lives there
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}
cycles=${CYCLES:-600000}
cycle_time=${CYCLE_TIME:-1ms}

if [[ -z ${STC_ETHERCAT_LIBRARY:-} ]]; then
  echo "run-open-ended.sh: set STC_ETHERCAT_LIBRARY to the CI-built shim" >&2
  exit 2
fi
if [[ ! -f $STC_ETHERCAT_LIBRARY ]]; then
  echo "run-open-ended.sh: shim not found: $STC_ETHERCAT_LIBRARY" >&2
  exit 2
fi
if ! command -v "$STC" >/dev/null 2>&1; then
  echo "run-open-ended.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi

echo "stc: $("$STC" --version)"
echo "shim: $STC_ETHERCAT_LIBRARY"
echo "cycles: $cycles cycle-time: $cycle_time"

# No --rt: phase 3 checks frame acceptance and replies on an open-ended
# (unwired/unloaded) bench, not real-time scheduling.
exec "$STC" run \
  --manifest-path "$here/stc.toml" \
  --io ethercat \
  --cycles "$cycles" \
  --cycle-time "$cycle_time" \
  -v
