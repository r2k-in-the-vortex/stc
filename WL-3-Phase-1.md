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

Remaining open gates: the bridge choice per target class (after M1–M3), CI adoption of the bridge
toolchain (after spike evidence), and Phase 1 report acceptance/retirement (after M7).

## 5. Work breakdown

| Milestone | Work | Deliverable/evidence |
| --- | --- | --- |
| M0 — Baseline | Confirm Phase 0 records, CI, registry, and Rust baselines; inventory the three bridge targets and their dependencies; record available toolchains. Establish the Ubuntu execution host baseline, bootstrap the pinned Rust/Lean toolchains, and verify the Phase 0 checks there. | Complete: host/toolchain record; Phase 0 checks green on the host; Aeneas/Charon user-space install and pinned build recorded (2026-11-02); no claim change. |
| M1 — Exact bridge spike | Run Aeneas/Charon on `crates/stc-codegen/src/exact.rs` (or the feasible subset). Measure translation coverage, generated Lean readability, proof obligations, and gaps versus `Stc.Exact`. | Complete 2026-11-02: pure kernel translated; derived/helper bodies failed; Aeneas/Mathlib compile cost outstanding; evidence in §11. |
| M2 — Keys bridge spike | Run the spike on `crates/stc-got/src/keys.rs` normalization/key functions. Record string/UTF-8 and allocation/borrow friction; compare against the key-normalization contract; run vector/differential checks. | Keys-spike section; evidence and gap list. |
| M3 — Codec bridge spike | Pick one codec path (`wire.rs` primitive encode/decode or one descriptor section). Measure generated-model/proof-obligation cost and whether a proved Lean checker is the better bridge. | Codec-spike section; bridge recommendation. |
| M4 — Decision and architecture | Apply the decision matrix per target class; record ADRs; define bridge module layout and proof-engineering policy; do not raise claims yet. | Bridge decision section; module skeleton; owner decision request for toolchain adoption. |
| M5 — IR boundary and D-V6 subset | Freeze and document the full emitted-fragment boundary; implement only the bridge/D-V6 slice; write D-V6's first end-to-end subset; leave the full Lean `IRFrag` extension to Phase 5; update coverage tables. | Boundary document, updated coverage, program-plan/report text. |
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
| Aeneas/Charon versions and build/CI adoption | Install/evaluate in user space approved 2026-11-02; CI adoption remains open until spike evidence. |
| Bridge choice per target class | Open; decide after M1–M3 using the decision matrix. |
| Full emitted-IR fragment boundary / versioned format | Boundary freeze approved 2026-11-02; full Lean extension is Phase 5; any versioned-format change still requires approval. |
| D-V6 first end-to-end subset | Approved 2026-11-02 as the program-plan recommendation. |
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
- The generated Lean imports the Aeneas library; that library requires Mathlib v4.35.0. Building
  the Aeneas/Mathlib proof environment and compiling `Exact.lean` was not attempted in this
  spike; it is a measured follow-up cost.
- Spike conclusion: the pure kernel is translatable; the derived/helper bodies need isolation
  (opaque/filtered translation or a reduced wrapper), and the Aeneas/Mathlib library cost must be
  measured before choosing the bridge. No claim is raised.

## 12. Status board

| Item | State |
| --- | --- |
| M0 baseline and toolchain inventory | Complete — Rust/Lean/opam/OCaml/GMP/Aeneas/Charon installed; Phase 0 baseline green on the Ubuntu host |
| M1 exact bridge spike | Complete — pure kernel translated to `Exact.lean`; derived/helper bodies failed; Aeneas/Mathlib compile cost outstanding |
| M2 keys bridge spike | Not started |
| M3 codec bridge spike | Not started |
| M4 bridge decision and architecture | Not started |
| M5 IR boundary and D-V6 subset | Not started |
| M6 CI and registry | Not started |
| M7 report and retirement | Not started |
| Owner decision gates | Toolchain install, IR boundary, D-V6, and no-new-claims resolved 2026-11-02; bridge choice, CI adoption, and report acceptance open |
