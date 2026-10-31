#!/usr/bin/env bash
# WL-5.2 cycle-time sweep step runner (phase 5).
#
# Runs one open-ended CT sweep step against the deployed DIEWU fixture on
# ecat-host and exits 0 only when the step passes at frame level:
#
#   * the runtime reports state=running, faults=0, a complete domain working
#     counter, current (not stale) inputs, link up, one responding slave, no
#     master error, and no recovery;
#   * every completed cycle exchanged a frame (exchanges == cycles);
#   * the RT metrics summary reports zero skipped periods and the per-cycle
#     execution p99 is at most half the cycle time; the strict
#     missed_deadline counter is reported but not gated because the project
#     documents it as a non-hard counter (KNOWN-LIMITATIONS.md);
#   * the IgH master's Tx-error and lost-frame counters did not increase.
#
# Physical input values and measured outputs are not checked; phases 3-5 are
# open-ended (unwired/unloaded) and the all-points matrix is phase 6.
#
# This script is user-writable and is always executed as r2k by the
# privileged stc-wl52-run wrapper, which grants only CAP_SYS_NICE ambient.
# Run it through that wrapper:
#
#   sudo -n /usr/local/sbin/stc-wl52-run ct <cycle-time> <duration> <label>
#
# Exit status: 0 frame-level pass, 1 frame-level failure or run error,
#              2 usage or deployment error.

set -euo pipefail

readonly ROOT=/home/r2k/stc-wl52
readonly STC=$ROOT/bin/stc
readonly FIXTURE=$ROOT/diewu
readonly SHIM=$ROOT/lib/libstc_io_ethercat.so
readonly LOGS=$ROOT/logs
readonly ETHERCAT=/usr/local/bin/ethercat
readonly IGH_LIB_DIR=/usr/local/etherlab/lib

# The timeout covers the in-process release build plus the requested run
# window. The build is expected to take seconds; the allowance is a safety
# bound, not a second run time.
readonly BUILD_ALLOWANCE_SECONDS=60

log=
metrics=
exec_values=
wake_values=

cleanup() {
    [[ -z $exec_values ]] || rm -f "$exec_values"
    [[ -z $wake_values ]] || rm -f "$wake_values"
}
trap cleanup EXIT

usage() {
    cat <<'EOF'
usage: run-ct.sh <cycle-time> <duration-seconds> <label>

cycle-time:  1ms 500us 250us 200us 150us 100us 75us 50us 25us
duration:    60 or 600 seconds
label:       [a-z0-9-]{1,32}

The command and paths are fixed; run this script through
/usr/local/sbin/stc-wl52-run rather than directly.
EOF
}

fail() {
    printf 'run-ct.sh: %s\n' "$*" >&2
    if [[ -n $log && -f $log ]]; then
        printf -- '--- last log lines ---\n' >&2
        tail -n 20 "$log" >&2 || true
    fi
    exit 1
}

usage_error() {
    printf 'run-ct.sh: %s\n' "$*" >&2
    usage >&2
    exit 2
}

# Nearest-rank percentile of a sorted integer file: <file> <numerator> <denominator>.
percentile() {
    local file=$1 numerator=$2 denominator=$3 count rank
    count=$(wc -l <"$file")
    rank=$(( (numerator * count + denominator - 1) / denominator ))
    (( rank > 0 )) || rank=1
    (( rank <= count )) || rank=$count
    sed -n "${rank}p" "$file"
}

if [[ $# -ne 3 ]]; then
    usage_error "expected three arguments"
fi

cycle_time=$1
duration=$2
label=$3

case "$cycle_time" in
    1ms|500us|250us|200us|150us|100us|75us|50us|25us) ;;
    *) usage_error "unsupported cycle time: $cycle_time" ;;
esac

case "$duration" in
    60|600) ;;
    *) usage_error "unsupported duration: $duration (expected 60 or 600)" ;;
esac

[[ $label =~ ^[a-z0-9-]{1,32}$ ]] || usage_error "invalid label: $label"

[[ -x $STC ]] || usage_error "launcher not found or not executable: $STC"
[[ -d $FIXTURE ]] || usage_error "fixture directory not found: $FIXTURE"
[[ -f $FIXTURE/stc.toml ]] || usage_error "fixture manifest not found: $FIXTURE/stc.toml"
[[ -f $FIXTURE/io/bench.toml ]] || usage_error "fixture profile not found: $FIXTURE/io/bench.toml"
[[ -f $FIXTURE/src/main.st ]] || usage_error "fixture program not found: $FIXTURE/src/main.st"
[[ -f $SHIM ]] || usage_error "EtherCAT shim not found: $SHIM"
[[ -d $LOGS ]] || usage_error "log directory not found: $LOGS"
[[ -x $ETHERCAT ]] || usage_error "ethercat CLI not found: $ETHERCAT"

stamp=$(date -u +%Y%m%dT%H%M%SZ)
stem=ct-${cycle_time}-${duration}s-${label}-${stamp}
log=$LOGS/$stem.log
metrics=$LOGS/$stem.jsonl
master_before=$LOGS/$stem.master-before.txt
master_after=$LOGS/$stem.master-after.txt

export STC_ETHERCAT_LIBRARY=$SHIM
export LD_LIBRARY_PATH=$IGH_LIB_DIR

{
    printf 'run-ct: cycle=%s duration=%s label=%s\n' "$cycle_time" "$duration" "$label"
    printf 'run-ct: started=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'run-ct: host=%s\n' "$(uname -srm)"
    printf 'run-ct: stc=%s\n' "$("$STC" --version 2>&1 | head -n 1 || true)"
    printf 'run-ct: shim=%s\n' "$SHIM"
    printf 'run-ct: metrics=%s\n' "$metrics"
} >>"$log"

# Sample the master at the run boundaries. Never reset the counters.
"$ETHERCAT" master >"$master_before" 2>&1 \
    || fail "cannot sample the EtherCAT master before the run"
grep -q 'Phase: Idle' "$master_before" \
    || fail "the EtherCAT master is not Idle before the run; another application may own it"
"$ETHERCAT" slaves >"$LOGS/$stem.slaves-before.txt" 2>&1 || true

# `stc run` builds the release fixture and then drives the open-ended RT loop.
# A zero cycle count runs until the timeout sends SIGTERM; the launcher catches
# it, completes the current cycle, shuts down, and prints the final status.
set +e
timeout --signal=TERM --kill-after=30 --preserve-status \
    "$((duration + BUILD_ALLOWANCE_SECONDS))" \
    "$STC" run \
    --release \
    --manifest-path "$FIXTURE/stc.toml" \
    --io ethercat \
    --cycles 0 \
    --cycle-time "$cycle_time" \
    --rt \
    --rt-priority 90 \
    --rt-cpu 3 \
    --rt-metrics "$metrics" \
    -v 2>&1 | tee -a "$log"
run_status=${PIPESTATUS[0]}
set -e
printf 'run-ct: stc/timeout exit=%s\n' "$run_status" >>"$log"

"$ETHERCAT" master >"$master_after" 2>&1 \
    || fail "cannot sample the EtherCAT master after the run"

grep -q 'Phase: Idle' "$master_after" \
    || fail "the EtherCAT master is not Idle after the run"
grep -q 'Active: no' "$master_after" \
    || fail "the EtherCAT master is still active after the run"

status_line=$(grep -E '^stc: status: io-backend ethercat ' "$log" | tail -n 1 || true)
[[ -n $status_line ]] \
    || fail "no io-backend status line (stc/timeout exit $run_status); the run did not reach a healthy cycle"

field_is() {
    case " $status_line " in
        *" $1=$2 "*) return 0 ;;
        *) return 1 ;;
    esac
}

for expected in \
    state=running \
    faults=0 \
    domain.wc_state=2 \
    input.stale=0 \
    master.last_error=0 \
    master.link_up=1 \
    master.recoveries=0 \
    master.slaves_responding=1
do
    field_is "${expected%%=*}" "${expected#*=}" \
        || fail "status check failed: $expected in: $status_line"
done

exchanges=$(sed -n 's/.* exchanges=\([0-9][0-9]*\).*/\1/p' <<<"$status_line")
[[ -n $exchanges && $exchanges -gt 0 ]] || fail "status line reports no exchanges"

[[ -s $metrics ]] || fail "RT metrics file is missing or empty: $metrics"
summary=$(tail -n 1 "$metrics")
case "$summary" in
    *'"record":"summary"'*) ;;
    *) fail "RT metrics file has no summary record: $metrics" ;;
esac
case "$summary" in
    *'"skipped_periods":0'*) ;;
    *) fail "skipped periods in the RT summary: $summary" ;;
esac
strict_missed=$(sed -n 's/.*"missed_deadlines":\([0-9][0-9]*\).*/\1/p' <<<"$summary")
[[ -n $strict_missed ]] || fail "RT summary has no missed_deadlines count: $summary"

completed=$(sed -n 's/.*"cycles":\([0-9][0-9]*\).*/\1/p' <<<"$summary")
[[ -n $completed && $completed -gt 0 ]] || fail "RT summary reports no completed cycles"
[[ $exchanges -eq $completed ]] \
    || fail "exchanges ($exchanges) do not match completed cycles ($completed)"

# The timeout includes the in-process build, so a normal run exceeds the
# nominal window. Require at least the requested screening time; otherwise a
# slow build consumed the allowance and the step is not usable evidence.
case "$cycle_time" in
    1ms) period_ns=1000000 ;;
    500us) period_ns=500000 ;;
    250us) period_ns=250000 ;;
    200us) period_ns=200000 ;;
    150us) period_ns=150000 ;;
    100us) period_ns=100000 ;;
    75us) period_ns=75000 ;;
    50us) period_ns=50000 ;;
    25us) period_ns=25000 ;;
esac
expected_cycles=$(( duration * 1000000000 / period_ns ))
[[ $completed -ge $expected_cycles ]] \
    || fail "run covered $completed cycles; at least $expected_cycles are needed for ${duration}s at ${cycle_time}"

# Timing evidence from the per-cycle records. The strict missed_deadline
# counter is true for any positive wake lateness (KNOWN-LIMITATIONS.md); it is
# reported, not used as a hard-miss gate. A step fails on a hard miss: a
# skipped period, or an execution p99 that is not well below the cycle time
# (at most half the period, WL-5.2-Plan.md section 6.9).
exec_values=$(mktemp "${TMPDIR:-/tmp}/run-ct-exec.XXXXXX") \
    || fail "cannot create a temporary file for the execution distribution"
wake_values=$(mktemp "${TMPDIR:-/tmp}/run-ct-wake.XXXXXX") \
    || fail "cannot create a temporary file for the wake-lateness distribution"
sed -n 's/.*"execution_ns":\([0-9][0-9]*\).*/\1/p' "$metrics" >"$exec_values"
sed -n 's/.*"wake_lateness_ns":\(-\{0,1\}[0-9][0-9]*\).*/\1/p' "$metrics" >"$wake_values"
[[ -s $exec_values ]] || fail "no per-cycle execution records in $metrics"
[[ -s $wake_values ]] || fail "no per-cycle wake-lateness records in $metrics"
sort -n -o "$exec_values" "$exec_values"
sort -n -o "$wake_values" "$wake_values"

exec_p50=$(percentile "$exec_values" 50 100)
exec_p99=$(percentile "$exec_values" 99 100)
exec_p999=$(percentile "$exec_values" 999 1000)
exec_max=$(tail -n 1 "$exec_values")
wake_p50=$(percentile "$wake_values" 50 100)
wake_p99=$(percentile "$wake_values" 99 100)
wake_p999=$(percentile "$wake_values" 999 1000)
wake_max=$(tail -n 1 "$wake_values")

[[ $exec_p99 -le $(( period_ns / 2 )) ]] \
    || fail "execution p99 ${exec_p99}ns is not at most half of the ${cycle_time} cycle (${period_ns}ns); p50=${exec_p50}ns p99.9=${exec_p999}ns max=${exec_max}ns"

read_counter() {
    awk -v first="$2" -v second="$3" '$1 == first && $2 == second { print $3; exit }' "$1"
}

tx_errors_before=$(read_counter "$master_before" Tx errors:)
lost_frames_before=$(read_counter "$master_before" Lost frames:)
tx_errors_after=$(read_counter "$master_after" Tx errors:)
lost_frames_after=$(read_counter "$master_after" Lost frames:)

for value in "$tx_errors_before" "$lost_frames_before" "$tx_errors_after" "$lost_frames_after"; do
    [[ $value =~ ^[0-9]+$ ]] || fail "cannot parse the master's Tx-error/lost-frame counters"
done

[[ $tx_errors_after == "$tx_errors_before" ]] \
    || fail "master Tx errors changed: $tx_errors_before -> $tx_errors_after"
[[ $lost_frames_after == "$lost_frames_before" ]] \
    || fail "master lost frames changed: $lost_frames_before -> $lost_frames_after"

# The slave transitions to PREOP asynchronously after the master is released;
# poll briefly instead of depending on a single sample's timing.
slave_ok=0
for _ in {1..20}; do
    "$ETHERCAT" slaves >"$LOGS/$stem.slaves-after.txt" 2>&1 || true
    if grep -q 'PREOP' "$LOGS/$stem.slaves-after.txt"; then
        slave_ok=1
        break
    fi
    sleep 0.25
done
[[ $slave_ok == 1 ]] \
    || fail "the configured slave is not PREOP after the run: $(cat "$LOGS/$stem.slaves-after.txt" 2>/dev/null)"

printf 'run-ct: PASS cycle=%s duration=%s label=%s exchanges=%s cycles=%s skipped=0 strict_missed=%s\n' \
    "$cycle_time" "$duration" "$label" "$exchanges" "$completed" "$strict_missed"
printf 'run-ct: timing exec_ns p50=%s p99=%s p99.9=%s max=%s; wake_ns p50=%s p99=%s p99.9=%s max=%s\n' \
    "$exec_p50" "$exec_p99" "$exec_p999" "$exec_max" \
    "$wake_p50" "$wake_p99" "$wake_p999" "$wake_max"
printf 'run-ct: log=%s metrics=%s master=%s,%s\n' \
    "$log" "$metrics" "$master_before" "$master_after"
