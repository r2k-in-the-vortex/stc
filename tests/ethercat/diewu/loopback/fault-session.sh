#!/usr/bin/env bash
# Opens the interactive loopback debug session used by the phase-6 physical
# fault procedure in ../PHASE6.md.
#
# The write scope is enabled only so the procedure can set the Hold
# fault-observation flag and read the held output/input masks. The session
# does not use root or --rt. Type commands at the `stc debug` prompt; the
# procedure lists them in order.
#
# Required environment:
#   STC_ETHERCAT_LIBRARY   path to the CI-built libstc_io_ethercat.so
# Optional environment:
#   STC                    path to the stc binary (default: stc from PATH)
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}

if [[ -z ${STC_ETHERCAT_LIBRARY:-} ]]; then
  echo "fault-session.sh: set STC_ETHERCAT_LIBRARY to the CI-built shim" >&2
  exit 2
fi
if [[ ! -f $STC_ETHERCAT_LIBRARY ]]; then
  echo "fault-session.sh: shim not found: $STC_ETHERCAT_LIBRARY" >&2
  exit 2
fi
if ! command -v "$STC" >/dev/null 2>&1; then
  echo "fault-session.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi

exec "$STC" debug \
  --manifest-path "$here/stc.toml" \
  --io ethercat \
  --cycles 0 \
  --cycle-time 1ms \
  --allow-write
