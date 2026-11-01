#!/usr/bin/env bash
# Runs the DIEWU loopback all-points test on ecat-host.
#
# The script builds the project with the deployed stc package, then runs the
# Python harness in `stc debug` launch mode and reads the result through
# var.read. It uses the EtherCAT backend, no root, and no --rt. The harness
# exits 0 only when all 16 channels passed and the fieldbus stayed healthy.
#
# Required environment:
#   STC_ETHERCAT_LIBRARY   path to the CI-built libstc_io_ethercat.so
# Optional environment:
#   STC                    path to the stc binary (default: stc from PATH)
#
# Usage:
#   bash run-loopback.sh [extra loopback_test.py options]
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
STC=${STC:-stc}

if [[ -z ${STC_ETHERCAT_LIBRARY:-} ]]; then
  echo "run-loopback.sh: set STC_ETHERCAT_LIBRARY to the CI-built shim" >&2
  exit 2
fi
if [[ ! -f $STC_ETHERCAT_LIBRARY ]]; then
  echo "run-loopback.sh: shim not found: $STC_ETHERCAT_LIBRARY" >&2
  exit 2
fi
if ! command -v "$STC" >/dev/null 2>&1; then
  echo "run-loopback.sh: stc not found; put the CI package's bin/ on PATH or set STC" >&2
  exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "run-loopback.sh: python3 not found; install it or run loopback_test.py manually" >&2
  exit 2
fi

echo "stc: $("$STC" --version)"
echo "shim: $STC_ETHERCAT_LIBRARY"

# Build first so a compile error is reported once, in human-readable form,
# before the harness opens the debug channel.
"$STC" build --manifest-path "$here/stc.toml"

exec python3 "$here/loopback_test.py" \
  --stc "$STC" \
  --manifest "$here/stc.toml" \
  "$@"
