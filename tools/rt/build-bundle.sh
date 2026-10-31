#!/usr/bin/env bash
# Builds the WL-4 rt-bench bundle from a packaged Linux stc driver.
#
# usage: tools/rt/build-bundle.sh <stc-package-dir> <output-dir>
#
# The script runs on a Linux build host (CI). It never runs on the RT target.
set -euo pipefail

usage() {
    printf 'usage: %s <stc-package-dir> <output-dir>\n' "$0" >&2
    exit 2
}

[[ $# -eq 2 ]] || usage

package=$(cd "$1" && pwd)
[[ -x "$package/bin/stc" ]] || {
    printf 'missing packaged driver: %s\n' "$package/bin/stc" >&2
    exit 2
}

root=$(cd "$(dirname "$0")/../.." && pwd)
template="$root/tests/rt/bench"
cases="$root/tests/rt/cases"
workloads="$root/tests/rt/workloads"

mkdir -p "$2"
out=$(cd "$2" && pwd)
case "$out" in
    / | "") printf 'refusing to use %s as the output directory\n' "$out" >&2; exit 2 ;;
esac

work=$(mktemp -d)
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

stc="$package/bin/stc"

build_variant() {
    local tag=$1 source=$2 emit=$3
    shift 3
    local stage="$work/stage-$tag"
    local outdir="$work/$tag"
    rm -rf "$stage" "$outdir"
    mkdir -p "$stage/src"
    cp "$template/stc.toml" "$template/stc.lock" "$stage/"
    cp "$source" "$stage/src/main.st"
    printf 'building %s -> %s\n' "$tag" "$outdir" >&2
    "$stc" build --manifest-path "$stage/stc.toml" --locked --emit="$emit" \
        --out-dir "$outdir" "$@"
}

# Base (medium workload) and the two cold-start workload variants.
build_variant base "$template/src/main.st" "exe,module,metadata"
build_variant light "$workloads/light/main.st" "exe,module,metadata"
build_variant heavy "$workloads/heavy/main.st" "exe,module,metadata"

base_manifest="$work/base/rt-bench.stc-got.json"
[[ -f "$base_manifest" ]] || {
    printf 'base manifest missing: %s\n' "$base_manifest" >&2
    exit 1
}

# Online-change candidates. The field add/remove needs the untracked-removal
# acknowledgement; the layout change cannot use the base and is built cold.
build_variant code-only "$cases/code-only/main.st" "module,metadata" \
    --got-manifest "$base_manifest"
build_variant field-add-remove "$cases/field-add-remove/main.st" "module,metadata" \
    --got-manifest "$base_manifest" --allow-untracked-removal
build_variant trapping "$cases/trapping/main.st" "module,metadata" \
    --got-manifest "$base_manifest"
build_variant incompatible "$cases/incompatible/main.st" "module,metadata" --cold

copy_runtime() {
    local src=$1 dst=$2
    mkdir -p "$dst"
    shopt -s nullglob
    local files=(
        "$src"/rt-bench
        "$src"/*.so
        "$src"/*.stc-got.json
        "$src"/*.stc-got.bin
        "$src"/*.stc-meta.json
        "$src"/*.stc-frames.json
    )
    shopt -u nullglob
    [[ ${#files[@]} -gt 0 ]] || {
        printf 'no runtime artifacts in %s\n' "$src" >&2
        exit 1
    }
    cp -a "${files[@]}" "$dst/"
}

copy_candidate() {
    local src=$1 dst=$2
    mkdir -p "$dst"
    shopt -s nullglob
    local files=(
        "$src"/*.so
        "$src"/*.stc-got.json
        "$src"/*.stc-got.bin
        "$src"/*.stc-meta.json
        "$src"/*.stc-frames.json
    )
    shopt -u nullglob
    [[ ${#files[@]} -gt 0 ]] || {
        printf 'no candidate artifacts in %s\n' "$src" >&2
        exit 1
    }
    cp -a "${files[@]}" "$dst/"
}

rm -rf "$out"
mkdir -p "$out"
copy_runtime "$work/base" "$out"
copy_runtime "$work/light" "$out/workloads/light"
copy_runtime "$work/heavy" "$out/workloads/heavy"
for name in code-only field-add-remove incompatible trapping; do
    copy_candidate "$work/$name" "$out/candidates/$name"
done

mkdir -p "$out/runtime"
if [[ -f "$package/lib/libstc_runtime.so" ]]; then
    cp -a "$package/lib/libstc_runtime.so" "$out/runtime/"
fi

commit=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || printf 'unknown')
version=$("$stc" --version 2>/dev/null || printf 'stc')
cat > "$out/README.md" <<EOF
# rt-bench — WL-4 benchmark bundle

Built from $commit with $version.

## Layout

- \`rt-bench\` — base launcher (medium workload)
- \`rt-bench-<build-id>.so\` — base module
- \`rt-bench.stc-got.json\` and \`rt-bench.stc-got.bin\` — base manifest
- \`workloads/light/\` and \`workloads/heavy/\` — alternative cold-start launchers
- \`candidates/<name>/\` — online-change candidates (module + manifest)
- \`runtime/libstc_runtime.so\` — runtime library, if the launcher needs it

## Run

\`\`\`sh
./rt-bench --cycles 3 --cycle-time 1ms --status
\`\`\`

If \`ldd rt-bench\` reports \`libstc_runtime.so\` as missing, run with
\`LD_LIBRARY_PATH="\$PWD/runtime:\${LD_LIBRARY_PATH:-}"\`.

The trapping candidate adds a no-interval task, so it traps on the first scan
after activation. Deploy candidates over the local control channel; each
candidate directory contains its module and manifest paths.
EOF

(
    cd "$out"
    find . -type f ! -name SHA256SUMS -printf '%P\n' | sort | while IFS= read -r file; do
        sha256sum "$file"
    done > SHA256SUMS
)

printf 'bundle: %s\n' "$out"
cat "$out/SHA256SUMS"
