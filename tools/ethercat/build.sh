#!/usr/bin/env sh
# Builds the project-owned EtherCAT shim against an installed IgH 1.6.x
# userspace library.
#
# Usage:
#   sh tools/ethercat/build.sh OUTPUT [INCLUDE_DIR [LIB_DIR]]
#
# INCLUDE_DIR defaults to /usr/local/include and LIB_DIR to /usr/local/lib.
# Both must contain the IgH userspace installation (ecrt.h and
# libethercat.so). The base Rust workspace never links libethercat; only this
# optional shared object does.
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 OUTPUT [INCLUDE_DIR [LIB_DIR]]" >&2
    exit 2
fi

out=$1
include_dir=${2:-/usr/local/include}
lib_dir=${3:-/usr/local/lib}
cc=${CC:-cc}

if [ ! -f "$include_dir/ecrt.h" ]; then
    echo "build.sh: $include_dir/ecrt.h not found; build and install IgH EtherCAT 1.6.13 userspace first" >&2
    exit 2
fi
if [ ! -e "$lib_dir/libethercat.so" ]; then
    echo "build.sh: $lib_dir/libethercat.so not found; build and install IgH EtherCAT 1.6.13 userspace first" >&2
    exit 2
fi

mkdir -p "$(dirname "$out")"

# shellcheck disable=SC2086
$cc -std=c99 -O2 -fPIC -Wall -Wextra -Werror \
    -shared -Wl,--no-undefined \
    -I"$include_dir" \
    "$(dirname "$0")/stc_ec_shim.c" \
    -L"$lib_dir" -lethercat \
    -o "$out"

echo "built $out"
