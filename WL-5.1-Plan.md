# WL-5.1 Plan — Modbus TCP server I/O (completed)

Profile: `stc-2013-oop-utf8-v1`  
Date: 2026-11-01; completed 2026-11-02  
Branch: `WL-5.1` (sub-plan of `v2.0.0-planning`)  
Status: Done 2026-11-02. Phases 0–5 are complete; the implementation, documentation, overhead
evidence, and pull request are the phase-5 result. The permanent contract is in
[I/O Routing and Backends §4.5](specification/Architecture/IO-Routing-and-Backends.md) and
[CLI and Manifest §2.2.1](specification/Architecture/CLI-and-Manifest.md); the evidence report is
[Modbus-Server.md](specification/Verification/Modbus-Server.md). This working plan is retained
until the pull request merges and is retired afterwards.  
Parent: [V2.0.0-PLAN.md](V2.0.0-PLAN.md) §4 (WL-5.1)  
Baseline: `WL-5.1` at `a25bf69` (merge of WL-5.2 into `v2.0.0-planning`); the WL-5.2 working plan
is retired and its decisions are D-211–D-228.

## 1. Result

The Modbus TCP server backend is implemented in
`crates/stc-runtime/src/io_modbus_tcp.rs`, registered as the `modbus-tcp` and
`modbus-tcp-status` kinds, and integrated with the multi-backend coordinator unchanged since
WL-5.2. A device exposes mapped process-image channels as coils, discrete inputs, holding
registers, and input registers; each server device may listen on several explicit addresses, all
sharing one register bank and mapping. Client writes become PLC inputs at the next input
boundary; PLC outputs become client-visible after the publish boundary. The backend is
non-real-time, best-effort, and has no authentication or encryption.

The implementation was delivered in five phases with green cross-host CI: protocol core and
register bank, coordinator integration, error/lifecycle/online-change hardening, cross-host
end-to-end and real mixed-bench evidence, and overhead/documentation/PR consolidation. The
function surface is FC1/2/3/4/5/6/15/16 with exceptions 0x01/0x02/0x03/0x04/0x0B; supported
widths are 1 bit on the bit tables and 8/16/32/64 bits on the register tables, with zero-based
addressing, low/high byte placement, device-level word order, and per-channel named or indexed
permutations. Known limits are recorded in
[KNOWN-LIMITATIONS.md §10](KNOWN-LIMITATIONS.md).

## 2. Scope as implemented

In scope and delivered:

- Pure-`std` server backend with MBAP/PDU codec, register bank, explicit channel mapping,
  listener per bind, connection thread per client, and mutex-based sharing with the cycle thread.
- Profile schema for `bind`, `unit`, `max-connections`, `idle-timeout`, device `word-order`, and
  per-channel `table`/`address`/`byte`/`word-order`; structural validation in the manifest.
- Deterministic cycle-boundary latch/publish, all-or-nothing cold start, per-listener runtime
  fault isolation and retry, optional `modbus-tcp-status` layout-1 device, and verbose diagnostics.
- Unit, integration, error-path, concurrency, project-mode, cross-host, and end-to-end evidence;
  in-repo client/simulator; real mixed EtherCAT + Modbus TCP bench; release overhead harness.
- The follow-on documentation consolidation, decision recording, plan cleanup, and PR.

Out of scope and not delivered: Modbus TCP client/master (WL-11), Modbus RTU, TLS,
authentication, arbitrary encodings, derived addresses, `%M` routes, hard real-time or safety
claims, and a new global backend selector.

## 3. Decisions index

All decisions were accepted by the owner on 2026-11-02 and are recorded as D-229–D-243 in
[Decisions.md §2](specification/Decisions.md). The full normative text and verification notes are
there; this index is for navigation only.

| Plan ID | Decision | Subject | Status |
| --- | --- | --- | --- |
| M-1 | D-229 | Backend module, `modbus-tcp`/`modbus-tcp-status` kinds, one instance, no coordinator or dependency change | Implemented and verified |
| M-2 | D-230 | Explicit zero-based profile channel map; spans; no derivation from image offsets | Implemented and verified |
| M-3 | D-231 | Table/direction/width rules, byte placement, register spans, partial-wider-write rejection | Implemented and verified |
| M-4 | D-232 | Profile schema for binds, unit, connection limit, idle timeout, word order, channel entries | Implemented and verified |
| M-5 | D-233 | Listener per bind, connection threads, cycle-boundary visibility, atomic multi-writes | Implemented and verified |
| M-6 | D-234 | FC1/2/3/4/5/6/15/16, quantity limits, exception behavior, framing closure rules | Implemented and verified |
| M-7 | D-235 | Register endianness, low/high byte, device word order, per-channel named/index permutations | Implemented and verified |
| M-8 | D-236 | Connection limit across binds, idle and socket timeouts, per-connection isolation | Implemented and verified |
| M-9 | D-237 | All-or-nothing cold start; per-listener fault isolation and retry; no stale-input flag | Implemented and verified |
| M-10 | D-238 | Verbose diagnostics and optional layout-1 status device | Implemented and verified; control JSON gap recorded |
| M-11 | D-239 | Diagnostic codes `STC-E-IO-MODTCP-001`/`002`/`003`, `STC-W-IO-MODTCP-001`/`002` | Implemented and verified; warning allowlisted |
| M-12 | D-240 | Online change preserves an unchanged backend and requires a cold download for map changes | Implemented and verified |
| M-13 | D-241 | Device `kind` only; no global `[io].backend`, `--io`, `[test] io`, or host-ABI spelling | Implemented and verified |
| M-14 | D-242 | Cross-host and e2e evidence, mixed mock and real bench, overhead, no CI timing gate | Implemented and verified; independent client not run |
| M-15 | D-243 | No runtime bind override; reconcile the documented CLI and control-status gaps | Documentation corrected; no code change |

## 4. Evidence

- **Contract:** [IO-Routing-and-Backends §4.5](specification/Architecture/IO-Routing-and-Backends.md)
  (profile schema, zero-based mapping, widths/orders, scheduling, fault policy, status,
  diagnostics, security, online change) and
  [CLI-and-Manifest §2.2.1](specification/Architecture/CLI-and-Manifest.md) (profile schema and
  zero-based address convention).
- **Report:** [Modbus-Server.md](specification/Verification/Modbus-Server.md) — result,
  environment, architecture, addressing, mapping, scheduling/fault/status, protocol and error
  coverage, cross-host CI, fixture, real mixed bench, overhead, limitations, and evidence index.
- **Code:** `crates/stc-runtime/src/io_modbus_tcp.rs`,
  `crates/stc-runtime/src/backend_defaults.rs`, and `crates/stc-manifest/src/profile.rs`; the
  release overhead test is `crates/stc-runtime/tests/modbus_overhead.rs`.
- **Fixture:** [tests/modbus/README.md](tests/modbus/README.md) with `stc.toml`, multi-bind
  profile, ST program, client, and run scripts. The reusable Rust test client lives with the
  runtime integration tests and is what CI drives.
- **Mixed bench:** [tests/modbus/mixed/RESULTS.md](tests/modbus/mixed/RESULTS.md) — real
  concurrent EtherCAT + Modbus TCP run; no external Modbus↔EtherCAT cross-wiring is claimed.
- **Overhead:** [tests/modbus/overhead/windows-x86_64-2026-11-02.txt](tests/modbus/overhead/windows-x86_64-2026-11-02.txt) —
  three release runs with 0/1/4/8 clients, latch/publish/cycle distributions, request latency and
  throughput, and process CPU. Host-specific, not a real-time claim.
- **Cross-host CI:** runs 41473961570, 41478485216, 41483247583, 41488899175, and 41493523376
  cover phases 0–4 on Windows x86-64, Linux x86-64, and Linux AArch64. The phase-5 run is listed
  in §5.
- **No RT or safety claim.** The report and known limitations state the non-real-time,
  best-effort, unauthenticated boundary and what the evidence does not prove.

## 5. Work breakdown and phase status

| Phase | Work | Acceptance | Size |
| --- | --- | --- | --- |
| 0 | Lock the contracts: accept or amend M-1–M-15; record the profile schema including the bind array, the width/span/byte/word-order matrix, the zero-based addressing convention, protocol surface, scheduling model, connection/fault policy, status layout, diagnostic codes, and fixture design | Decisions recorded; schema and test matrix reviewed; no implementation started before the mapping and scheduling contracts are settled | S |
| 1 | Protocol core and register bank: MBAP/PDU codec, FC1/2/3/4/5/6/15/16, exceptions, table/address maps, 1/8/16/32/64-bit values, register spans, byte placement, device-level word order and per-channel permutation overrides, channel validation, bank, conversion, **zero-based address vectors**. Unit and property tests; no sockets | Codec and bank tests pass on all three hosts; malformed/truncated inputs cannot panic; table/direction/width/span rules, 8-bit low/high placement, device defaults, per-channel permutation orders, and `address 0 ↔ 00001/10001/30001/40001` vectors are covered; no socket code yet | M |
| 2 | Backend, profile, and coordinator integration: new `io_modbus_tcp` module; `register_modbus_tcp`; profile parser keys, bind-array and device/per-channel word-order validation; `cold_start`/`latch_inputs`/`publish_outputs`/`status`/`diagnostics`/`shutdown`; listener per bind and connection threads; `modbus-tcp-status`; loopback integration tests; mixed Modbus + mock EtherCAT test | `kind = "modbus-tcp"` selects the backend end to end; a real loopback client writes inputs visible at the next latch and reads outputs published after the scan; all four tables and all five widths work; a server with two binds serves both from one bank; status and diagnostics appear; the coordinator serves Modbus and the mock EtherCAT over one image; existing null/memory/file and EtherCAT tests are unchanged | M |
| 3 | Error paths, limits, lifecycle, and online change: malformed MBAP/PDU, protocol id, length, oversized/truncated frames, unsupported functions, quantity/byte-count errors, unmapped/read-only writes, partial 32/64-bit writes, invalid unit, partial frames, idle timeout, connection limit across binds, multiple clients, multiple server devices, duplicate/overlapping addresses and spans, duplicate bind entries, one-bind failure, listener retry, shutdown/drop/trap, atomic multi-writes, last-writer-wins, code-only swap and cold-required change, status device validation | Error matrix passes; no request can silently corrupt the image; all configuration errors fail closed with the documented codes; listener failure and recovery are observable per listener; multi-write atomicity and concurrency tests pass; online-change tests distinguish preserve from cold-required | M |
| 4 | Cross-host and end-to-end evidence: `tests/modbus/` fixture (ST project, profile, client/simulator, scripts, README); `stc run` end-to-end on Windows x86-64, Linux x86-64, Linux AArch64; project-mode and existing `[test] io` tests; mixed mock test; real mixed bench run on `ecat-host` with EtherCAT and Modbus TCP active concurrently, with no external Modbus↔EtherCAT signal connection; optional independent third-party client check | The fixture builds and runs on all three native hosts; the CI matrix is green; the end-to-end test writes all input tables and reads all output tables through a real TCP client, covering 8/16/32/64-bit values, device-default and per-channel permutation orders, and a multi-bind server; the mixed coordinator mock test covers cross-backend routes; the real bench run shows both backends active concurrently, and the no-external-cross-wiring limitation is recorded; any independent-client caveat is recorded | M |
| 5 | Overhead, documentation consolidation, and PR/review: overhead harness and results; consolidate permanent documentation into `specification/` (I/O contract, CLI/manifest, report, known limitations, README/user guide, decisions); remove stale and working-only content from this plan; update V2.0.0-PLAN.md with the implemented work and FutureWishlist; reconcile the `--io-config`, `--io-ethercat-lib`, and control-status gaps; open the `WL-5.1` → `v2.0.0-planning` pull request; final CI and owner review | Overhead distributions and method are reported; permanent documentation is in `specification/`; stale working content is removed; V2.0.0-PLAN.md is updated; the WL-5.1 PR is green and owner-approved; after merge the working plan is retired and its permanent content is in `specification/`; known limitations state non-RT, no-auth, boolean-on-bit-tables, zero-based addressing, 8-bit byte placement, word-order scope, and security boundaries; decisions are recorded as D-229 onward; CI is green; owner accepts | M |

### 5.1 Phase status

| Phase | Status | Commit | CI |
| --- | --- | --- | --- |
| 0 — contracts | Done | `3547e58` | [41473961570](https://github.com/r2k-in-the-vortex/stc/actions/runs/41473961570) — success |
| 1 — protocol core and bank | Done | `ebfbbc6` | [41478485216](https://github.com/r2k-in-the-vortex/stc/actions/runs/41478485216) — success |
| 2 — backend and coordinator integration | Done | `6c55414` | [41483247583](https://github.com/r2k-in-the-vortex/stc/actions/runs/41483247583) — success |
| 3 — error paths, lifecycle, online change | Done | `97c592d` | [41488899175](https://github.com/r2k-in-the-vortex/stc/actions/runs/41488899175) — success |
| 4 — cross-host and end-to-end | Done | `bf3e7fa` | [41493523376](https://github.com/r2k-in-the-vortex/stc/actions/runs/41493523376) — success |
| 5 — overhead, documentation consolidation, PR | Done | `13bbbed`, `5ceb9df` | [41500681839](https://github.com/r2k-in-the-vortex/stc/actions/runs/41500681839) — success |

## 6. Exit criteria

- Backend works on Windows x86-64 MSVC, Linux x86-64 GNU, and Linux AArch64 GNU; the existing
  CI matrix is green.
- All four tables, all five widths, byte placement, register spans, device word order, and
  per-channel named/index permutations are enforced and tested.
- Addressing is explicitly zero-based; tests pin address 0/1/1024 against vendor
  `00001`/`10001`/`30001`/`40001` notation.
- FC1/2/3/4/5/6/15/16 and exception behavior are covered, including partial wider-write rejection.
- A server can bind multiple addresses from a TOML array sharing one bank; cold-start failure is
  all-or-nothing; one listener can fail and recover while others serve; status shows listener
  counts.
- Request handling cannot silently corrupt the image: multi-writes are atomic, read-only and
  unmapped writes fail closed, and last-writer-wins is documented.
- Client writes are visible at the next input boundary and PLC outputs after the publish
  boundary; no socket work runs on the cycle thread.
- A Modbus project with an EtherCAT mock runs over one process image; the mixed mock test proves
  cross-backend routes and fault isolation; the real bench shows both backends concurrent without
  external cross-wiring.
- The status device, verbose diagnostics, online-change behavior, and all five diagnostic codes
  are implemented, validated, and documented.
- Cycle-boundary overhead is measured and reported; no hard real-time or safety claim is made.
- Permanent documentation is consolidated in `specification/`; this plan is clean; V2.0.0-PLAN.md
  and FutureWishlist.md record completion; the pull request is green and owner-approved.

## 7. Change log

| Date | Change |
| --- | --- |
| 2026-11-01 | Created the WL-5.1 working plan on the post-WL-5.2 baseline. Proposed the explicit profile channel map, table/direction semantics, dedicated-thread scheduling, protocol surface, fault policy, status device, online-change behavior, and evidence plan; no implementation started. |
| 2026-11-02 | Recorded the owner's answers and accepted M-1–M-15 as the locked contract; phases 0–4 completed with commits `3547e58`, `ebfbbc6`, `6c55414`, `97c592d`, and `bf3e7fa`, each with green cross-host CI. |
| 2026-11-02 | Phase 5 complete: `13bbbed` consolidated the permanent documentation and overhead evidence, and `5ceb9df` fixed the ARM e2e discrete-input read race exposed by the first phase-5 CI run. Final run 41500681839 succeeded on all three hosts; the final state is delivered in the `WL-5.1` → `v2.0.0-planning` pull request. |
