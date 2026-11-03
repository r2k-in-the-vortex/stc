# Phase 1 Report — Verification Architecture and Bridge

Profile: `stc-2013-oop-utf8-v1`
Date: 2026-11-03
Status: Phase 1 M1–M6 are complete and M7 preparation is complete. This report is submitted for
owner acceptance; Phase 1 is **not yet accepted**, and the temporary `WL-3-Phase-1.md` working
plan is **not retired**. All Phase 1 evidence is V0. The Phase 0 S-level semantics remains the
only reviewed claim.
Companion: [Verification Program Plan](../VerificationProgramPlan.md) ·
[Core IR Boundary](../Core-IR-Boundary.md) · [Semantics](../Semantics.md) ·
[Phase 0 Report](Phase0-Report.md) · [Phase 0 Design](Phase0-Design.md) ·
[Bridge toolchain pins](../../../lean/bridge-toolchain.toml) ·
[Verification registry](../../../lean/verification-status.toml)

## 1. Purpose, scope, and non-claims

Phase 1 is the verification architecture and bridge phase of the program. It decides how Lean
evidence constrains the Rust compiler, builds the bridge and tooling that later phases use, and
connects the frozen Phase 0 reference semantics to the implementation. It produces the bridge and
its evidence; it does not produce large correctness proofs.

The phase delivered:

- a timeboxed bridge spike on three representative targets: the pure exact kernel
  (`crates/stc-codegen/src/exact.rs`), GOT key normalization (`crates/stc-got/src/keys.rs`), and
  the `stc-got` wire codec (`crates/stc-got/src/wire.rs`);
- a per-target bridge decision, not a blanket toolchain choice;
- a bridge architecture that keeps the fast proof build independent of Aeneas/Mathlib, with
  pinned toolchains and source hashes and a manual/nightly drift workflow;
- a frozen full production-module emitted-IR boundary version `1` and the D-V6 first
  end-to-end subset;
- V0 registry evidence (`P1-E01`–`P1-E07`) and CI checks that enforce the pin/hash record without
  installing Aeneas or Mathlib in fast CI.

The phase does **not** claim:

- **Refinement.** Phase 1 is V0 evidence, not a refinement proof. No Rust function is proved
  equivalent to a Lean model, and no generated model is connected to `Stc.Exact` by a theorem.
- **A discharged exact bridge.** The filtered exact model leaves four external operations, and
  their equations are undischarged. The exact bridge remains V0 until the equations, the
  encoding, and the per-operation refinement obligations are proved.
- **Implemented checkers.** The keys and wire contracts fix interfaces and theorem names only.
  Their implementations and proofs belong to Phase 3.
- **An extended Lean IR fragment.** The full Lean `IRFrag`/`WellFormed` extension to the frozen
  boundary belongs to Phase 5; codegen preservation belongs to Phase 7; the D-V6
  `pipeline_correct` theorem belongs to Phase 9.
- **A new claim level.** No component is claimed above V0 except the S-level Phase 0 semantics.
  The Phase 0 `semantics_version` stays `1.0`; no Phase 0 definition, theorem statement, or
  versioned format changed in Phase 1.
- **A checked-in generated artifact.** The generated exact and wire models are not committed as
  of this report. Regeneration is a reviewed manual procedure on the pinned host.

## 2. Execution environment and pinned toolchains

The environment below was recorded on the Ubuntu PREEMPT_RT execution host. Connection details,
credentials, and private addresses are deliberately excluded from tracked files; the host
resource envelope is recorded because every spike result is conditional on it.

| Item | Recorded value |
| --- | --- |
| OS and kernel | Linux 6.12 PREEMPT_RT, x86-64 |
| Host envelope | 3 CPUs, 7.6 GiB RAM, 37 GiB free disk at baseline |
| Disk after Aeneas/Charon build | 9.7 GiB free |
| Proof-environment measurement | 1796 build jobs with the Mathlib cache; 18 GiB free after pruning rebuildable artifacts |
| Rust | `1.99.1` with clippy, rustfmt, llvm-tools, and rust-src |
| Lean | `4.35.0` (`leanprover/lean4:v4.35.0`); Lake `5.0.0-src+eb6cafb` |
| clang | `18.1.3` |
| opam / OCaml / GMP | `2.6.1` / `5.3.0` / `6.3.0` |
| Aeneas | `nightly-2026.11.02-ab34bd2` (`ab34bd2`), Lean backend, user-space install |
| Charon | `0.1.304` (`7b462d9`), `--preset=aeneas` |

The durable pin record is [`lean/bridge-toolchain.toml`](../../../lean/bridge-toolchain.toml).
The Lean and Rust pins are also carried by `lean/lean-toolchain` and `rust-toolchain.toml`; the
drift guard checks that the records agree. Mathlib `v4.35.0` is proof-only and confined to the
separate `lean/aeneas/` package.

The spike inputs are pinned by source hash:

| Target | Source | SHA-256 |
| --- | --- | --- |
| Exact | `crates/stc-codegen/src/exact.rs` | `85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4` |
| Keys | `crates/stc-got/src/keys.rs` | `af80f2d87d4c9f9d6f59310210d5020daccbeb7ef0b02af03d91bf2aa9ad34bc` |
| Wire | `crates/stc-got/src/wire.rs` | `ae840ab12704434b162013fafc85facc2883275dcaefccd2951fd2d09b7810fd` |

Reproducibility rules fixed in Phase 1:

- a generated model carries a header naming the Aeneas and Charon versions and the source path
  and SHA-256;
- regeneration runs only on the pinned Ubuntu host with the pinned binaries, is a reviewed diff,
  and never runs in fast CI or on the Windows editing host;
- a pin change requires owner review and a new bridge spike;
- generated files under `lean/aeneas/StcGenerated/` are never hand-edited.

## 3. Bridge architecture and target decisions

### 3.1 Architecture decisions

| ID | Decision |
| --- | --- |
| M4-D1 | Generated models live in a separate, non-default Lake package `lean/aeneas/`, not under `lean/Stc/`. The default `lake build` at `lean/` never resolves or builds Mathlib/Aeneas. |
| M4-D2 | The filtered exact model is `lean/aeneas/StcGenerated/Exact/Model.lean`; the hand-written `Stc.Exact` model is unchanged and remains the reference. |
| M4-D3 | Generated and hand-written code are separated by directory and module: `StcGenerated/` is generated-only and never edited; hand-written bridge code is in `lean/Stc/Bridge/` and `lean/aeneas/Bridge/`. |
| M4-D4 | The four external axioms are modeled in `Stc.Bridge.Exact.Externals`; each is an equation against the model, named in the TCB and the registry, never an unstated assumption. No V1 claim until they are discharged. |
| M4-D5 | Keys bridge is a proved Lean contract/checker in `Stc.Bridge.Keys`; Aeneas is not used for the string/iterator-heavy module. |
| M4-D6 | Wire codec bridge is a proved Lean checker in `Stc.Bridge.Wire`; the successful Aeneas translation is fallback evidence only. |
| M4-D7 | Pins and source hashes are recorded in `lean/bridge-toolchain.toml`; regeneration is manual/reviewed on the pinned Ubuntu host and never runs in fast CI. |
| M4-D8 | Registry evidence is defined in M4 and added in M6. No Phase 0 registry record or claim changes. |
| M4-D9 | No claim above V0 is made in Phase 1; the Phase 0 S-level semantics remains the only reviewed claim. |

### 3.2 Module layout

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
    lake-manifest.json              # resolved Aeneas/Mathlib revisions
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

Only `lean/aeneas/Bridge/Exact/Refinement.lean` and `Audit.lean` may import generated modules.
The default `Stc` package must not gain a Mathlib or Aeneas dependency, so `lake build`,
`lake exe vectors`, `lake exe stc-sem-tests`, and the registry audit stay fast.

### 3.3 Per-target bridge decisions

The Phase 1 decisions refine D-V1 per target class. Each choice is backed by the spike in §4;
none raises a claim.

| Target class | Preferred bridge | Chosen Phase 1 bridge | Evidence | Rejected or fallback | Claim now |
| --- | --- | --- | --- | --- | --- |
| Pure exact kernel (`exact.rs`) | Aeneas/Charon refinement | Filtered Aeneas/Charon model with hand-written encoding and externals | Filtered model typechecks (`rc=0`); four external axioms enumerated | Whole-file translation (rejected: unsupported helper bodies and `REGISTRY`); extraction (fallback, not needed for the spike) | V0 |
| Key/layout kernel (`keys.rs`) | Aeneas/Charon refinement | Proved Lean normalization contract/checker | Aeneas exits `2` with no model; the chosen path is a checker | Aeneas (not viable as written); source refactor would need a new spike; extraction is a fallback | V0; checker planned |
| Artifact codec (`wire.rs`) | Proved Lean checker on the artifact (V2 path) | Proved Lean wire contract/checker | Aeneas translation succeeds and typechecks; proof cost unmeasured | Aeneas as primary bridge (rejected); the translation is retained as fallback evidence | V0; checker planned |

The exact bridge is additive: no `Stc.Exact` definition or theorem statement changes, so
`semantics_version` stays `1.0`. `Stc.Bridge.Exact.Encode` maps generated representations
(`IntType` with `u32` width and a signedness flag, `Result i128 Trap`, integer values) to
`Stc.Exact.IntType`, `Stc.Exact.Trap`, and `Stc.Exact.Res`. The future refinement obligation is
stated on decoded results, per operation, for example `exact_add_refines`, `exact_div_refines`,
`exact_convert_refines`, and `exact_sel_refines`. The filtered functions are pure and loop-free,
so no harness loop model or fuel bound is expected; if regeneration introduces one, the theorem
must state the fuel obligation explicitly.

### 3.4 The four external axioms

The filtered exact model leaves four operations external. They are not accepted into a claim
as-is; each is restated as an equation against the hand-written model and is either proved or
carried as a named TCB/registry hypothesis.

| External operation | Rust use and domain | Hand-written model | Trust treatment |
| --- | --- | --- | --- |
| `I64::from(u8)` | `IntType.count`; total widening conversion | Natural embedding of the `u8` value into `Int` | Definition replacement with the Aeneas split-file, or carry as a named TCB hypothesis |
| `I64::rem_euclid` | `IntType.count` for shifts and rotates; divisor always positive, so the Rust zero-divisor and `MIN % -1` panics are unreachable | Euclidean remainder `a.emod b` under `0 < b`, result in `[0, b)` | Domain lemma from the kernel control flow, or carry as a named TCB hypothesis |
| `I128::checked_neg` | `neg`; `Option<i128>` semantics | `none` exactly at `i128::MIN`, otherwise `some (-a)` | Definition replacement, or carry as a named TCB hypothesis |
| `I128::abs` | `abs`; called only after the signed `MIN` guard, so the Rust panic case is unreachable | `if a < 0 then -a else a` on the guarded domain | Guard lemma, or carry as a named TCB hypothesis |

While any equation is undisclosed, a fuel obligation is unstated, or the generated model has not
passed the axiom audit, the exact bridge stays V0 and the allowed claim remains
"model-verified; implementation tested against the model".

### 3.5 Keys and wire checker interfaces

The keys checker is the primary bridge for GOT key normalization; it is hand-written Lean and
does not model Rust borrowing, allocations, or iterators. `Stc.Bridge.Keys.Contract` fixes:

- `IdentityKind` with the nine identity tags and their canonical key forms;
- `Key`/`NormalizedKey`, the key grammar predicate, and the normalization relation;
- `normalize : String → Except KeyError NormalizedKey` and
  `validate : IdentityKind → String → Except KeyError Unit`;
- interface theorems `normalize_sound` (every accepted output satisfies the grammar and is
  canonical), `normalize_idempotent`, `validate_sound`, plus an artifact-boundary
  `check_artifact_sound` for Phase 3.

The wire checker is the primary bridge for the codec; the Aeneas `StcGot.lean` translation is
fallback evidence. `Stc.Bridge.Wire.Contract` fixes:

- a byte-buffer state with an explicit position, and little-endian decoders for `u8`, `u16`,
  `u32`, `u64`, `i64`, `i128`, `bool`, `tag`, `id128`, `digest32`, and length-prefixed strings;
- `count` with the untrusted-length bound and `finish` with the no-trailing-bytes rule;
- interface theorems `decode_sound` (a successful decode advances exactly its width and returns
  the specified value), `decode_encode` (round-trip on valid values), `count_bounded`, and
  `finish_sound`.

Neither checker is implemented in Phase 1. A checker proof becomes V2 evidence only when the
checker runs on the artifacts the compiler ships or on every module emitted in CI; fixture-only
runs stay V0/V1.

### 3.6 Proof-engineering policy

- **Pins.** Lean and Rust are pinned in `lean/lean-toolchain` and `rust-toolchain.toml`; the
  bridge pins and source hashes are in `lean/bridge-toolchain.toml`; resolved Aeneas/Mathlib
  revisions belong in `lean/aeneas/lake-manifest.json`. A pin change requires owner review and a
  new bridge spike.
- **Mathlib is proof-only.** It is confined to `lean/aeneas/`; it never enters the default `Stc`
  library, executable models, vector generator, registry audit, or proved checkers.
- **Generated files are not edited.** `StcGenerated/` is generated-only; hand-written proofs live
  under `Bridge/`. Regeneration is a reviewed diff.
- **No `sorry`/`admit`.** Fast CI greps the Phase 0 modules, `Stc/Exact`, and conditionally
  `Stc/Bridge` and `aeneas/Bridge`. A generated `sorry` body is filtered out or recorded as
  opaque; it is never imported into a claim.
- **Axiom audit.** Every top-level theorem cited by a claim is audited with `#print axioms`; the
  only permitted standard axioms are `propext`, `Classical.choice`, and `Quot.sound`. The four
  external bridge axioms are named in the TCB and registry `tcb` field. `native_decide` is not
  used in a trusted theorem.
- **Registry.** Phase 1 records are V0 evidence with status `tested` (spikes and pins) or
  `planned` (checker interfaces); none is `proved` and none claims refinement.
- **Claims.** No claim above V0 in Phase 1.

## 4. Spike results

The spikes are timeboxed feasibility measurements. A failed translation is a valid result and
is recorded as such; no unverified model was written to replace it.

### 4.1 Exact kernel spike (M1)

The source is `crates/stc-codegen/src/exact.rs`, SHA-256
`85f3fbfc5263c918f80fc52b46322510de6645bef66c3fc078c7ba4ee826cba4`, 738 lines.

| Run | Charon | Aeneas | Result |
| --- | --- | --- | --- |
| Whole file | `--preset=aeneas`, edition 2024, compiled as a library; 2.3 MB LLBC with 94 transparent functions, 55 opaque functions, and 217 globals | `rc=1`; 1708-line `Exact.lean` with 119 `def`s and 22 `axiom`s | Rejected. Unsupported derived `PartialEq`/`Debug` impls, `MINMAX_TYPES`, `comparison`, `shift`, `bits`, `unary`, `binary`, the `mux` closure, and `REGISTRY::{promoted_const}` failed; the generated file has a hard `REGISTRY` error (`Array.to_slice` implicit `n`) and emits the unsupported helper bodies as `sorry`. |
| Filtered kernel | Roots for `add`, `sub`, `mul`, `neg`, `div`, `modulo`, `abs`, the bitwise/shift/rotate functions, `compare`, and `sel`; 502 KB LLBC | `rc=0`; 12.8 KB `Exact.lean` | Accepted as the exact-bridge slice. `lake env lean` typechecks the generated file (`rc=0`). All core arithmetic, conversion, and selection functions are present. |

The filtered model's only remaining proof modeling surface is the four external axioms in §3.4.
The whole-file output is not accepted as evidence.

### 4.2 Keys spike (M2)

Charon `0.1.304` with `--preset=aeneas --start-from stc_got::keys` compiled `stc-got` and produced
a 6.8 MB LLBC (271 prepasses). Aeneas `ab34bd2` returned `rc=2` and produced no Lean file.
The recorded blockers were:

- early returns inside loops (`stc_got::digest::parse_hex`);
- unsupported `str::rsplit_once` and `str::strip_suffix` signatures;
- a higher-ranked lifetime constraint in the slice iterator `find`;
- an internal Aeneas `CFailure`.

The start-from filter did not isolate the key functions from these dependencies. Aeneas is not
viable for the keys module as written; the chosen bridge is the proved Lean normalization
contract/checker (§3.5). A source refactor or an extraction fallback would require a new spike.
No claim is raised.

### 4.3 Wire codec spike (M3)

Charon `0.1.304` with `--preset=aeneas --start-from stc_got::wire` and keys/descriptor/manifest
opaque produced a 1.3 MB LLBC. Aeneas `ab34bd2` succeeded (`rc=0`, 3.16 s) and generated a 49 KB
`StcGot.lean` with 54 transparent functions and 65 opaque functions. `wire.Encoder`/`Decoder` and
the primitive `u8`/`u16`/`u32`/`u64`/`i64`/`i128`/`bool`/`tag`/`id128`/`digest32`/`string`/
`list_header` functions are present. The generated file typechecks in the pinned proof
environment (`rc=0`).

Aeneas warned that the crate contains extracted external/unknown definitions and recommends
`-split-files` so they can be supplied explicitly. The translation is fallback evidence, not the
chosen bridge; the artifact-codec decision matrix prefers a proved Lean checker (§3.3), and the
wire proof-obligation cost is not yet measured. No claim is raised.

### 4.4 Proof environment build and cost

The Aeneas Lean backend requires Mathlib `v4.35.0`. The proof environment was built on the pinned
Ubuntu host in 1796 jobs using the Mathlib cache; after pruning rebuildable artifacts, 18 GiB
remained free. That environment is what typechecks the filtered exact model and the wire
translation.

The cost is deliberately outside fast CI. After M6 added `Verification.Bridge.Evidence`, the
default `lake build` grows to 66 jobs and still installs no Aeneas or Mathlib. Full Aeneas/Charon
regeneration remains a reviewed manual procedure on the pinned host; the spike did not establish
a proof-cost budget for either generated model, so no proof-cost claim is made.

## 5. Emitted-IR boundary and D-V6 subset

M5 froze the full production-module boundary in
[`Core-IR-Boundary.md`](../Core-IR-Boundary.md) at boundary version `1`. The boundary was derived
from the emitter and the committed snapshot/verifier tests, not from LLVM documentation. It
records:

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
  and process-image and vtable access as later-phase forms;
- the seven entry-point signatures, the descriptor/function-table/metadata shapes, and the
  control-flow and UB policy;
- explicit construct and build-form exclusions: test builds, debug builds, launcher executables,
  atomics, volatile, `invoke`/`landingpad`, `bitcast`/`inttoptr`, vectors, dynamic `alloca`,
  `switch`, inline asm, and the never-emitted intrinsics;
- versioning and change-control rules tying additions to the boundary document, the P0.4 table,
  the Lean model, and the verifier.

The D-V6 first end-to-end subset is fixed exactly as owner-approved: scalar types, expressions,
assignments, control flow, functions, programs, and GOT globals; no OOP, references, strings, or
aggregates. The boundary document maps it to [Semantics.md](../Semantics.md) §1 and the P0.4
[Core IR Coverage](../Core-IR-Coverage.md) tables and assigns every deferred feature to its
owning phase.

Phase 1 adds no Lean `IRFrag`/`WellFormed` slice. The reviewed Phase 0 fragment stays
authoritative for the core; Phase 5 owns the Lean extension to the full boundary; Phase 7 owns
codegen preservation; Phase 9 owns the D-V6 theorem. The Phase 1 executable evidence remains the
Rust structural verifier and the clang acceptance tests (V0).

## 6. Registry and CI evidence

### 6.1 Registry

At this report the live registry in [`lean/verification-status.toml`](../../../lean/verification-status.toml)
contains 52 records: 42 proved, 8 tested, 1 planned, and 1 descoped. The P0.7 freeze-time count of
45 remains the Phase 0 audit result; the live total grows when later phases append records.
`tools/ci/check_verification_registry.py` reports the live total.

Phase 1 adds the seven `P1` records. Every one is `kind = "evidence"`, `level = "V0"`, and
status `tested` or `planned`; the checker rejects a proved, higher-level, or non-evidence Phase 1
record.

| ID | Status | Evidence |
| --- | --- | --- |
| P1-E01 | tested | The filtered exact model typechecks under the pinned Aeneas/Mathlib environment (`rc=0`); the whole-file output is rejected. |
| P1-E02 | tested | The four external axioms are enumerated and their statements reviewed; none is discharged. |
| P1-E03 | tested | Exact/keys/wire source SHA-256 values match the spike inputs. |
| P1-E04 | tested | Keys Aeneas translation fails (`rc=2`; loops, string, and lifetime errors); the checker/contract is the chosen fallback. |
| P1-E05 | tested | Wire Aeneas translation succeeds (54 transparent, 65 opaque functions) and typechecks (`rc=0`); fallback evidence only. |
| P1-E06 | tested | `lean/bridge-toolchain.toml` matches the environment used for the spikes. |
| P1-E07 | planned | Keys and wire checker contracts/interfaces are defined; implementation and proofs are Phase 3. |

`lean/Verification/Bridge/Evidence.lean` (module `Verification.Bridge.Evidence`) is hand-written
core Lean and part of the default package. It carries the toolchain pin values, the three source
pins, the three translation results, the four external-axiom records, and the planned checker
interfaces. It adds no Mathlib/Aeneas import and no theorem. The registry checker resolves a
record's module in the default `lean/` package first and then in the separate `lean/aeneas/`
package, so a future generated-model record can cite `StcGenerated.*` without moving the file
under `Stc/`.

### 6.2 Fast CI

The fast `lean` job is the only required proof job. Its budget is the existing 30-minute timeout
on `ubuntu-24.04`; the docs job stays at 10 minutes. The job runs:

- the default `lean/` build;
- the `sorry`/`admit` checks for the Phase 0 modules, `Stc/Exact`, and conditionally
  `Stc/Bridge` and `aeneas/Bridge`;
- the registry and axiom audit;
- the vector-regeneration diff;
- the differential oracle on the core corpus;
- `lake exe stc-sem-tests`.

It installs neither Aeneas nor Mathlib. Splitting these checks into separate jobs was rejected in
M6 because each split would rebuild the Lean package without adding coverage. The docs job runs
`check_docs.py` and markdownlint.

### 6.3 Bridge-drift workflow and operational prerequisites

[`bridge-drift.yml`](../../../.github/workflows/bridge-drift.yml) is a separate manual/nightly
workflow and is never part of the required CI matrix.

- The nightly schedule is `23 3 * * *`, and `workflow_dispatch` is always available.
- The `drift` job runs on `ubuntu-24.04` with a 10-minute timeout. It runs
  `python3 tools/ci/check_bridge_toolchain.py`, which validates the pin-record schema and profile,
  the Lean/Rust pin consistency, the `semantics_version` match with the registry, the three
  source SHA-256 values, and generated-file headers when generated models exist. It needs no
  OCaml, Lean proof environment, or Mathlib.
- The manual `full` input adds the `pinned` job on `runs-on: [self-hosted, linux, x64,
  stc-bridge]` with a 60-minute timeout. That job runs the guard with `--check-environment` to
  compare the installed Lean and Rust versions and to locate the Aeneas/Charon binaries. Their
  version output is reported and must be confirmed manually; the pinned builds do not have a
  parseable recorded version format.

Operational prerequisites, provisioned outside the repository:

- a self-hosted runner labelled `stc-bridge` matching the pinned Ubuntu bridge host, with the
  pinned Lean, Rust, Aeneas, and Charon installations available;
- repository variables `STC_AENEAS_BIN` and `STC_CHARON_BIN` naming the pinned Aeneas and Charon
  binaries; the workflow maps them to the `AENEAS` and `CHARON` environment variables that the
  guard reads.

The `pinned` job is skipped unless it is dispatched manually with `full=true`. Full Aeneas/Charon
regeneration stays a reviewed manual procedure on the pinned host and never runs on a
GitHub-hosted runner. The generated exact and wire models are not checked in as of this report;
the guard reports their absence as a notice, and `--require-generated` turns that into an error
once a model is declared and expected. The cheap guard is also part of
`tools/ci/check_local.py`, so local runs do not need the proof environment.

The source hash is the drift trigger: a bridge source change without a pin update or regeneration
fails the drift check. No Phase 1 record or check raises a claim; all remain V0.

## 7. Trust boundary and claim language for Phase 1

The Phase 0 TCB is unchanged: the Lean kernel and pinned toolchain, the repository definitions,
Batteries for executable models, the versioned dump/case/report inputs, and the Rust/clang/OS/
hardware oracle. Phase 1 adds the following bridge-specific TCB surface:

- the pinned Aeneas/Charon toolchain and its Mathlib `v4.35.0` proof environment;
- a generated model only after the pinned regeneration, typecheck, axiom audit, and reviewed
  diff;
- the four external axiom equations while any remains undischarged;
- the source hash that ties the generated model to the Rust source.

The permitted Phase 1 claim is V0: for the exact kernel, "model-verified; implementation tested
against the model". The keys and wire paths remain planned checkers with no implementation. The
following claims are **not** permitted in Phase 1:

- "implementation proved equivalent to the model" (V1);
- "artifact validated by a proved checker" (V2);
- "verified code in the trusted path" (V3);
- "verified compiler for the defined subset" (V4);
- any statement that the four external axioms are discharged, the keys/wire checkers exist, or
  the full Lean IR fragment has been extended.

A generated axiom is never accepted as an unstated assumption. If an equation is carried as a
hypothesis, it is listed in the TCB and the registry `tcb` field.

## 8. Residual risks and follow-ups

| Risk or follow-up | Owner and timing | Mitigation and current state |
| --- | --- | --- |
| Four external exact axioms undischarged | Phase 2 | Each has a hand-written model and a discharge-or-hypothesis plan (§3.4); no V1 claim until discharged. |
| Exact refinement obligation not written | Phase 2 | `Stc.Bridge.Exact.Encode`, `Externals`, and the per-operation refinement theorems are fixed by name; no theorem exists yet. |
| Generated exact model not checked in | Phase 2 regeneration | Regeneration is manual/reviewed on the pinned host; the drift guard reports the absence and `--require-generated` can enforce it later. |
| Keys checker implementation and proofs absent | Phase 3 | Interfaces and theorem names are fixed in §3.5; Aeneas is not used for this module. |
| Wire checker implementation and proofs absent; proof cost unmeasured | Phase 3 | The Aeneas translation is fallback evidence only; the checker is the chosen bridge; no proof-cost claim is made. |
| Full Lean `IRFrag` extension absent | Phase 5 | Boundary version `1` is frozen; Phase 5 extends the model to exactly that boundary. |
| Codegen preservation absent | Phase 7 | Phase 0 `IRFrag`/`IRStep` and the boundary frame the obligation; no preservation claim exists. |
| D-V6 end-to-end theorem absent | Phase 9 | The subset scope is fixed in the boundary document §11; `pipeline_correct` is not started. |
| Pinned host capacity (3 CPUs, 7.6 GiB RAM) | Ongoing | Use `-j2`, monitor memory and disk, prune rebuildable artifacts, and run long jobs overnight with logs. |
| Self-hosted runner or repository variables missing | Owner/infrastructure | The nightly `drift` job still runs on a GitHub-hosted runner; the manual `full` audit is skipped until the `stc-bridge` runner and `STC_AENEAS_BIN`/`STC_CHARON_BIN` variables exist. |
| Aeneas/Charon version output not machine-parsed | `bridge-drift` operator | The guard reports the output and requires manual confirmation of the pinned build. |
| Generated model drifts from Rust | Bridge drift | Source SHA-256 pins, pinned toolchain, header checks, and the nightly drift job; regeneration is a reviewed diff. |
| Claim inflation | All phases | Registry enforces V0 evidence for `P1`; no proved or higher-level Phase 1 record is accepted. |
| Credential or host-detail leakage | All phases | Connection details and credentials stay outside tracked files; this report records only the resource envelope. |

## 9. Exit criteria and Phase 1 review checklist

### 9.1 Exit criteria assessment

| Program-plan exit criterion | Result |
| --- | --- |
| A bridge spike report exists | Met by this report. |
| D-V1 through D-V10 are resolved | D-V1 is resolved per target (§3.3); D-V2, D-V3, D-V4, D-V5, D-V6, D-V7, D-V8, and D-V9 are recorded in the program plan §12; D-V10 remains a standing non-goal. |
| The Phase 0 semantics is the reference for the bridge | Met; the exact bridge targets `Stc.Exact`, and no `1.0` definition or theorem changed. |
| The registry, no-`sorry` check, vector-regeneration check, and oracle harness run in CI | Met; see §6.2. |
| The full IR fragment and the first end-to-end subset are written down | Met; boundary version `1` and D-V6 in `Core-IR-Boundary.md`. |
| No component is claimed above V0 except the S-level Phase 0 semantics | Met; 52 records with the seven V0 `P1` evidence records and no V1/V2 record. |

### 9.2 Review checklist

The checklist below is the proposed review scope for the owner's acceptance decision.

| Area | Check |
| --- | --- |
| Report | This report records the purpose, scope, non-claims, environment and pins, bridge decisions, spike results, proof-environment cost, IR boundary and D-V6 subset, registry/CI evidence, residual risks, and the review checklist. |
| Bridge decisions | Each target class has a chosen bridge, a rejected alternative, and evidence; no choice claims V1/V2. |
| Toolchain pins | `lean/bridge-toolchain.toml` matches `lean/lean-toolchain` and `rust-toolchain.toml`; the three source SHA-256 values match; the drift guard passes (`0` generated models checked in, `3` source pins match). |
| Registry | 52 live records (42 proved, 8 tested, 1 planned, 1 descoped); `P1-E01`–`P1-E07` are V0 evidence; no status inflation. |
| Proof audit | `lake build` passes; no `sorry`/`admit`; the axiom audit reports only the standard axioms for proved records; `Verification.Bridge.Evidence` adds no theorem. |
| CI | Fast CI is toolchain-free and within the 30-minute `lean`/10-minute docs budget; the separate drift workflow is not in the required matrix; its runner and variable prerequisites are documented. |
| IR boundary | Boundary version `1` is frozen, traceable to the emitter and tests, and has explicit exclusions; D-V6 is fixed; Phase 5/7/9 ownership is stated. |
| Claim boundary | Phase 1 is V0 evidence, not a refinement proof; no component is claimed above V0 except the Phase 0 S-level semantics; the allowed and disallowed claim language is explicit. |
| Owner decisions | Phase 1 report acceptance and retirement of the temporary `WL-3-Phase-1.md` working plan remain open. |

**Owner acceptance is pending.** This report does not retire the working plan; retirement
requires the owner's acceptance of Phase 1 and a separate approval to delete the working file.

## 10. References

- [Verification Program Plan](../VerificationProgramPlan.md)
- [Core IR Boundary](../Core-IR-Boundary.md)
- [Core IR Coverage](../Core-IR-Coverage.md)
- [Semantics](../Semantics.md)
- [Semantics Charter](../Semantics-Charter.md)
- [Phase 0 Report](Phase0-Report.md)
- [Phase 0 Design and Decision Record](Phase0-Design.md)
- [Bridge toolchain pins](../../../lean/bridge-toolchain.toml)
- [Verification registry](../../../lean/verification-status.toml)
- [Bridge evidence module](../../../lean/Verification/Bridge/Evidence.lean)
- [Bridge drift guard](../../../tools/ci/check_bridge_toolchain.py)
- [Bridge drift workflow](../../../.github/workflows/bridge-drift.yml)
- [Testing and Verification](../../Architecture/Testing-and-Verification.md)
