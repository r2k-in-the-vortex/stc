#!/usr/bin/env bash
# Runs the mixed DIEWU EtherCAT + Modbus TCP bench fixture.
#
# Both hardware backends are served by one coordinator:
#   * the DIEWU slave is opened through the CI-built EtherCAT shim;
#   * the Modbus TCP server listens on 0.0.0.0:15030 for the network client.
#
# Required environment:
#   STC_ETHERCAT_LIBRARY   path to the CI-built libstc_io_ethercat.so
# Optional environment:
#   STC                    path to the stc binary (default: stc from PATH)
#   CYCLES                 cycle count, default 60000 (60 s at 1 ms)
#   CYCLE_TIME             cycle time, default 1ms
#   LD_LIBRARY_PATH        /usr/local/etherlab/lib is prepended when unset
#
# Run client.py from the bench workstation against <host>:15030 while this
# script is running. The no-cross-wiring limitation and the expected status
# are documented in README.md; recorded evidence is in RESULTS.md.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}
cycles=${CYCLES:-60000}
cycle_time=${CYCLE_TIME:-1ms}

if [[ -z ${STC_ETHERCAT_LIBRARY:-} ]]; then
  echo "run.sh: set STC_ETHERCAT_LIBRARY to the CI-built shim" >&2
  exit 2
fi
if [[ ! -f $STC_ETHERCAT_LIBRARY ]]; then
  echo "run.sh: shim not found: $STC_ETHERCAT_LIBRARY" >&2
  exit 2
fi
if ! command -v "$STC" >/dev/null 2>&1; then
  echo "run.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi

export LD_LIBRARY_PATH="/usr/local/etherlab/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

echo "stc: $("$STC" --version)"
echo "shim: $STC_ETHERCAT_LIBRARY"
echo "cycles: $cycles cycle-time: $cycle_time"

# `--io ethercat` keeps the established global default for unbound image
# areas; both device kinds still dispatch through the one coordinator.
exec "$STC" run \
  --manifest-path "$here/stc.toml" \
  --io ethercat \
  --cycles "$cycles" \
  --cycle-time "$cycle_time" \
  -v
