# WL-3 Phase 1 — Verification Architecture and Bridge (Working Plan)

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-02
Status: Working planning file. This is **not enduring documentation**; it is retired after Phase 1,
with stale content deleted and durable results moved to the Phase 1 report, the program plan, the
semantics/TCB documents, and the registry.
Branch: `WL-3`
Parent: [Verification Program Plan](specification/Verification/VerificationProgramPlan.md) Phase 1
Phase report: `specification/Verification/Reports/Phase1-Report.md` (durable)
Phase 0 inputs: [Semantics](specification/Verification/Semantics.md),
[Phase 0 Report](specification/Verification/Reports/Phase0-Report.md),
[Phase 0 Design](specification/Verification/Reports/Phase0-Design.md)
Execution host: Ubuntu PREEMPT_RT host `ecat`, described only in the gitignored
`.env/RT_Host.md`. Long-running toolchain, bridge, and proof jobs run there; connection details and
credentials must never be committed or copied into tracked documentation.

## 1. Goal

Decide how Lean evidence constrains the Rust compiler, build the bridge/tooling that later phases
use, and connect the frozen Phase 0 reference semantics to the implementation. Phase 1 produces
the bridge and its evidence; it does **not** produce large correctness proofs.

Program-plan exit criteria:

- a bridge spike report exists;
- D-V1 through D-V10 are resolved;
- the Phase 0 semantics is the reference for the bridge;
- the registry, no-`sorry` check, vector-regeneration check, and oracle harness run in CI;
- the full IR fragment and the first end-to-end subset are written down;
- no component is claimed above V0 except the S-level Phase 0 semantics.

## 1.1 Execution environment

Baseline recorded and verified 2026-11-02 on the Ubuntu host:

- Linux 6.12 PREEMPT_RT, x86-64, 3 CPUs, 7.6 GiB RAM, 37 GiB free disk.
- Installed in user space: Rust `1.99.1` with clippy/rustfmt/llvm-tools/rust-src, Lean
  `4.35.0` (Lake `5.0.0-src+eb6cafb`), and clang `18.1.3`.
- Installed for the bridge spike: opam `2.6.1`, OCaml `5.3.0`, local GMP `6.3.0`, Aeneas
  `nightly-2026.11.02-ab34bd2` with its pinned Charon commit `7b462d9`. Binaries:
  `~/aeneas/bin/aeneas`, `~/aeneas/charon/bin/charon`. Disk after the build: 9.7 GiB free.
- Caveat: Charon-ML's documentation build failed because `odoc` is not installed; the
  Aeneas/Charon CLI binaries built and Aeneas returned `rc=0`. M1 must confirm whether any spike
  step needs the Charon-ML library; `odoc` is documentation-only.
- Verified on `WL-3` at `25b031b`: `lake build` (65 jobs), `stc-sem-tests` (11 groups), registry
  and docs checks, the conformance gate (74/74 cases; report matches), and
  `cargo test --workspace --locked` all pass. The gate requires building
  `stc-driver --bin stc` and the uninstrumented test runtime first, as CI does.
- Windows remains the editing host and runs short local checks; the Ubuntu host runs toolchain
  setup, Aeneas/Charon generation, full proof builds, workspace tests, and overnight jobs.
- Remote work uses the SSH alias/instructions in the gitignored `.env/RT_Host.md`; no host
  credential, key, or private address is recorded in this file, the report, or any tracked file.
- Long jobs run under `tmux` with a log file; the worktree is synced through the Git repository,
  not ad-hoc copies. Do not force-push or leave uncommitted remote work.
- Record exact OS, compiler, Lean, Rust, Aeneas/Charon, and clang versions and the host resource
  envelope in the Phase 1 report. Do not raise a bridge claim from a run whose toolchain is not
  pinned and recorded.

Bootstrap order for M0:

1. Install the pinned Rust toolchain from `rust-toolchain.toml` with `rustup` in the user account.
2. Install the pinned Lean toolchain from `lean/lean-toolchain` with `elan`/`lake`.
3. Verify the Phase 0 baseline on the host: `lake build`, `lake exe stc-sem-tests`, the
   conformance corpus gate, and `cargo test --workspace --locked`.
4. Evaluate Aeneas/Charon installation in user space (OCaml/opam or release artifacts); record the
   exact versions, install path, and disk/build cost. Do not make a bridge decision merely because
   installation succeeds.

## 2. Non-goals and claim discipline

- No V1/V4 implementation-refinement claim is made by the spike. A bridge spike measures
  feasibility, coverage, generated-code readability, and proof obligations.
- A claim is raised only after the evidence for that level exists and is registered with a
  specification reference, owner, and TCB note.
- No `sorry`/`admit`; the Phase 0 axiom policy and registry checker remain in force.
- The frozen `semantics_version = 1.0` is cited, not redefined. A change to a `1.0` definition,
  theorem statement, format field, or scope requires the version bump defined in the Phase 0
  handoff.
- Mathlib stays out of executable models; any adoption is proof-only and pinned.

## 3. Inputs from Phase 0

Phase 1 consumes:

- `stcSem`/`runFuel` and `irRun`/`irExecRec`, with `interpreter_sound`,
  `interpreter_complete`, and `ir_recursive_model_equiv`;
- `Step`/`Steps` and `IRStep`/`IRStepStar` as the declarative loop-shaped references;
- the registry schema/checker and the no-`sorry`/axiom policy;
- the intended bridge points:
  - `crates/stc-codegen/src/exact.rs` against `Stc.Exact`;
  - `crates/stc-got/src/keys.rs` against GOT key normalization;
  - one `stc-got` codec path (`wire.rs` primitive encode/decode or one descriptor section);
- the explicit-state/control-flow and fuel obligations in
  [Phase0-Report §8.3](specification/Verification/Reports/Phase0-Report.md):
  a bridge theorem must state its fuel bound, may not identify the host call stack with the
  model frame stack, and must preserve the step relation, not only the final value.

## 4. Scope to settle in Phase 1

### 4.1 Bridge decision per target class

Use the program plan §7 decision matrix:

| Target class | Preferred bridge | Fallback | Do not claim |
| --- | --- | --- | --- |
| Pure scalar kernel (`exact.rs`, conversion tables) | Aeneas/Charon refinement | Lean extraction to C/Rust used in the build | V1 from vectors alone |
| Key/layout/classification kernels | Aeneas/Charon refinement | Lean extraction or proved checker at the artifact boundary | V1 from matching test vectors |
| Artifact codecs/validators | Proved Lean checker on the artifact (V2) | Aeneas proof of the Rust decoder | V2 for a checker that inspects fixtures only |

The spike measures the preferred and fallback paths; the decision is recorded per target class,
not as a blanket "adopt Aeneas" statement.

### 4.2 Toolchain policy

- **Owner decision (2026-11-02):** install and evaluate Aeneas/Charon in user space on the
  Ubuntu host; inventory and pin exact versions; add them to CI only after the spike evidence is
  reviewed. No system-wide changes without a separate approval.
- Check whether `aeneas`/`charon` are available on the development/CI platform; record exact
  versions and installation path.
- Pin the toolchain and generated-Lean location; generated models must be reproducible.
- Keep hand-written proofs separate from generated models; never edit generated Lean by hand.
- Record the Rust source hash the model was generated from; fail CI on drift.
- Keep the fast proof CI under the documented budget; put long regeneration in a nightly job if
  needed.

### 4.3 Full IR fragment and first end-to-end subset

- **Owner decision (2026-11-02):** Phase 1 freezes and documents the full emitted-LLVM-IR
  fragment boundary, but implements only the fragment pieces needed by the bridge and the D-V6
  subset. Phase 5 owns the full Lean `IRFrag`/`WellFormed` extension. This is a boundary/spec
  freeze now, not a reinterpretation of the frozen `1.0` core.
- **Owner decision (2026-11-02):** approve the D-V6 first end-to-end subset: scalar types,
  expressions, assignments, control flow, functions, programs, and GOT globals; no OOP,
  references, strings, or aggregates.
- Record both the fragment boundary and the subset in the program plan and the Phase 1 report.
- **Recorded 2026-11-03 (M5):** the boundary and subset are frozen in
  [`Core-IR-Boundary.md`](specification/Verification/Core-IR-Boundary.md); see §13.

### 4.4 TCB and claim language

Write the TCB/claim-language document (D-V4): what is trusted, what each level means, which
claims are permitted, and what evidence a level change requires. Reuse the Phase 0 TCB and extend
it with the bridge toolchain/translation boundary.

### 4.5 Owner decisions resolved 2026-11-02

| Gate | Resolution |
| --- | --- |
| Aeneas/Charon toolchain | Install/evaluate in user space on the Ubuntu host; pin exact versions; add to CI only after spike evidence review; no system-wide changes without separate approval. |
| Full IR fragment | Freeze and document the full emitted-fragment boundary in Phase 1; implement only the bridge/D-V6 slice; Phase 5 owns the full Lean `IRFrag` extension. |
| D-V6 subset | Approved: scalar types, expressions, assignments, control flow, functions, programs, GOT globals; no OOP, references, strings, or aggregates. |
| Claim level | No new claims: all Phase 1 bridge/tooling work stays V0; Phase 0's S-level semantics remains the only reviewed claim. |
| Aeneas/Mathlib proof environment | Approved: prune rebuildable build artifacts and build the Aeneas Lean backend plus Mathlib v4.35.0 on the Ubuntu host to measure compile/proof cost. |
| Bridge-toolchain CI | Resolved 2026-11-03: fast CI stays free of Aeneas/Mathlib; add a separate manual/nightly drift workflow that installs the pinned toolchain and checks regeneration.
| Exact kernel bridge | Aeneas/Charon on the filtered pure exact kernel; derived/helper bodies opaque or filtered; compile/prove against `Stc.Exact`. |
| Keys bridge | Proved Lean key-normalization contract/checker; do not use Aeneas for the string/iterator-heavy keys module. |
| Codec bridge | Proved Lean wire-format checker as primary; the successful Aeneas translation is fallback evidence, not the chosen bridge. |

Remaining open gate: Phase 1 report acceptance and working-plan retirement (after M7). The bridge
choice and bridge-toolchain CI decisions are resolved above.

## 5. Work breakdown

| Milestone | Work | Deliverable/evidence |
| --- | --- | --- |
| M0 — Baseline | Confirm Phase 0 records, CI, registry, and Rust baselines; inventory the three bridge targets and their dependencies; record available toolchains. Establish the Ubuntu execution host baseline, bootstrap the pinned Rust/Lean toolchains, and verify the Phase 0 checks there. | Complete: host/toolchain record; Phase 0 checks green on the host; Aeneas/Charon user-space install and pinned build recorded (2026-11-02); no claim change. |
| M1 — Exact bridge spike | Run Aeneas/Charon on `crates/stc-codegen/src/exact.rs` (or the feasible subset). Measure translation coverage, generated Lean readability, proof obligations, and gaps versus `Stc.Exact`. | Complete 2026-11-02: filtered pure-kernel `Exact.lean` typechecks under Aeneas/Mathlib; four external axioms remain for proof modeling; evidence in §11. |
| M2 — Keys bridge spike | Run the spike on `crates/stc-got/src/keys.rs` normalization/key functions. Record string/UTF-8 and allocation/borrow friction; compare against the key-normalization contract; run vector/differential checks. | Complete 2026-11-02: Aeneas cannot translate the module as-is (loops/string/lifetime errors); fallback or refactor required; evidence in §11. |
| M3 — Codec bridge spike | Pick one codec path (`wire.rs` primitive encode/decode or one descriptor section). Measure generated-model/proof-obligation cost and whether a proved Lean checker is the better bridge. | Complete 2026-11-02: wire encoder/decoder translated (`StcGot.lean`, 54 transparent functions); external-definition and Aeneas/Mathlib proof costs remain; evidence in §11. |
| M4 — Decision and architecture | Apply the decision matrix per target class; record ADRs; define bridge module layout and proof-engineering policy; do not raise claims yet. | Complete 2026-11-03: per-target decisions, module layout, generated/hand-written boundary, pins and source hashes, external-axiom trust policy, and registry evidence plan recorded in §12; no claim change. |
| M5 — IR boundary and D-V6 subset | Freeze and document the full emitted-fragment boundary; implement only the bridge/D-V6 slice; write D-V6's first end-to-end subset; leave the full Lean `IRFrag` extension to Phase 5; update coverage tables. | Complete 2026-11-03: full production-module boundary and D-V6 subset frozen in `specification/Verification/Core-IR-Boundary.md`; program plan, Core IR Coverage, Core Syntax Coverage, and `check_docs.py` updated; no Lean-side fragment slice and no claim change; record in §13. |
| M6 — CI and registry | Extend the registry for bridge/toolchain records; add no-`sorry`, axiom, vector-regeneration, and oracle checks; add Aeneas regeneration drift checks if the bridge is adopted; document CI budget. | CI changes and a green run; registry entries. |
| M7 — Report and retirement | Write `specification/Verification/Reports/Phase1-Report.md`; move durable decisions to the program plan/TCB/claim doc/registry; delete stale working notes and retire this file. | Durable report; Phase 1 review package; working file deleted. |

Milestones M1–M3 are timeboxed spikes. A failed/unavailable toolchain is a valid spike result; do
not fake progress by writing an unverified Lean model.

## 6. Phase report outline

`specification/Verification/Reports/Phase1-Report.md` will record:

1. Scope, inputs, and non-claims.
2. Toolchain versions, platform, installation/pinning, reproducibility.
3. Exact-spike coverage, generated-code readability, proof obligations, and gaps.
4. Keys-spike coverage and friction points.
5. Codec-spike result and the proved-checker alternative.
6. Bridge decision per target class with evidence and rejected alternatives.
7. IR fragment extension and D-V6 subset.
8. Registry/CI/oracle changes and the CI budget.
9. TCB/claim-language changes.
10. Residual risks, owned follow-ups, and the Phase 1 review checklist.

## 7. Verification and reporting commands

Baseline checks for every milestone:

```text
cd lean && lake build
lake exe stc-sem-tests
PYTHONUTF8=1 python tools/ci/check_verification_registry.py
python tools/ci/check_docs.py
cargo build --locked -p stc-driver --bin stc
cargo build --locked -p stc-runtime --target-dir target/test-runtime
STC_CLANG=<clang> python tools/ci/check_semantics_placeholder.py
npx --yes markdownlint-cli2@0.24.0
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

Bridge-specific commands and toolchain versions are recorded in the Phase 1 report as they are
established.

On the Ubuntu host, run the same checks from a clean, pushed worktree under `tmux`, with a
timestamped log, for example:

```text
ssh <alias-from-.env/RT_Host.md>
cd <clean-worktree>
tmux new -s wl3-p1
cd lean && lake build -j2 2>&1 | tee ../phase1-lake.log
cd .. && cargo test --workspace --locked 2>&1 | tee phase1-cargo-test.log
```

Use `-j2` on the 3-CPU host, monitor memory, and leave long Aeneas/Charon or proof jobs running
overnight. Commit/push results from the host or fetch them back through Git; do not sync secrets or
untracked credentials.

## 8. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Aeneas/Charon unavailable or unsupported on this platform | Spike cannot produce a refinement model | Record the blocker; use the decision-matrix fallback and a proved-checker path; do not claim V1. |
| Rust features exceed translation coverage (borrows, allocations, string formatting) | Partial or unreadable generated model | Timebox; isolate pure kernels; record exact gaps and whether extraction/checker is preferable. |
| Proof obligations explode | Schedule slip | Spike only; measure first; defer large proofs to Phase 2+. |
| Generated Lean drifts from Rust | False confidence | Pin toolchain; record source hash; CI regeneration/diff check. |
| Full IR fragment grows unbounded | Scope creep | Freeze the emitted-fragment boundary; versioned extension; exclude uncovered constructs explicitly. |
| Claim inflation | Overclaiming V1/V4 | Enforce the claim-language document; no level increase without registered evidence and review. |
| Toolchain/license/build-time cost | CI and maintenance burden | Measure; pin; nightly long job; document the TCB. |
| Ubuntu host capacity (3 CPUs, 7.6 GiB RAM, 37 GiB free) | Lean/Aeneas builds may OOM or fill disk | Use `-j2`, monitor memory/disk, clean caches, run long jobs overnight with logs, and record the resource envelope. |
| Remote credential leakage or untracked remote work | Security/consistency risk | Keep connection details in the gitignored `.env/RT_Host.md`; never commit keys/addresses; sync through Git and check `git status` before/after remote runs. |

## 9. Owner decision gates

| Gate | Status |
| --- | --- |
| Aeneas/Charon versions and build/CI adoption | Install/evaluate in user space approved 2026-11-02; Aeneas/Mathlib proof environment built 2026-11-02; exact pins and source hashes recorded 2026-11-03 in `lean/bridge-toolchain.toml`; CI adoption resolved 2026-11-03 as a separate manual/nightly drift workflow, with fast CI toolchain-free. |
| Bridge choice per target class | Resolved 2026-11-02: exact = filtered Aeneas kernel; keys = proved checker/contract; codec = proved Lean checker. Any change requires a new owner review. |
| Bridge architecture and proof-engineering policy | Recorded 2026-11-03 in §12; no new owner decision required; no claim change. |
| Full emitted-IR fragment boundary / versioned format | Boundary freeze approved 2026-11-02 and recorded 2026-11-03 in `specification/Verification/Core-IR-Boundary.md` (boundary version 1); full Lean extension is Phase 5; any versioned-format change still requires approval. |
| D-V6 first end-to-end subset | Approved 2026-11-02 and fixed 2026-11-03 in `specification/Verification/Core-IR-Boundary.md` §11; Phase 9 owns the theorem. |
| Claim level increase or refinement/translation-validation claim | Closed for Phase 1: no new claims; V0 evidence only, Phase 0 S remains the only reviewed claim. |
| Phase 1 report acceptance and working-plan retirement | Open; after M7. |

## 10. Retirement protocol

On Phase 1 sign-off:

- move durable decisions, evidence, TCB changes, and residual risks to the Phase 1 report, the
  program plan, the semantics/TCB documents, and the registry;
- delete stale working notes from this file;
- delete `WL-3-Phase-1.md` once every durable item has a home;
- keep `specification/Verification/Reports/Phase1-Report.md` as the enduring record.

## 11. Spike log

### M1 — `exact.rs` (2026-11-02)

- Source: `crates/stc-codegen/src/exact.rs`, SHA-256
  `85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4`, 738 lines.
- Charon `0.1.304 (7b462d9)` with `--preset=aeneas`, compiling the file as a library with edition
  2024, produced a 2.3 MB `exact.llbc` (94 transparent functions, 55 opaque functions, 217
  globals).
- Aeneas `ab34bd2`, Lean backend, produced a 1708-line `Exact.lean` (119 `def`s, 22 `axiom`s).
  All core arithmetic/conversion/selection functions are present, including `checked`, `add`,
  `sub`, `mul`, `neg`, `div`, `modulo`, `abs`, `bit_and`, `bit_or`, `bit_xor`, `bit_not`, `shl`,
  `shr`, `rol`, `ror`, `compare`, and `sel`.
- Aeneas returned `rc=1` because unsupported bodies failed: derived `PartialEq`/`Debug` impls,
  `MINMAX_TYPES`, `comparison`, `shift`, `bits`, `unary`, `binary`, the `mux` closure, and
  `REGISTRY::{promoted_const}`. These are macro/trait-impl/iterator-shaped helpers, not the pure
  arithmetic kernel.
- The generated Lean imports the Aeneas library; that library requires Mathlib v4.35.0. The
  proof environment was later built successfully: 1796 jobs, Mathlib cache, 18 GiB free.
- With that environment, `lake env lean StcGot.lean` typechecks (`rc=0`). `Exact.lean` does not:
  the only hard error is in `REGISTRY` (`Array.to_slice` implicit `n`), and the unsupported
  helper bodies (`shift`, `comparison`, `bits`, `unary`, `binary`, `REGISTRY`) are emitted as
  `sorry`. The pure arithmetic definitions themselves are present.
- Spike conclusion: the pure kernel is translatable, but the whole-file translation must be
  filtered to the pure functions so the unsupported helper/`REGISTRY` bodies are not emitted;
  the filtered rerun is the next exact-spike step. No claim is raised.
- Refined filtered run: Charon with roots for `add`, `sub`, `mul`, `neg`, `div`, `modulo`,
  `abs`, the bitwise/shift/rotate functions, `compare`, and `sel` produced a 502 KB LLBC.
  Aeneas returned `rc=0` and generated a 12.8 KB `Exact.lean`; `lake env lean` typechecked it
  with `rc=0`. The file contains the pure core definitions plus four external axioms
  (`I64::from(u8)`, `I64::rem_euclid`, `I128::checked_neg`, `I128::abs`). This is the chosen
  exact-bridge slice; the remaining proof obligation is the external-axiom model and the
  refinement/equivalence argument against `Stc.Exact`.

### M2 — `stc-got` keys (2026-11-02)

- Charon `0.1.304`, `--preset=aeneas --start-from stc_got::keys`, compiled `stc-got` and
  produced a 6.8 MB LLBC (271 prepasses).
- Aeneas `ab34bd2` returned `rc=2` and produced no Lean file. Errors:
  - early returns inside loops (`stc_got::digest::parse_hex`);
  - unsupported `str::rsplit_once` and `str::strip_suffix` signatures;
  - a higher-ranked lifetime constraint in the slice iterator `find`;
  - an internal Aeneas `CFailure`.
- The start-from filter did not isolate the key functions from these dependencies; string
  normalization and iterator/loop shapes need refactoring or a non-Aeneas bridge. No claim is
  raised.
- Spike conclusion: Aeneas is not viable for the keys module as written; the fallback is a
  proved Lean key-normalization contract/checker, extraction, or a source refactor with a new
  spike.

### M3 — `stc-got` wire codec (2026-11-02)

- Charon `0.1.304`, `--preset=aeneas --start-from stc_got::wire` with keys/descriptor/manifest
  opaque, produced a 1.3 MB LLBC.
- Aeneas `ab34bd2` succeeded (`rc=0`, 3.16 s) and generated a 49 KB `StcGot.lean` with 54
  transparent functions and 65 opaque functions. `wire.Encoder`/`Decoder` and the primitive
  `u8`/`u16`/`u32`/`u64`/`i64`/`i128`/`bool`/`tag`/`id128`/`digest32`/`string`/`list_header`
  functions are present.
- Aeneas warned that the crate contains extracted external/unknown definitions and recommends
  `-split-files` so they can be supplied explicitly. The generated Lean imports Aeneas and still
  needs the Aeneas/Mathlib proof environment; compile/proof cost is not yet measured.
- Spike conclusion: the wire codec path is translatable by Aeneas, but the artifact-codec expert
  matrix prefers a proved Lean checker; the generated model is evidence for the fallback path.

## 12. M4 bridge architecture and proof-engineering record (2026-11-03)

### 12.1 Decisions

| ID | Decision |
| --- | --- |
| M4-D1 | Generated models live in a separate, non-default Lake package `lean/aeneas/`, not under `lean/Stc/`. The default `lake build` at `lean/` never resolves or builds Mathlib/Aeneas. |
| M4-D2 | The filtered exact model is `lean/aeneas/StcGenerated/Exact/Model.lean`; the hand-written `Stc.Exact` model is unchanged and remains the reference. |
| M4-D3 | Generated and hand-written code are separated by directory and module: `StcGenerated/` is generated-only and never edited; hand-written bridge code is in `lean/Stc/Bridge/` and `lean/aeneas/Bridge/`. |
| M4-D4 | The four external axioms are modeled in `Stc.Bridge.Exact.Externals`; each is an equation against the model, named in the TCB and the registry, never an unstated assumption. No V1 claim until they are discharged. |
| M4-D5 | Keys bridge is a proved Lean contract/checker in `Stc.Bridge.Keys`; Aeneas is not used for the string/iterator-heavy module. |
| M4-D6 | Wire codec bridge is a proved Lean checker in `Stc.Bridge.Wire`; the successful Aeneas translation is fallback evidence only. |
| M4-D7 | Pins and source hashes are recorded in `lean/bridge-toolchain.toml`; regeneration is manual/reviewed on the pinned Ubuntu host and never runs in fast CI. |
| M4-D8 | Registry evidence is defined here but added in M6. No Phase 0 registry record or claim changes in M4. |
| M4-D9 | No claim above V0 is made in Phase 1; the Phase 0 S-level semantics remains the only reviewed claim. |

### 12.2 Module layout

```text
lean/
  Stc/                              # default Lake package; core Lean only
    Exact/                          # existing Stc.Exact reference model (unchanged)
    Bridge/
      Exact/
        Encode.lean                 # generated value/result encoding to Stc.Exact
        Externals.lean              # models for the four external Rust operations
      Keys/
        Contract.lean               # declarative key grammar and normalization contract
        Checker.lean                # executable checker and soundness interface
      Wire/
        Contract.lean               # declarative wire format and decoder relation
        Checker.lean                # executable decoder/validator and soundness interface
  aeneas/                           # separate Lake package; Mathlib/Aeneas allowed
    lakefile.lean
    lake-manifest.json              # resolved Aeneas/Mathlib revisions (M6)
    lean-toolchain                  # same v4.35.0 pin
    StcGenerated/
      Exact/Model.lean              # filtered Aeneas output; generated, do not edit
      Exact/Externals.lean          # Aeneas split-file externals; generated, do not edit
      Wire/Model.lean               # fallback evidence; generated, do not edit
    Bridge/
      Exact/Refinement.lean         # hand-written V1 obligation; imports both sides
      Exact/Audit.lean              # axiom inventory for the bridge
  bridge-toolchain.toml             # durable pins and source hashes
```

The default package is exactly the `stc` Lake package at `lean/`. It must not gain a Mathlib or
Aeneas dependency: the existing `lake build`, `lake exe vectors`, `lake exe stc-sem-tests`, and the
registry audit stay fast. `lean/aeneas/` is a separate package with its own manifest; it depends on
the parent package for `Stc.Exact` and on the pinned Aeneas Lean backend plus Mathlib. Only
`lean/aeneas/Bridge/Exact/Refinement.lean` and `Audit.lean` may import generated modules.

### 12.3 Exact bridge: generated model, `Stc.Exact`, and the four external axioms

The filtered Aeneas model contains the pure kernel: `checked`, `add`, `sub`, `mul`, `neg`, `div`,
`modulo`, `abs`, the bitwise/shift/rotate functions, `compare`, and `sel`. The bridge is additive:
no `Stc.Exact` definition or theorem statement changes, so `semantics_version` stays `1.0`.
`Stc.Bridge.Exact.Encode` defines the map from the generated representations (`IntType` with `u32`
width and a signedness flag, `Result i128 Trap`, integer values) to `Stc.Exact.IntType`,
`Stc.Exact.Trap`, and `Stc.Exact.Res`. The refinement obligation is stated on decoded results, per operation, for
example `exact_add_refines`, `exact_div_refines`, `exact_convert_refines`, and `exact_sel_refines`:
the decoded generated result equals the corresponding `Stc.Exact` operation, including the trap
value. The filtered functions are pure and loop-free, so no harness loop model or fuel bound is
expected; if regeneration introduces one, the theorem must state the fuel obligation explicitly
and may not present it as total correctness.

The filtered model leaves four operations external. They are modeled as follows; the exact
statements are fixed when the model is checked in and are audited by `#print axioms`.

| External op | Rust use and domain | Hand-written model | Trust treatment |
| --- | --- | --- | --- |
| `I64::from(u8)` | `IntType.count`; total widening conversion | Natural embedding of the `u8` value into `Int` | Discharge by replacing the generated external with this definition (Aeneas split-file), or carry as a named TCB hypothesis |
| `I64::rem_euclid` | `IntType.count` (used by `shl`, `shr`, `rol`, `ror`); divisor is always positive, so the Rust zero-divisor and `MIN % -1` panics are unreachable | Euclidean remainder `a.emod b` under `0 < b`, result in `[0, b)` | Discharge with a domain lemma from the kernel control flow, or carry as a named TCB hypothesis |
| `I128::checked_neg` | `neg`; `Option<i128>` semantics | `none` exactly at `i128::MIN`, otherwise `some (-a)` | Discharge by definition replacement, or carry as a named TCB hypothesis |
| `I128::abs` | `abs`; called only after the signed `MIN` guard, so the Rust panic case is unreachable | `if a < 0 then -a else a` on the guarded domain | Discharge with the guard lemma, or carry as a named TCB hypothesis |

The generated axioms are not accepted into a claim as-is. Each is restated as an equation against
the hand-written model; a proof may leave it as an explicit hypothesis, but the hypothesis is then
listed in the TCB and in the registry `tcb` field. While any equation is undisclosed, a fuel
obligation is unstated, or the generated model has not passed the axiom audit, the exact bridge
stays V0 and the allowed claim remains "model-verified; implementation tested against the model".

### 12.4 Keys and wire checker interfaces

The keys checker is the primary bridge for GOT key normalization; it is hand-written Lean and does
not model Rust borrowing, allocations, or iterators. `Stc.Bridge.Keys.Contract` fixes:

- `IdentityKind` with the nine identity tags and their canonical key forms;
- `Key`/`NormalizedKey`, the key grammar predicate, and the normalization relation;
- `normalize : String → Except KeyError NormalizedKey` and
  `validate : IdentityKind → String → Except KeyError Unit`;
- interface theorems `normalize_sound` (every accepted output satisfies the grammar and is
  canonical), `normalize_idempotent`, and `validate_sound`, plus an artifact-boundary
  `check_artifact_sound` for Phase 3.

The wire checker is the primary bridge for the codec; the Aeneas `StcGot.lean` translation is
fallback evidence. `Stc.Bridge.Wire.Contract` fixes:

- a byte-buffer state with an explicit position, and little-endian decoders for `u8`, `u16`,
  `u32`, `u64`, `i64`, `i128`, `bool`, `tag`, `id128`, `digest32`, and length-prefixed strings;
- `count` with the untrusted-length bound and `finish` with the no-trailing-bytes rule;
- interface theorems `decode_sound` (a successful decode advances exactly its width and returns
  the specified value), `decode_encode` (round-trip on valid values), `count_bounded`, and
  `finish_sound`.

Neither checker is implemented in Phase 1. The interfaces fix the placement and theorem names so
Phase 3 can implement them without changing the architecture. A checker proof becomes V2 evidence
only when it runs on the artifacts the compiler ships or on every module emitted in CI; fixture-only
runs stay V0/V1.

### 12.5 Generated pins, source hashes, and the regeneration boundary

`lean/bridge-toolchain.toml` records Lean `leanprover/lean4:v4.35.0`, Rust `1.99.1`, Mathlib
`v4.35.0` (proof-only), Aeneas `nightly-2026.11.02-ab34bd2`, Charon `0.1.304` at `7b462d9`, and
the OCaml/opam/GMP install baseline. It also records the three bridge source hashes:

| Target | Source | SHA-256 |
| --- | --- | --- |
| Exact | `crates/stc-codegen/src/exact.rs` | `85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4` |
| Keys | `crates/stc-got/src/keys.rs` | `af80f2d87d4c9f9d6f59310210d5020daccbeb7ef0b02af03d91bf2aa9ad34bc` |
| Wire | `crates/stc-got/src/wire.rs` | `ae840ab12704434b162013fafc85facc2883275dcaefccd2951fd2d09b7810fd` |

Every generated file carries a header naming the Aeneas/Charon versions and the source path and
SHA-256. Regeneration boundary:

1. Run only on the pinned Ubuntu host with the pinned toolchain and the pinned Aeneas/Charon
   binaries; never in the fast CI job and never on the Windows editing host.
2. Charon runs with `--preset=aeneas` and explicit roots for the filtered kernel; Aeneas uses the
   Lean backend. The whole-file exact output (unsupported helper bodies and `REGISTRY` as `sorry`)
   is not accepted as evidence.
3. Typecheck the generated model in the `lean/aeneas` package, run the axiom audit, and review the
   generated diff. Generated files are never hand-edited to make them compile.
4. Commit the generated model and the pin/hash record in the same reviewed change. A source change
   without regeneration or an owner-approved pin update fails the M6 drift check.

### 12.6 Proof-engineering policy

- **Pins.** Lean/Rust are pinned in `lean/lean-toolchain` and `rust-toolchain.toml`; the bridge
  pins and source hashes are in `lean/bridge-toolchain.toml`; the resolved Aeneas/Mathlib
  revisions are in `lean/aeneas/lake-manifest.json` (M6). A pin change requires owner review and a
  new bridge spike.
- **Mathlib is proof-only.** It is confined to `lean/aeneas/`; it never enters the default `Stc`
  library, executable models, vector generator, registry audit, or proved checkers.
- **Generated files are not edited.** `StcGenerated/` is generated-only; hand-written proofs live
  under `Bridge/`. Regeneration is a reviewed diff, not an edit.
- **No `sorry`/`admit`.** The existing CI grep stays; M6 extends it to the hand-written bridge
  modules and to any generated model used as evidence. A generated `sorry` body is filtered out or
  recorded as opaque; it is never imported into a claim.
- **Axiom audit.** Every top-level theorem cited by a claim is audited with `#print axioms`; the
  only permitted standard axioms are `propext`, `Classical.choice`, and `Quot.sound`. The four
  external bridge axioms are named in the TCB and registry `tcb` field. `native_decide` is not
  used in a trusted theorem.
- **Registry.** M6 extends the registry, not M4. The planned Phase 1 records are V0 evidence with
  status `tested` (spikes) or `planned` (checker interfaces); none is `proved` and none claims
  refinement. The Phase 0 registry count and freeze record stay unchanged in M4.
- **CI budget.** The fast `lean` job builds only `lean/`; it must not install Mathlib or Aeneas.
  The generated-model build and regeneration belong to a nightly/manual `aeneas` job. The spike
  measured the Aeneas/Mathlib environment at 1796 build jobs; M6 records the budget and timeout.
- **Claims.** No claim above V0 in Phase 1. The exact bridge stays "model-verified; implementation
  tested against the model" until the refinement theorem and its external-axiom equations are
  discharged and registered.

### 12.7 Registry evidence plan (executed in M6)

M6 adds `P1` to the registry phase vocabulary and a hand-written `Verification.Bridge.Evidence`
module that carries the evidence values (tool versions, source hashes, translation counts). It
also extends module resolution to `lean/aeneas/` for generated-model records. The planned entries
are:

| ID | Kind | Level | Status | Evidence |
| --- | --- | --- | --- | --- |
| P1-E01 | evidence | V0 | tested | Filtered exact model typechecks under the pinned Aeneas/Mathlib environment (`rc=0`). |
| P1-E02 | evidence | V0 | tested | The four external axioms are enumerated and their statements reviewed; none is discharged. |
| P1-E03 | evidence | V0 | tested | Exact/keys/wire source SHA-256 values match the spike inputs. |
| P1-E04 | evidence | V0 | tested | Keys Aeneas translation fails (`rc=2`; loops, string, and lifetime errors); the checker/contract is the chosen fallback. |
| P1-E05 | evidence | V0 | tested | Wire Aeneas translation succeeds (54 transparent, 65 opaque functions) and the generated file typechecks in the proof environment (`rc=0`); proof-obligation cost remains unmeasured; fallback evidence only. |
| P1-E06 | evidence | V0 | tested | `lean/bridge-toolchain.toml` matches the environment used for the spikes. |
| P1-E07 | evidence | V0 | planned | Keys and wire checker contracts/interfaces are defined; implementation and proofs are Phase 3. |

When M6 adds these records it also rewords the Phase 0 report's "45 records" statement as a
P0.7-freeze count, so the live registry total stays accurate. No V1/V2 record is added in Phase 1.

## 13. M5 emitted-IR boundary and D-V6 subset record (2026-11-03)

### 13.1 Boundary

The full production-module boundary is frozen in
`specification/Verification/Core-IR-Boundary.md` (boundary version `1`). It was derived from the
emitter and the committed snapshot/verifier tests, not from LLVM documentation. It records:

- the module envelope: header, deterministic section order, function and global linkage, entry
  points, and the absence of named types, module asm, comdats, aliases, and production metadata;
- types: `void`, `i1`–`i64`, opaque `ptr`, `float`/`double`, the overflow result, fixed arrays,
  structs, and the emitted aggregate shapes;
- instructions: integer/bitwise/floating binops, comparisons, `select`, casts, `phi`,
  `extractvalue`/`insertvalue`, `alloca`/`load`/`store`/GEP, direct and indirect calls, and the
  `llvm.memcpy` call;
- the direct-callee set (overflow and rotate intrinsics, floating intrinsics, `memcmp`, and the
  internal helpers), the indirect-call origins (`Function`, `Import`, vtable code), the host
  service table, and the trap/assert patterns;
- module globals, the GOT two-load access pattern, cell-kind use, the canonical slot-table shape,
  process-image and vtable access as later-phase forms;
- the seven entry-point signatures, the descriptor/function-table/metadata shapes, and the
  control-flow and UB policy;
- explicit construct and build-form exclusions: test builds, debug builds, launcher executables,
  atomics, volatile, `invoke`/`landingpad`, `bitcast`/`inttoptr`, vectors, dynamic `alloca`,
  `switch`, inline asm, and the never-emitted intrinsics;
- versioning and change-control rules tying additions to the boundary document, the P0.4 table,
  the Lean model, and the verifier.

### 13.2 D-V6 subset

The D-V6 first end-to-end subset is fixed exactly as owner-approved: scalar types, expressions,
assignments, control flow, functions, programs, and GOT globals; no OOP, references, strings, or
aggregates. The boundary document maps it to `Semantics.md` §1 and the P0.4 tables, lists the
included source constructs and target forms, and assigns every deferred feature to its owning
phase. The end-to-end theorem remains Phase 9's `pipeline_correct`; M5 fixes its scope only.

### 13.3 Implementation decision

Phase 1 adds no Lean `IRFrag`/`WellFormed` slice. The reviewed Phase 0 fragment stays authoritative
for the core; Phase 5 owns the Lean extension to the full boundary; Phase 7 owns codegen
preservation; Phase 9 owns the D-V6 theorem. This keeps the default `lean/` build core-only and
adds no Aeneas/Mathlib dependency and no claim. The Phase 1 executable evidence remains the Rust
structural verifier and the clang acceptance tests (V0).

### 13.4 Files and coordination

New: `specification/Verification/Core-IR-Boundary.md`. Updated:
`specification/Verification/VerificationProgramPlan.md` (status, Phase 1 M5 record, feature matrix,
D-V3/D-V6, next steps), `specification/Verification/Core-IR-Coverage.md` and
`specification/Verification/Core-Coverage.md` (boundary links and scope relationship),
`tools/ci/check_docs.py` (required document), and this working plan. No registry record, Lean
module, toolchain pin, or claim changed.

## 14. Status board

| Item | State |
| --- | --- |
| M0 baseline and toolchain inventory | Complete — Rust/Lean/opam/OCaml/GMP/Aeneas/Charon installed; Phase 0 baseline green on the Ubuntu host |
| M1 exact bridge spike | Complete — filtered pure-kernel Lean typechecks; four external axioms remain for proof modeling |
| M2 keys bridge spike | Complete — Charon produced LLBC; Aeneas failed on loops/string/lifetime support; fallback/refactor required |
| M3 codec bridge spike | Complete — wire codec translated to Lean; external definitions and Aeneas/Mathlib proof cost remain |
| M4 bridge decision and architecture | Complete 2026-11-03 — decisions, module layout, pins/hashes, external-axiom policy, and registry evidence plan recorded in §12; no claim change |
| M5 IR boundary and D-V6 subset | Complete 2026-11-03 — full production-module boundary and D-V6 subset frozen in `Core-IR-Boundary.md`; coverage tables, program plan, and `check_docs.py` updated; no Lean-side slice and no claim change |
| M6 CI and registry | Not started |
| M7 report and retirement | Not started |
| Owner decision gates | Toolchain install, IR boundary, D-V6, no-new-claims, bridge choices, and bridge-toolchain CI resolved; Phase 1 report acceptance open |
