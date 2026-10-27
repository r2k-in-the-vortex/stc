# GOT Artifacts and Schemas

Status: Format 1 and the M5 OOP descriptor/interface records implemented (2026-10-25); M6 host ABI 2.2 test-clock append and project/state/I/O integration implemented (2026-10-27); M6.5 phase 0 fixes debug service ID 13 at ABI 2.3, the checkpoint-table location, and the hook feature/entry bits (2026-10-27); M6.5 implementation pending  
Profile: `stc-2013-oop-utf8-v1`  
Scope: Normative formats for stable keys, `GotId`, the compiler catalog, the module descriptor,
the persistent GOT manifest, the online-change plan, host ABI 2 (2.2 implemented; 2.3 adds the
debug service; production modules require 2.1), and build IDs.

This document is the machine-readable companion to
[Online Change and GOT](Online-Change-and-GOT.md). It fixes the bytes and identifiers that the
host model, preservation policy, and lifecycle operate on. It supersedes the informal descriptor,
host API, and manifest sketches in `update_plan_got.md` §6.4 and §6.9; the summaries in
[Runtime and Execution](Runtime-and-Execution.md) §2.3.1–§2.3.2 are normative only through the
formats here.

M4.5 populates the schemas for the M4 scalar surface. M5 extends the same formats with aggregate
roots, object/type descriptors, method/vtable entries, and place descriptors. No format version
is bumped merely because a record table gains entries: the record layouts below are the complete
v1 layouts, and M4.5 simply leaves later record kinds empty.

## 1. Version inventory

| Item | Version | Constant |
| --- | --- | --- |
| Stable key grammar | 1 | `STC_KEY_V1` |
| `GotId` derivation | 1 | `STC_GOTID_V1` |
| Compiler catalog | 1 | `STC_CATALOG_V1` |
| Type-key/layout derivation | 1 | `STC_LAYOUT_V1` |
| Module descriptor | 1 | `STC_DESCRIPTOR_V1` |
| Loadable module (container contract) | 1 | `STC_MODULE_V1` |
| Host API | 2.2 implemented; 2.3 adds debug service 13 (M6.5, planned) | `STC_HOST_ABI_MAJOR=2`, `STC_HOST_ABI_MINOR=3` when service 13 is used |
| GOT manifest | 1 | `STC_MANIFEST_V1` |
| Online-change plan | 1 | `STC_PLAN_V1` |
| Build ID derivation | 1 | `STC_BUILD_V1` |

Format numbers are stored explicitly. A reader accepts only the exact format numbers it
implements. Changing any field order, field meaning, digest input, or ordering rule requires a
new format number for the affected artifact. Additive host API entries are the only compatible
change within a major version; they bump the host API minor version.

## 2. Common encoding

### 2.1 Primitive binary encoding

All binary artifacts are byte-packed with no implicit alignment padding. Multi-byte integers are
little-endian. The primitive encodings are:

| Notation | Encoding |
| --- | --- |
| `u8`, `u16`, `u32`, `u64` | Unsigned integer of that width |
| `i64` | Two's-complement signed 64-bit integer |
| `bool` | `u8`: `0` false, `1` true; any other value is invalid |
| `enum` | `u8` tag; only the tags listed for that enum are valid |
| `id128` | 16 raw bytes, most significant byte first when rendered |
| `digest32` | 32 raw bytes, SHA-256 output order |
| `string` | `u32` byte length followed by that many UTF-8 bytes; no terminator |
| `list<T>` | `u32` element count followed by the elements in order |
| `option<T>` | `u8` tag (`0` absent, `1` present) followed by `T` when present |
| `record` | The listed fields in the listed order, concatenated |

`string` values must be valid UTF-8. Stable keys, profile IDs, target triples, and C export names
are additionally restricted to printable ASCII by the rules that define them. A length or count
that exceeds the remaining bytes of the containing buffer is invalid; readers never allocate from
an untrusted count before checking it against the remaining input.

Sectioned binary data uses:

```text
section := tag:u16 length:u32 payload:[length]u8
```

Tags with the high bit (`0x8000`) set are unknown-critical: a reader that does not understand
the tag rejects the artifact. Unknown tags without that bit are skipped. Section payloads are
records or lists of records as defined for each section. Sections appear in ascending tag order,
at most once each, unless a section is explicitly optional.

### 2.2 Canonical JSON

The JSON projection of the manifest is canonical so two builds are byte-identical. It is written
as UTF-8 without a byte-order mark, with LF line endings, two-space indentation, no trailing
spaces, and exactly one trailing LF. Object keys are sorted by ascending UTF-8 byte order.
Arrays keep the order defined for their table; they are never re-sorted by a JSON writer.

Numbers are integers only: no leading `+`, no leading zeroes (except `0` itself), no `-0`, no
fraction, and no exponent. Sizes, counts, and offsets use decimal; identifiers and digests use
`0x`-prefixed lowercase hexadecimal strings. Floating-point values do not appear.

Strings use the minimal escaping needed: `\"`, `\\`, `\n`, `\r`, `\t`, and `\u00XX` (uppercase
hexadecimal) for the remaining control characters below U+0020. Non-ASCII characters are written
as raw UTF-8, not `\u` escapes. Solidus is not escaped.

Every field defined by a record is present. An absent option is `null`; an empty list is `[]`.
Unknown fields are invalid. This makes the JSON projection fully equivalent to the binary body.

Enums are rendered in JSON as the tag name converted to snake_case by inserting an underscore
before each interior uppercase letter and then lowercasing: `ProgramField`, `OnlineCandidate`,
`TypeInfo`, and `Drop` become `"program_field"`, `"online_candidate"`, `"type_info"`, and
`"drop"`.
The numeric tags in the record definitions are the binary projection only. A reader accepts only
the listed names. Booleans are `true`/`false`; `option` values use `null` for absence.

### 2.3 Identifiers and digests

`GotId` and `build_id` values are 128-bit and are rendered as `0x` followed by exactly 32
lowercase hexadecimal digits. Digests are SHA-256 outputs and are rendered as `0x` followed by
exactly 64 lowercase hexadecimal digits. File names that embed an identifier use the 32
hexadecimal digits without the `0x` prefix.

All hash inputs begin with an ASCII domain string followed by a NUL byte, for example
`stc:gotid:v1\0`. Domain separation prevents a hash computed for one purpose from being accepted
for another. Hash inputs are the exact byte concatenations defined in this document; no
serializer, locale, or platform newline conversion participates.

### 2.4 Validation limits

The formats do not impose a maximum artifact size in v1, and section lengths are `u32` in the
manifests below. Readers must nevertheless enforce resource limits derived from the input size:
string lengths, list counts, section lengths, and record sizes are checked against the remaining
buffer before any allocation; nesting is not used in these formats. A reader rejects an artifact
that claims more data than it contains or that would exceed its configured memory budget.
Malformed input is a validation error (`STC-E-MANIFEST-002` or `STC-E-MODULE-001`), never a
panic.

## 3. Stable keys

### 3.1 Normalization

Stable keys are ASCII and case-insensitive-normalized. Each component is derived as follows:

- **Package name:** `[package].name` uppercased with ASCII case folding. Names already match
  `[A-Za-z_][A-Za-z0-9_-]*`; `-` is preserved. A manifest-free M4 bridge build uses the package
  name `_ROOT`.
- **Identifier:** the source spelling uppercased with ASCII case folding. Identifiers are
  `[A-Za-z_][A-Za-z0-9_]*` (STC-LEX-012), so no other escaping is needed.
- **Name path:** normalized identifiers joined with `.`.
- **Instance path:** normalized identifiers joined with `/`. For a configured program instance it
  is `CONFIGURATION/RESOURCE/INSTANCE`; nested object instances append further `/`-separated
  segments.
- **Signature key:** defined in §4.2.
- **Type key:** defined in §4.1.

A key never contains a source file path, a local dependency alias, an arena ID, a slot number, or
an absolute path. The package name is the identity scope, not the importing project's alias.

### 3.2 Key forms

A stable key has the form:

```text
<PACKAGE>::<PATH>[#<INSTANCE>][@<SIGNATURE>][#<DETAIL>]
```

with exactly one form per identity kind:

| Identity kind | Key form | Example |
| --- | --- | --- |
| Global data root | `PKG::PATH` | `MOTOR_CONTROL::GLOBALS.SPEED` |
| Program/instance field root | `PKG::PATH#CONFIG/RESOURCE/INSTANCE` | `MOTOR_CONTROL::MAIN.SPEED#MAIN/R1/INST` |
| Nested field root | `PKG::PATH.SUBPATH#CONFIG/RESOURCE/INSTANCE[/NESTED]` | `MOTOR_CONTROL::MAIN.STATE.SUB#MAIN/R1/INST/FB1` |
| Hidden object-record root | `PKG::PATH#OBJREC` or `PKG::PATH#INSTANCE/OBJREC` | `MOTOR_CONTROL::MAIN.TIMER#MAIN/R1/INST/OBJREC` |
| Hidden class-handle root | `PKG::PATH#HANDLE` or `PKG::PATH#INSTANCE/HANDLE` | `MOTOR_CONTROL::MAIN.MOTOR#MAIN/R1/INST/HANDLE` |
| Task guard root | `PKG::CONFIG/RESOURCE#TASK.GUARD` | `MOTOR_CONTROL::MAIN/R1#FAST.GUARD` |
| Function | `PKG::PATH@SIGNATURE` | `MOTOR_CONTROL::MATH.ADD@(IN:DINT,IN:DINT)->DINT` |
| Program body | `PKG::TYPE#BODY@()->VOID` | `MOTOR_CONTROL::MAIN#BODY@()->VOID` |
| Type | `PKG::PATH` | `MOTOR_CONTROL::TYPES.MOTOR` |
| Type field identity | `PKG::TYPE_PATH.FIELD_PATH` | `MOTOR_CONTROL::TYPES.MOTOR.SPEED` |
| Descriptor | `PKG::TYPE_PATH#DESC:<KIND>` | `MOTOR_CONTROL::TYPES.MOTOR#DESC:VTABLE` |
| Instance | `PKG::CONFIG/RESOURCE/INSTANCE` | `MOTOR_CONTROL::MAIN/R1/INST` |
| Task | `PKG::CONFIG/RESOURCE#TASK` | `MOTOR_CONTROL::MAIN/R1#FAST` |
| Export | `PKG::PATH@SIGNATURE#C:<C_NAME>` | `MOTOR_CONTROL::API.READ@()->DINT#C:stc_read` |

`<C_NAME>` preserves the case-sensitive C identifier after the fixed `C:` marker. The `#BODY`,
`#DESC:`, `.GUARD`, `#HANDLE`, and `#OBJREC` markers use uppercase spellings. `HANDLE` and
`OBJREC` are reserved final detail segments for compiler-generated hidden roots (§8.5.6); they
follow the `#`, never a `.`, so no source identifier can produce one. A renamed declaration keeps
its `GotId` through a tracked rename or move even though its key changes; the old key is retained
in `previous_keys`.

A key is unique per identity kind. Two declarations may not share a key and kind. Functions with
different signatures have different keys because the signature is part of the key; overloads
therefore never collide.

### 3.3 Matching

Baseline matching is deterministic and uses no heuristics:

1. Apply explicit tracked rename/move maps from the build (`--rename OLD=NEW`). A map entry names
   a key, or a type field path for a field rename that applies to every instance of that type.
   The matched record carries the baseline `GotId` forward and records its old key in
   `previous_keys`.
2. Otherwise match an active identity by exact `(kind, key)` against the baseline.
3. Otherwise it is a new identity and receives a freshly derived `GotId`.
4. Baseline data-root identities with no match are removals and become `Drop` actions. They
   require `--allow-untracked-removal` or `--cold`; otherwise the build fails with
   `STC-E-MANIFEST-003`. A removed function, type, or descriptor is retired without storage
   quarantine and does not require the flag. Direct-variable identities never produce a required
   drop; a removed export is a rejection (§10).
5. A key match with a different identity kind is a rejection (`STC-E-MANIFEST-002`). For
   functions, a changed signature produces a new function identity and a removal of the old one.
6. Name similarity, type equality, source diffing, and ordinal matching are never used.

Data-root and type identities exclude the type and layout from the key, so a type or layout
change keeps the identity and is classified separately. Function identities include the
signature. A tracked rename that changes a data root's type is a layout change, not a new
identity.

## 4. Type keys, signatures, and layout hashes

### 4.1 Type keys

A type key is a canonical structural description independent of source spelling, alias names,
and target layout:

```text
typekey :=
    "BOOL" | "SINT" | "INT" | "DINT" | "LINT"
  | "USINT" | "UINT" | "UDINT" | "ULINT"
  | "REAL" | "LREAL"
  | "TIME" | "LTIME" | "DATE" | "LDATE" | "TOD" | "LTOD" | "DT" | "LDT"
  | "BYTE" | "WORD" | "DWORD" | "LWORD"
  | "CHAR" | "WCHAR" | "UCHAR"
  | "STRING[" capacity "]" | "WSTRING[" capacity "]" | "USTRING[" capacity "]"
  | "ARRAY[" bound ".." bound "]OF(" typekey ")"
  | "REF_TO(" typekey ")" | "PTR_TO(" typekey ")"
  | "T:" ID_HEX
  | "VOID"
```

- `capacity` and the array bounds are decimal integers; a conformant-array bound is `*`.
- `T:<ID_HEX>` denotes a named user-defined type (struct, FB, class, interface, enum, subrange)
  by its type `GotId` in 32 lowercase hexadecimal digits without `0x`. Aliases are transparent:
  a root declared through an alias uses the target type key. A tracked type rename keeps
  `T:<ID_HEX>` stable because the type `GotId` is carried forward.
- `UCHAR` is one 32-bit Unicode scalar value; `USTRING[n]` is `n` 32-bit scalars (A-005).
- `PTR_TO` is the transient-only `STC-POINTER` extension. A persistent root may never have a
  `PTR_TO` type key, with one exception: a hidden class-handle or object-record root (§8.5.6)
  carries the compiler-internal pointer/record key, is never retained, and is not declared by
  source.

### 4.2 Signature keys

A signature key describes one callable's interface:

```text
signature := "(" [ parameter *( "," parameter ) ] ")->" result
parameter := mode ":" typekey
mode      := "IN" | "OUT" | "INOUT"
result    := typekey
```

Spaces do not appear. Parameters appear in declaration order. The receiver (`THIS`) is not a
parameter because the owner is part of the key. Function-block body signatures use the FB's
input/output/in-out parameter list. A program body has the fixed signature `()->VOID`.
Standard functions use their resolved concrete parameter types, not an overload name.

### 4.3 Layout hashes

A layout hash identifies the target-specific storage shape of a type or root:

```text
layout_hash =
  SHA-256("stc:layout:v1\0" || target_triple || 0x00 || endianness:u8 ||
          pointer_width:u8 || typekey || 0x00 || layout_bytes)[0..16]
```

`layout_bytes` is the canonical shape description:

| Shape | Encoding |
| --- | --- |
| Elementary | `0x01 \|\| size:u32 \|\| align:u32 \|\| encoding:u8` |
| String | `0x02 \|\| capacity:u32 \|\| unit_bytes:u8 \|\| size:u32 \|\| align:u32` |
| Array | `0x03 \|\| element_count:u64 \|\| stride:u64 \|\| typekey \|\| 0x00 \|\| element_layout_hash:id128` |
| Struct/FB/class | `0x04 \|\| size:u32 \|\| align:u32 \|\| layout_hash:id128` |
| Overlap | `0x05 \|\| size:u32 \|\| align:u32 \|\| union_layout_hash:id128` |
| Reference/pointer | `0x06 \|\| kind:u8 \|\| typekey \|\| 0x00` |

The elementary `encoding:u8` tags are fixed:

| Tag | Type keys |
| --- | --- |
| 1 | `BOOL` |
| 2 | `SINT`, `INT`, `DINT`, `LINT` |
| 3 | `USINT`, `UINT`, `UDINT`, `ULINT` |
| 4 | `BYTE`, `WORD`, `DWORD`, `LWORD` |
| 5 | `REAL` |
| 6 | `LREAL` |
| 7 | `TIME`, `LTIME` |
| 8 | `DATE`, `LDATE` |
| 9 | `TOD`, `LTOD` |
| 10 | `DT`, `LDT` |
| 11 | `CHAR` |
| 12 | `WCHAR` |
| 13 | `UCHAR` |

For the M4.5-supported targets the elementary size/alignment pairs are the natural ones:
`BOOL`/`CHAR` 1/1, `WCHAR` 2/2, `UCHAR`/`REAL`/32-bit types 4/4, `LREAL`/64-bit integer and
time/date types 8/8, and 8-bit/16-bit integer and bit-string types at their widths. The encoding
tag is descriptive; the type key is also hashed, so the tag is not the sole layout discriminator.

For struct/FB/class and overlap shapes, `layout_hash`/`union_layout_hash` is the SHA-256 of the
canonical field/view list: each entry is `path || 0x00 || offset:u64 || size:u64 || align:u32 ||
typekey || 0x00 || field_layout_hash:id128`, sorted by `(offset, path)` and concatenated. This
keeps the top-level hash stable while allowing the manifest's layout table to describe fields.
M4.5 uses only the elementary and string rows; the other rows are part of format 1 so M5 does not
need a format change.

`layout_hash` is target-specific. A manifest from a different target triple, endianness, or
pointer width is never a valid layout baseline for an online change; it may be used only for
cold-layout identity matching after the layout data is recomputed.

## 5. `GotId` derivation

### 5.1 Input tuple

A `GotId` is derived from an identity tuple:

```text
kind:u8
package:string
path:string
qualifier:string
```

`package` is the normalized package name. `path` and `qualifier` are kind-specific:

| Kind | Tag | `path` | `qualifier` |
| --- | --- | --- | --- |
| Data root | 1 | `PATH`, `PROGRAMTYPE.FIELD[.SUBPATH]`, or `CONFIG/RESOURCE#TASK.GUARD` | instance path, empty for a global or task guard |
| Function | 2 | qualified function/method path, or `TYPE#BODY` | signature key |
| Type | 3 | qualified type path | empty |
| Descriptor | 4 | qualified owner type path | descriptor kind (`VTABLE`, `TYPEINFO`, `OBJECT`) |
| Instance | 5 | `CONFIG/RESOURCE/INSTANCE` | program type path |
| Task | 6 | `CONFIG/RESOURCE` | task name |
| Export | 7 | qualified function path | `SIGNATURE#C:C_NAME` |
| Field | 8 | `TYPE_PATH.FIELD_PATH` | empty |
| Direct variable | 9 | `PATH` or `PROGRAMTYPE.FIELD` | instance path, empty for a global |

### 5.2 Derivation

```text
input  = "stc:gotid:v1\0" || kind:u8 || string(package) || string(path) ||
         string(qualifier)
got_id = SHA-256(input)[0..16]
```

The `string` encoding is the primitive encoding from §2.1, so the fields cannot run together.
The derivation uses only the current identity tuple. A tracked rename preserves the old `GotId`
by copying it, not by rehashing the new key.

### 5.3 Collision and reuse rules

- A build computes the full SHA-256 for every new identity. If two identities with different
  input tuples truncate to the same 128-bit `GotId`, the build fails with
  `STC-E-MANIFEST-002`.
- A baseline manifest is invalid if two records share a `GotId` with different `(kind, key)`
  values.
- A new declaration may reuse a baseline `GotId` only through an explicit tracked rename/move.
  A reintroduced retired key would re-derive the same `GotId` from its content tuple; v1 has no
  reuse/reissue policy, so the build rejects that case with `STC-E-MANIFEST-002` rather than
  resurrecting quarantined storage (§12.1). A key with a new spelling, or a forced cold build,
  produces a fresh layout instead.
- A `GotId` is never derived from a slot number, source location, mangled symbol, `DefId`,
  `FieldId`, or file order.

## 6. Compiler catalog

The compiler builds one catalog model after semantic analysis and layout. Every emitted artifact
is a projection of that model; the model is not read back from an artifact during the same
build.

### 6.1 Catalog records

The catalog contains:

- **Roots:** one record per persistent data root. M4.5 roots are scalar globals and per-instance
  scalar program fields, plus one task-guard root per task. Direct `AT` variables are not
  persistent roots; they use direct cells and a direct identity record. M5 phase 4b.1 adds hidden
  non-retained `HANDLE` roots for class values and `OBJREC` roots for class/FB instances (§8.5.6).
- **Functions:** one record per user function, program body, method, FB body, standard function
  that is called indirectly, and export wrapper. Compiler-generated helpers are module-local and
  do not get identities.
- **Types:** one record per named user-defined type that owns persistent layout or descriptors.
  Elementary types are implicit and are not listed.
- **Descriptors:** one record per runtime-owned type descriptor or vtable. M4.5 emits none;
  M5 phase 4b projects one TYPEINFO per class/FB type and one VTABLE when the type has class
  method slots. Object records are host storage reached through hidden roots and are not
  catalog descriptors (§8.5).
- **Instances:** one record per configured program instance.
- **Tasks:** one record per task plus its guard root.
- **Exports:** one record per `{stc:export}` wrapper.
- **Slots:** one record per host cell, derived from the records above.

The in-memory catalog uses stable IDs and sorted vectors. No hash-map iteration reaches an
emission path. Source locations are recorded as package-relative paths, never absolute paths.

### 6.2 Slot assignment

Every host cell is pointer-sized. Slot numbers are local to one build and are not identities.
Slots are assigned in five groups, in this order:

1. **Data:** persistent root cells, sorted by `GotId` bytes ascending.
2. **Function:** function cells, sorted by `GotId` bytes ascending.
3. **Descriptor:** runtime descriptor/vtable cells, sorted by `GotId` bytes ascending.
4. **Direct:** direct-variable cells, sorted by `GotId` bytes ascending.
5. **Import:** host service cells, sorted by service ID ascending.

Slot numbers are the positions in that concatenation, starting at zero. Within a group no two
records share a `GotId`. An import slot's service ID is defined in §8.2. The ordering is
canonical, so a reader verifies that the slot table is contiguous, grouped, and sorted; a
non-canonical table is invalid even if its bindings happen to work.

### 6.3 Catalog digest

```text
catalog_digest = SHA-256(
  "stc:catalog:v2\0" ||
  roots_section_payload || functions_section_payload || types_section_payload ||
  descriptors_section_payload || instances_section_payload || tasks_section_payload ||
  exports_section_payload || slots_section_payload ||
  type_metadata_section_payload || aggregates_section_payload)[0..32]
```

Each `*_section_payload` is the encoded payload defined in §9.2, without its section tag and
length. The digest covers source locations because they are fields of the records. It does not
cover the manifest header, plan, or history. The format-2 type-metadata and aggregate payloads
are covered too, so a changed enum member, subrange bound, aggregate shape, or aggregate field
layout changes the catalog digest and therefore the build ID even when every root layout is
unchanged (D-180).

## 7. Module descriptor format 1

### 7.1 Blob layout

The descriptor is a self-contained byte blob embedded in module read-only data. It contains no
pointers, relocations, or target addresses; the host copies and parses it. The module exposes it
through `stc_module_descriptor` (§7.3).

```text
magic                [8]u8 = "STCMOD\0\0"
descriptor_format    u16   = 1
descriptor_flags     u16   = 0
descriptor_size      u32          total blob bytes, including this field
required_features    u64          feature bit set the host must implement
host_abi_major       u16   = 2
host_abi_minor       u16   = 1   (minimum; 2 for test builds, 3 for debug hooks)
module_format        u16   = 1
endianness           u8    = 0   (0 = little, 1 = big)
pointer_width        u8    = 8
profile              string       exact profile ID
compiler_version     string       compiler package version, ASCII
target_triple        string       resolved target triple
build_id             id128
manifest_digest      digest32
manifest_format      u16   = 2
entry_flags          u32
function_count       u32
slot_count           u32
functions            list<FunctionRecord>
slots                list<SlotRecord>
```

`descriptor_size` must equal the blob length exactly. `descriptor_flags` is zero in format 1;
nonzero values are invalid. `manifest_format` is the outer format of the paired manifest:
descriptor format 1 accepts manifest format 2 only (D-180). An old host that expects 1 rejects
the pair rather than reinterpreting the manifest. `entry_flags` bits:

| Bit | Meaning |
| --- | --- |
| 0 | `stc_module_prepare` performs root initialization |
| 1 | `stc_module_cycle` is callable |
| 2 | `stc_module_shutdown` performs retained-state work |
| 3 | `stc_module_detach` releases module-local resources |
| 4 | Debug build form (the default non-`--release` artifact). Set by M4.5, M6, and M6.5 debug-form builds; it does not by itself imply debug information or checkpoint hooks |
| 5 | The module uses retained-state host services |
| 6 | Debug hooks are active; equals `required_features` bit 4 (M6.5; always clear in M4.5, M6, test, and release artifacts) |
| 7–31 | Reserved, must be zero in format 1 |

`FunctionRecord`:

```text
got_id          id128
key             string
signature_key   string
flags           u32
```

`FunctionRecord` flags:

| Bit | Meaning |
| --- | --- |
| 0 | User function |
| 1 | Program body |
| 2 | FB body |
| 3 | Method |
| 4 | Export wrapper |
| 5 | Standard function called indirectly |
| 6 | Indirect-callable (host may bind a function cell to this entry) |
| 7–31 | Reserved, zero |

Records are sorted by `GotId` bytes ascending. The function array returned by
`stc_module_functions` has the same length and order as this list.

`SlotRecord`:

```text
slot     u32
kind     enum { Data=1, Function=2, Descriptor=3, Direct=4, Import=5 }
service  u8          import service ID; zero for other kinds
got_id   id128       zeros for Import; nonzero for other kinds
```

Records are sorted by `slot` ascending and must be exactly the canonical assignment in §6.2.
`slot_count` equals the number of records. Every `Function` slot's `GotId` appears exactly once
in the function list; every `Data`/`Direct` slot's `GotId` appears in the manifest root table;
every `Descriptor` slot's `GotId` appears in the manifest descriptor table.

`required_features` bits for format 1:

| Bit | Meaning |
| --- | --- |
| 0 | Host ABI 2 base services |
| 1 | Online-change plan present in the paired manifest |
| 2 | OOP descriptors/vtables in use (M5 phase 4b) |
| 3 | Place-descriptor references in use (M5 phase 4b) |
| 4 | Debug checkpoint hooks in use (M6.5). Set only when the module imports service 13 and emits at least one checkpoint; requires host minor 2.3; clear in test and release artifacts (D-177) |
| 5–63 | Reserved; a host that does not implement a set bit rejects the module |

M4.5 and M6 set bit 0 and bit 1. They do not set bit 4; the host does not require it. The
`entry_flags` debug-build bit may be set by the default artifact form, but it conveys no
debug-information or checkpoint capability until M6.5 sets the feature bit and emits the
corresponding data.

M6.5 sets bit 4 only when it emits at least one `debug_check` call; in that case it also sets
`entry_flags` bit 6 and requires host minor 2.3. A descriptor with bit 4 and entry bit 6
disagreeing is inconsistent and is rejected as `STC-E-MODULE-002`. Test builds and release
artifacts clear both bits; test builds require minor 2.2 and release artifacts require 2.1. The
checkpoint table itself is not a descriptor field: it lives in the metadata sidecar's `debug`
object ([Debug and Monitoring](Debug-and-Monitoring.md) §16), which is paired by build ID and is
not part of descriptor or manifest identity.

### 7.2 Descriptor digest

```text
descriptor_digest = SHA-256(
  "stc:descriptor:v1\0" || descriptor_bytes_with_manifest_digest_zeroed)[0..32]
```

The `manifest_digest` field is located by parsing the header; the digest is computed over a copy
where those 32 bytes are zero. The final descriptor embeds the real manifest digest. The host
recomputes the digest the same way and compares it with `header.descriptor_digest` in the
manifest. This binds the descriptor to the manifest without a circular digest.

### 7.3 Entry points and signatures

Every module exports exactly these C symbols with target C ABI:

```c
const uint8_t* stc_module_descriptor(uint64_t* out_size);
const void* const* stc_module_functions(uint32_t* out_count);

int32_t stc_module_attach(const stc_host_api* api,
                          void* const* slots,
                          uint32_t slot_count,
                          uint64_t generation_id);
int32_t stc_module_prepare(uint32_t mode,
                           uint64_t generation_id,
                           const uint8_t* fresh_root_mask,
                           uint32_t fresh_root_mask_size);
int32_t stc_module_cycle(uint64_t generation_id);
int32_t stc_module_shutdown(uint32_t mode);
void stc_module_detach(uint64_t generation_id);
```

- `stc_module_descriptor` returns a pointer to the blob and writes its length. It is callable
  before attach and must not call host services.
- `stc_module_functions` returns an array of opaque code addresses in descriptor order and writes
  the count. It is callable before attach and must not call host services. The host copies the
  addresses it needs; it does not retain module pointers after unload.
- `slots` is an array of `slot_count` pointers to host cells. `slots[i]` is the address of the
  cell for slot `i`; the cell holds one pointer-sized value. Generated code reads a cell value
  with two loads: the pointer from the slot table, then the value from the cell. It never writes
  a cell.
- `generation_id` is a nonzero host-assigned monotonic value. Modules may use it for diagnostics
  and debug records, and must reject a zero value.
- `mode` is `0` cold install, `1` online prepare, `2` explicit reset. M4.5 emits and accepts mode
  `0` and `1`; mode `2` is reserved for the explicit reset policy and is rejected until that
  policy is implemented.
- `fresh_root_mask` is indexed by slot number, not by root ordinal. Bit `slot % 8` of byte
  `slot / 8` is one when a data root must be initialized. Bits for non-data slots are ignored.
  Its size is `ceil(slot_count / 8)` bytes. In cold mode the mask is ignored and every data root
  is initialized. In online mode a preserved root's bit is zero and the module must not write
  that root during prepare.
- `stc_module_attach` returns `0` on success, `1` for an invalid argument or state, `2` for an
  unsupported feature/ABI, and `3` for a slot-count or descriptor mismatch. `stc_module_prepare`
  returns `0` on success, `1` for an invalid mode/state, and `2` when initialization fails.
  `stc_module_cycle` returns `0` on a completed cycle and `1` on an invalid state. A trap never
  returns (§8.4).
- The host passes a generation-specific slot array and never reuses it for a different
  generation. The module must not cache a cell value across `stc_module_detach` or across a
  generation switch; the host only switches at a cycle boundary, so values cached within one
  cycle are valid.

## 8. Host API 2 (2.2 implemented; 2.3 adds the debug service; production modules require 2.1)

### 8.1 Table

```c
typedef struct stc_host_api {
    uint32_t abi_version;   /* (major << 16) | minor; 2.1 is 0x00020001, 2.2 is 0x00020002, 2.3 is 0x00020003 */
    uint32_t struct_size;   /* bytes; modules reject a too-small table */

    void     (*trap)(int32_t code, uint32_t span_id);
    int64_t  (*cycle_now_ns)(void);
    uint64_t (*monotonic_now_ns)(void);
    int64_t  (*local_date_ns)(void);
    void*    (*alloc)(uint64_t size, uint64_t align);
    void     (*free)(void* pointer);
    uint8_t* (*io_image)(uint32_t area);
    uint64_t (*io_size)(uint32_t area);
    int32_t  (*state_load)(uint8_t* buffer, int64_t len, int64_t layout_hash);
    int32_t  (*state_save)(const uint8_t* buffer, int64_t len, int64_t layout_hash);
    void     (*assert)(int32_t condition, int32_t code, uint32_t span_id);
    /* Host ABI 2.2 (M6 test builds only; appended, not present in a 2.1 host): */
    void     (*set_cycle_time_ns)(int64_t ns);
    /* Host ABI 2.3 (M6.5 debug hooks only; appended, not present in a 2.1/2.2 host): */
    void     (*debug_check)(uint32_t checkpoint_id, uint32_t span_id);
} stc_host_api;
```

The table is host-owned and read-only to the module. The module receives it in
`stc_module_attach` and caches it. Fields are appended only within a major version, and
`struct_size` grows accordingly. The host and module use the same target C ABI, so field offsets
are identical; `struct_size` protects against a module built against a newer minor version.

An import slot's host cell holds the service entry point itself (the function address from the
corresponding table field). The host binds import cells when it builds the generation's slot
table; the module loads the cell value and calls it exactly like a function cell. The module may
also use its cached host API pointer for a service that has no import slot, but M4.5-generated
code uses import slots for every service it calls.

### 8.2 Services

Service IDs are used by import slot records and are fixed for host ABI 2 (2.0–2.3):

| ID | Field | Contract |
| --- | --- | --- |
| 1 | `trap` | Reports `(code, span_id)` and transfers control to the host trap boundary; never returns |
| 2 | `cycle_now_ns` | Clock value latched once at cycle entry, shared by all timers in the scan |
| 3 | `monotonic_now_ns` | Live monotonic clock; host-only callers except generated date/time helpers |
| 4 | `local_date_ns` | Local date/time as nanoseconds since the epoch, latched for the scan |
| 5 | `alloc` | Transient zeroed host-owned block of at least `size` bytes aligned to `align`; null on failure |
| 6 | `free` | Releases a block from `alloc`; unknown or double frees are module faults |
| 7 | `io_image` | Base pointer for area `1` inputs, `2` outputs, `3` markers; null if unconfigured |
| 8 | `io_size` | Byte length of the corresponding area |
| 9 | `state_load` | Loads retained bytes into a module buffer; returns `0` or a state status |
| 10 | `state_save` | Saves retained bytes from a module buffer; returns `0` or a state status |
| 11 | `assert` | Traps with `(code, span_id)` when `condition` is zero; otherwise returns |
| 12 | `set_cycle_time_ns` | Host ABI 2.2/M6 test builds only: sets the next cycle's latched time; does not change the current scan |
| 13 | `debug_check` | Host ABI 2.3/M6.5 debug hooks only: called at a checkpoint with `(checkpoint_id, span_id)`; returns normally, may block while the session is stopped, and is a no-op with no session (D-177) |

Service 13 is the only debug service in v1 and is appended after service 12. Its signature is
`void (*debug_check)(uint32_t checkpoint_id, uint32_t span_id)`. The host validates the
checkpoint ID/span against the metadata sidecar's debug object; a mismatch reports
`STC-E-DEBUG-016` and disables breakpoints but does not affect normal execution. A module that
does not import service 13 requires no debug support and keeps its prior minor (2.1 production,
2.2 test).

`alloc` blocks are zeroed, stable for the lifetime of the requesting module generation, and freed
on failed prepare, retire, or unload. The host rejects an attempt to register a transient block
as a persistent root. `free` accepts only pointers returned by `alloc` in the same generation.

`io_image` areas are process-image areas, not direct-variable cells. Direct cells are separately
resolved by the host per generation. Retained-state status values follow
[State and RETAIN](State-and-Retain.md); the M6 host implements the file-backed services and
returns the documented status values.

The `assert` entry is used by the M6 test runner and M5 hosted checks; production codegen does
not emit assertions.

### 8.3 Compatibility

`abi_version` is `(major << 16) | minor`, with major 2 and minor 1 for the M5 compiler. A
module's descriptor carries the required major/minor. The host accepts a module when:

1. majors are equal, and
2. host minor is greater than or equal to the module's required minor, and
3. host `struct_size` covers every service the module uses.

Minor 1 appends no service and changes no signature or struct prefix: it records the inline
TYPEINFO, VTABLE, and object-record layout fixed in §8.5, which generated code reads without a
host call. The table shape, `struct_size` requirement, and service IDs are identical to 2.0, so
the minor gate is what keeps a 2.0-only host from running OOP code built against the layout. A
module built for 2.0 runs on a 2.1 host. A 2.1 module is rejected by a 2.0 host.

Minor 2 (M6 phase 6) appends service ID 12, `set_cycle_time_ns(int64_t)`, after `assert`; the
struct prefix and every existing signature and service ID are unchanged. Only test builds import
it, so their descriptor requires minor 2 and a 2.1 host rejects them before attach. Production
modules do not import service 12 and keep required minor 2.1, so they continue to run on a 2.1
host. A breaking change to a service signature or the struct prefix increments the major version.

Minor 3 (M6.5) appends service ID 13, `debug_check(uint32_t checkpoint_id, uint32_t span_id)`,
after service 12; the prefix, `struct_size` gate, and every existing signature and service ID are
unchanged. Only debug builds that emit at least one checkpoint import it, so their descriptor
requires minor 3 and a 2.1/2.2 host rejects them before attach. Test builds remain at minor 2;
hook-free debug builds and release builds remain at 2.1 because they emit no hooks. M4 ABI 1
artifacts are not loadable. No other debug service is added in v1; any later debug service
appends after ID 13 under the same major.
The checkpoint table and session protocol are not part of this table: see
[Debug and Monitoring](Debug-and-Monitoring.md) §13–§16.

Module entry-point names and their meanings are part of module format 1, not of the host API
minor version. Adding a new entry point requires a module format change; adding a host service
does not.

### 8.4 Trap boundary

`trap` is non-returning by contract. The host calls `stc_module_prepare` and
`stc_module_cycle` from inside a platform trap boundary. A trap records the code, span, and
generation, performs a non-local exit to that boundary, and marks the generation failed. The
module never unwinds through its own frames, and generated code never continues after a trap.

The runtime implementation confines the non-local exit to its C ABI boundary. It may use a small
C shim for the platform's `setjmp`/`longjmp` equivalent, or an equivalent mechanism with the same
observable contract. Modules must not install signal handlers, use C++ exceptions across the
module boundary, or call `abort` for recoverable conditions. A trap during a rollback-window
cycle causes the host to restore the previous generation; a trap outside a rollback window ends
the run with exit code 70 in executable mode and a recorded status in embedding mode.

Trap codes are the existing `STC_TRAP_*` values 1–10 from
[Runtime and Execution](Runtime-and-Execution.md).

### 8.5 OOP runtime records

Host ABI 2.1 fixes the byte layouts that generated code reads directly for class, function-block,
and interface values (D-147–D-151). The records are host-owned: a module allocates none of them,
stores only through the storage pointers they contain, and never writes a record header, a
TYPEINFO field, or a VTABLE entry. They are versioned separately from the descriptor format and
from the catalog descriptor records.

A catalog `DescriptorRecord`/`DescriptorDecl` for a TYPEINFO or VTABLE keeps the **owning type's**
canonical type key, layout hash, size, and alignment, so artifact validation, engine
classification, preservation, and migration compare descriptor records exactly as type records.
The record layout below is a different thing: its byte size includes the record header and its
variable-length arrays and is not the owning type's size. The compiler emits one TYPEINFO
descriptor per class or FB type and one VTABLE descriptor when the type has class method slots;
interface types have no descriptor of their own. `DescriptorKind::Object` stays defined but is
unprojected in M5 because object records are host storage reached through hidden roots (§8.5.6),
not catalog identities.

All offsets are bytes; `ptr` is pointer-sized (8 bytes for format 1), `id128` is the 16-byte
`GotId` digest order used by every artifact, and every field is naturally aligned. Generated code
computes a variable field's address with pointer-sized GEP steps from the fixed header and never
embeds a target size or a record byte length.

#### 8.5.1 Object record

```c
struct stc_object_record {
    const stc_typeinfo* typeinfo;   /* offset 0 */
    void*               leaf[N];    /* offsets 8, 16, ...; resolved storage addresses */
};
```

`N` is the owning type's leaf count in the same base-first ordinal order as the `[N x ptr]` body
root table. The record occupies `8 * (N + 1)` bytes with alignment 8 and stores no length or
format word: its shape is given by the TYPEINFO's `record_format` and `leaf_count`, and the
record root's size (§8.5.6) must equal `8 * (N + 1)`.
Entry `i` is the resolved storage address of leaf `i` — a host root address for a persistent
object or an activation address for a local object — never a GOT cell address. `typeinfo` is
non-null in a valid record.

The record pointer is the object handle: it is `THIS`, a class value, a method/FB body's first
argument, and the `handle` field of an `Object` place descriptor. A null handle or record traps
`STC-TRAP_NULL_REF` when it is dereferenced or dispatched. Interface values are nullable
(§8.5.4); FB values are never null; a valid record is fully initialized once prepare completes.

#### 8.5.2 TYPEINFO

```text
offset  field               type
0       record_format       u32   = 1
4       kind                u32   1 = class, 2 = function block
8       flags               u32   bit 0 = abstract (informational); bits 1..31 reserved, zero
12      leaf_count          u32   N; object record leaf entries
16      base_count          u32   base-chain length
20      interface_count     u32   interface-map length
24      vtable_slot_count   u32   class VTABLE entries; 0 = no VTABLE
28      reserved            u32   = 0
32      type_id             id128 owning type GotId
48      layout_hash         id128 owning type's target layout hash
64      class_vtable        ptr   class VTABLE record, or null
72      base_chain          id128[base_count]
                                nearest base first; the type itself is excluded
...     interfaces          interface[interface_count]
                                { id128 interface_id; ptr vtable; }
```

A base-chain entry is compared by identity, never by name. The interface map is **base-first**:
entries inherited from the base type keep their indices, and interfaces newly satisfied by this
type (including interfaces reached through an interface's `EXTENDS` closure) are appended sorted
by `GotId` ascending. Each entry's `vtable` points at the per-(object type, interface) VTABLE
built for **this** type, so an inherited index remains valid for a derived runtime object while
overrides still take effect. The map contains every interface the object satisfies, including
transitively extended ones.

#### 8.5.3 VTABLE

```text
offset  field           type
0       record_format   u32   = 1
4       kind            u32   1 = class method table, 2 = interface method table
8       entry_count     u32
12      reserved        u32   = 0
16      owner_id        id128 implementing class/FB type GotId
32      interface_id    id128 interface type GotId; zero for a class table
48      entries          ptr[entry_count]
```

Every entry is a pointer to a **host function cell**, not a code address. Dispatch loads the
entry to get a cell address and then loads the cell to get the code address, so an online change
patches cells at activation without rewriting any vtable. A class table has one entry per class
method slot (`entry_count == typeinfo.vtable_slot_count`); each method function record carries
its sema-assigned `vtable_slot`, an override already carries the base slot, and the host places
the record at that slot without recomputing the order.

An interface table has one entry per prototype in the interface's `EXTENDS` closure, ordered
base-first: each interface in the closure contributes its own prototypes sorted by (name,
signature), with a duplicate (name, signature) kept only at its earliest occurrence. A
prototype's slot is therefore its index in its declaring interface's table, and a derived
interface's table is a prefix extension of its base interface's table, so one `vtable_slot` per
prototype record is sufficient. The host places each entry at the prototype record's
sema-assigned `vtable_slot` and supplies the most-derived implementation found by matching the
prototype's method name (the final key segment) and signature against the object type's base
chain.

#### 8.5.4 Interface values

An interface value is a 16-byte `{ ptr record; ptr vtable; }` pair: the object record pointer and
the per-(object type, interface) VTABLE pointer. `NULL` is both fields zero. The pair is passed,
stored, and compared as one value; dispatch indexes the pair's vtable directly and does not
consult TYPEINFO. A valid non-null interface has both fields non-null. A null-interface test reads
the record pointer.

#### 8.5.5 Class and FB storage projection

A class value is a pointer-sized **handle**; an FB value is an inline instance. The class
**instance** layout (the field struct) is not the class **value** layout. Consequences:

- A class-typed field, array element, local, parameter, or result occupies one pointer in its
  containing layout. `FieldRecord`/root type keys project it as `PTR_TO(T:<class GotId>)`, or
  `ARRAY[...]OF(PTR_TO(T:<class GotId>))` for an array, while the class `TypeRecord` keeps the
  instance size, alignment, and layout hash.
- The owned instance of a class-typed persistent declaration is separate storage: its fields are
  ordinary roots under the declaration path (for example `PKG::T.C.F#I`), and it has its own
  record root. A class-typed local owns a stack instance and a stack handle slot.
- Arrays of classes are arrays of handles; each element gets its own owned instance, record
  root, and field roots. A persistent array of handles is one `HANDLE` root with the array type
  key; its record and owned-instance roots are per element.
- FB-typed declarations stay inline for their value layout and still get a record root for the
  body ABI.
- Class handle roots and object record roots are non-retained. The owned instance's field roots
  keep their declared retention policy, so a cold start restores the instance's retained fields
  while prepare re-points the fresh handle at its owned instance.
- `:=` between class values copies the handle (sema's reference assignment); `:=` between FB
  values copies fields. Assignment through a dereferenced `Object` reference copies state into
  the referenced object and never rebinds the original handle.

#### 8.5.6 Hidden root keys

Hidden roots are data roots with the reserved detail markers `HANDLE` and `OBJREC`. They use the
existing `<PACKAGE>::<PATH>[#<INSTANCE>]` forms with the marker appended as the final segment of
the detail suffix (or as the whole suffix for a global), and are the only data roots allowed to
carry a pointer-typed type key.

| Hidden root | Key | Type key | Size / align |
| --- | --- | --- | --- |
| Object record | `PKG::PATH#OBJREC` or `PKG::PATH#INSTANCE/OBJREC` | `ARRAY[0..N]OF(PTR_TO(T:<owner GotId>))` | `8 * (N + 1)` / 8 |
| Class handle | `PKG::PATH#HANDLE` or `PKG::PATH#INSTANCE/HANDLE` | `PTR_TO(T:<class GotId>)`, or `ARRAY[l..u]OF(PTR_TO(T:<class GotId>))` for a handle array | 8, or the array size / 8 |

`PATH` is the declaration path, including the synthetic `.E<index>` element segment for array
elements; `INSTANCE` is the declaration's instance path (`CONFIG/RESOURCE/INSTANCE[/NESTED...]`)
and is absent for a global. Both roots use root kind `nested_field`, retention `none`, initializer
`none`, and the declaration's own `owner_type`/`field_path` convention (`owner_type` is the type
owning the first field segment, absent for a global or program field; `field_path` is the
declaration's segment path, empty for a global). The identity tuple is `DataRoot` with the
declaration `path` and the full detail suffix as `qualifier` (`HANDLE`, `INSTANCE/HANDLE`,
`OBJREC`, or `INSTANCE/OBJREC`), so the handle and record roots of one declaration derive
distinct `GotId`s and `require_identity` sees the same path/qualifier split as the key. The
object record's `[0..N]` array has `N + 1` pointer elements: element 0 is the TYPEINFO pointer
and elements `1..N` are the leaf storage pointers, so the type key describes the record's exact
byte layout and its bound carries the leaf count into the layout hash.

The `HANDLE` and `OBJREC` markers never occur in a source identifier position: they follow the
`#` marker, so a user field named `HANDLE` or `OBJREC` keeps its ordinary path key. A generated
key never uses `HANDLE` or `OBJREC` as a nested instance name, and a source declaration whose
derived key would collide with a hidden root is rejected rather than merged. The marker is
recognized only as the whole detail suffix or as the final segment after a complete
(`CONFIG/RESOURCE/INSTANCE`) path, so a three-segment ordinary instance that happens to be named
`HANDLE` or `OBJREC` keeps its ordinary root key.

The host allocates hidden roots like any data root and zeroes fresh storage. Generated
`stc_module_prepare` then fills every fresh object record (TYPEINFO pointer plus leaf storage
addresses) and every fresh class handle (the owned instance's record address); preserved records
and handles are not rewritten. Hidden roots are excluded from retained-state leaves and metadata
payloads. `stc-got`'s catalog and engine validation accept the two markers, reject a marker on
any other root kind, and require the pointer type key only on marked roots; no manifest or
descriptor format number changes.

#### 8.5.7 Interface-prototype function records and trap stubs

Every interface method prototype is projected as an ordinary function record: identity kind
`function`, key `PKG::INTERFACE.METHOD@SIGNATURE`, `FunctionKind::Method`, `owner_type` = the
interface type, the sema-assigned `vtable_slot`, and a `Function` slot like any callable. The
module emits a **trap-stub body** for it, which traps `STC-TRAP_UNIMPLEMENTED` with the
prototype's span. The host binds the prototype's function cell to that stub exactly like other
function cells. Dispatched interface calls never use the prototype's own cell: they load the
implementation from the interface value's per-pair vtable, so reaching the stub means dispatch
was bypassed or an implementation is missing. The per-pair table is built from the prototype
records in the interface's `EXTENDS` closure (`owner_type` is the declaring interface and
`vtable_slot` is the prototype's slot in that declaration) and the object type's matching
implementation, so no manifest field and no format number change.

#### 8.5.8 Inline access rules

Generated code may read, but never write, a record. The permitted reads are:

1. `record->typeinfo` and `record->leaf[i]`; field storage is reached through `leaf[i]`.
2. `typeinfo->record_format`, `kind`, `leaf_count`, `base_count`, `interface_count`,
   `vtable_slot_count`, `type_id`, `layout_hash`, `class_vtable`,
   `base_chain[0..base_count]`, and `interfaces[0..interface_count]` (`interface_id` and
   `vtable`).
3. `vtable->entries[slot]` to obtain a host function cell, then the cell value to obtain code.
4. An interface value's two pointers.

Every null handle, record, TYPEINFO, VTABLE, or interface field that a read depends on is checked
before use and traps `STC-TRAP_NULL_REF`. `?=` and interface upcasts read only TYPEINFO and the
map and never call the host; a failed `?=` yields the all-zero value and never traps. Local
records are per-activation allocas filled with the owning type's TYPEINFO cell value and
activation leaf addresses; VTABLE and TYPEINFO records and descriptor cells are never allocated
or modified by generated code. A hidden record whose TYPEINFO `leaf_count` does not match its
root size is invalid, and a live record or interface value makes a descriptor shape change
reject the swap (D-151).

## 9. GOT manifest format 2

### 9.1 Files and outer header

The binary manifest is `<name>.stc-got.bin`. Its outer header is:

```text
magic        [8]u8 = "STCGOT\0\0"
format       u16   = 2
flags        u16   = 0
body_length  u64
digest       digest32
body         [body_length]u8
```

`flags` is zero in format 2; nonzero values are invalid. `body` is the concatenation of the
sections in §9.3. `digest` is defined in §9.4. Bytes after `body_length` are invalid. A manifest
whose outer `format` is not 2 is rejected with a version error; format-1 manifests are never
upgraded, reinterpreted, or partially read. The descriptor's `manifest_format` must be 2, while
the descriptor, module, state, and host ABI formats remain 1 and 2 respectively (D-180).

The JSON manifest is `<name>.stc-got.json`:

```json
{
  "body": {
    "aggregates": [],
    "descriptors": [],
    "exports": [],
    "functions": [],
    "header": {},
    "history": {},
    "instances": [],
    "plan": {},
    "roots": [],
    "slots": [],
    "tasks": [],
    "type_metadata": [],
    "types": []
  },
  "digest": "0x…",
  "flags": 0,
  "format": 2
}
```

Canonical JSON rules from §2.2 apply. The `body` object is the JSON projection of the binary
body, using the same field names and array order. `digest` is the same value as the binary
header's digest; a JSON-only verifier encodes the body per §9.3 to check it. A tool that converts
JSON to the binary projection must reproduce the exact body bytes; a mismatch is an invalid
manifest.

### 9.2 Records

The following notation uses the primitive types of §2.1. Every field is present in both
projections; `option<T>` is `null` when absent in JSON.

`SourceLocation`:

```text
package  string
file     string      package-relative path, '/' separators, no drive or absolute prefix
start    u32         byte offset
end      u32         byte offset
```

`Header`:

```text
profile                  string
compiler_version         string
target_triple            string
module_name              string
active_configuration     string
endianness               u8
pointer_width            u8
module_format            u16
host_abi_major           u16
host_abi_minor           u16
generator_flags          u64
required_features        u64
build_id                 id128
parent_build_id          option<id128>
parent_manifest_digest   option<digest32>
descriptor_digest        digest32
source_fingerprint       digest32
catalog_digest           digest32
plan_digest              digest32
```

`generator_flags` bits:

| Bit | Meaning |
| --- | --- |
| 0 | Debug build (default) |
| 1 | Release build (`--release`) |
| 2 | Relaxed syntax accepted |
| 3 | Standard FB library disabled |
| 4 | Multi-task testing override enabled |
| 5 | Untracked removal explicitly allowed |
| 6 | Cold-layout build |
| 7–63 | Reserved, zero |

`RootRecord`:

```text
got_id              id128
key                 string
previous_keys       list<string>
root_kind           enum { Global=1, ProgramField=2, NestedField=3, Direct=4, TaskGuard=5 }
owner_type          option<id128>
field_path          string
type_key            string
layout_hash         id128
size                u64
align               u32
retention           enum { None=0, Retain=1 }
initializer         enum { None=0, Zero=1, Constant=2, Expression=3 }
initializer_digest  option<digest32>
source              option<SourceLocation>
slot                u32
```

`Direct` roots use a direct cell and are never preserved. `TaskGuard` roots are host-owned
one-byte or one-word completion flags keyed by task identity. A hidden `HANDLE`/`OBJREC` root
uses `root_kind = NestedField`, `retention = None`, `initializer = None`, and a pointer or
record type key (§8.5.6); its `owner_type`/`field_path` follow the declaration's ordinary root
convention. `size` is the root's storage size in bytes; `align` is a power of two. `field_path`
is relative to the owner type and is empty for globals and direct variables that are not fields.
A retained root has `retention = Retain`; other roots use `None`. `initializer_digest` hashes the
canonical initializer description (`"stc:init:v1\0" || type_key || 0x00 || canonical value bytes`
for constants, or the deterministic init-expression fingerprint for expressions); the value
itself is not stored in the manifest.

`FunctionRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
signature_key   string
function_kind   enum { Function=1, ProgramBody=2, FbBody=3, Method=4,
                       ExportWrapper=5, StdFunction=6 }
owner_type      option<id128>
flags           u32
slot            u32
export_name     option<string>
vtable_slot     option<u32>
source          option<SourceLocation>
```

Function flags reuse the descriptor bits in §7.1. `vtable_slot` is the sema-assigned class or
interface method slot for `Method` records (including interface prototypes) and is absent for
other kinds; the host places vtable entries by it and never recomputes the order. `export_name`
is the C symbol for wrapper and no-mangle records.

`FieldRecord`:

```text
got_id          id128
path            string
type_key        string
layout_hash     id128
offset          u64
size            u64
align           u32
retention       enum { None=0, Retain=1 }
source          option<SourceLocation>
```

`TypeRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
type_key        string
layout_hash     id128
size            u64
align           u32
type_kind       enum { Struct=1, FunctionBlock=2, Class=3, Interface=4,
                       Enum=5, Subrange=6, Alias=7 }
base            option<id128>
interfaces      list<id128>
vtable_shape    option<id128>
fields          list<FieldRecord>
source          option<SourceLocation>
```

Field records are sorted by `(offset, path)`. Type records are sorted by `(key, got_id)`.
M4.5 emits no type records for the elementary scalar surface.

`EnumMemberRecord`:

```text
name   string      normalized member identifier
value  i128        little-endian 16-byte integer value
```

`SubrangeBounds`:

```text
lower  i128        inclusive lower bound
upper  i128        inclusive upper bound
```

`TypeMetadataRecord`:

```text
type_id       id128
enum_members  list<EnumMemberRecord>   sorted by (value, name)
subrange      option<SubrangeBounds>
```

`EnumMemberRecord` is encoded as `string(name) || i128(value)`; the list header is a `u32`
count. `TypeMetadataRecord` is encoded as `id128(type_id) || u32(enum_members.count) ||
members... || option_tag || [SubrangeBounds]`. `subrange` is present only for a subrange type and
`enum_members` is empty for it; an enum has a member set and no bounds. The section contains
exactly one record for every `Enum` or `Subrange` type record and no record for another kind.
Records are sorted by `type_id` ascending bytes. Enum members and subrange bounds are canonical
input matter: they normalize identifier spelling and preserve the declared integer values.

`AggregateFieldRecord`:

```text
path         string   top-level program field path, normalized
type_key     string   declared canonical type key
layout_hash  id128    declared type's target layout hash
```

`AggregateRecord`:

```text
package      string
path         string
qualifier    option<string>
kind         enum { Type=1, Program=2 }
shape_key    string
layout_hash  id128
fields       list<AggregateFieldRecord>   sorted by path
```

`AggregateRecord` is encoded as `string(package) || string(path) || option_tag ||
[string(qualifier)] || u8(kind) || string(shape_key) || id128(layout_hash) ||
u32(fields.count) || [string(path) || string(type_key) || id128(layout_hash)]...`.
A `Type` aggregate names a declared named type in `shape_key`, has no qualifier, and has no
fields; its split roots are the roots under `package::path`. A `Program` aggregate names the
program path in `shape_key`, carries the instance qualifier, and lists its top-level fields in
canonical path order; each field must have at least one root under
`package::path.field#qualifier`. The aggregate layout hash is the declared target layout hash
and is nonzero. Aggregates are sorted by `(package, path, qualifier-presence, qualifier)`.

`DescriptorRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
descriptor_kind enum { TypeInfo=1, Vtable=2, Object=3 }
owner_type      option<id128>
type_key        string
layout_hash     id128
size            u64
align           u32
slot            u32
source          option<SourceLocation>
```

Descriptor records are sorted by `(key, got_id)`. A descriptor cell is host-owned; the record
describes its shape, and the host creates the runtime object during prepare. Vtable and object
descriptor shape changes are rejected while a live instance or interface depends on them. M4.5
emits no descriptors; M5 phase 4b.1 emits one TYPEINFO per class/FB type and one VTABLE when the
type has class method slots (interface types get no descriptor).

`InstanceRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
program_type    option<id128>
program_type_key string
configuration   string
resource        string
instance_path   string
task_key        option<string>
source          option<SourceLocation>
```

`TaskRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
configuration   string
resource        string
name            string
single          bool
interval_ns     option<i64>
priority        option<i64>
guard_got_id    option<id128>
source          option<SourceLocation>
```

`ExportRecord`:

```text
got_id          id128
key             string
previous_keys   list<string>
c_name          string
function_got_id id128
signature_key   string
no_mangle       bool
source          option<SourceLocation>
```

`SlotRecord` has the same fields and ordering as §7.1 and must equal the descriptor's slot list
exactly. `Direct` slot IDs refer to `RootRecord` entries with `root_kind = Direct`.

`Plan`:

```text
verdict                  enum { Cold=1, OnlineCandidate=2, Rejected=3 }
parent_build_id          option<id128>
parent_manifest_digest   option<digest32>
actions                  list<ActionRecord>
diagnostics              list<PlanDiagnostic>
```

`ActionRecord`:

```text
got_id       id128
kind         u8
key          string
old_key      option<string>
action       enum { Preserve=1, Allocate=2, Migrate=3, Reset=4, Drop=5, Reject=6 }
reason_code  string
detail       string
```

`PlanDiagnostic`:

```text
code         string
severity     enum { Warning=1, Error=2 }
message      string
got_id       option<id128>
old_key      option<string>
new_key      option<string>
```

`History`:

```text
retired      list<RetiredRecord>
```

`RetiredRecord`:

```text
got_id                  id128
kind                    u8
key                     string
previous_keys           list<string>
retired_build_id        id128
retired_manifest_digest digest32
cause                   enum { Dropped=1, Reset=2, Replaced=3 }
```

`retired_build_id` and `retired_manifest_digest` identify the manifest in which the identity was
active. History is build history, not runtime quarantine state. The host maintains quarantine
independently.

### 9.3 Body sections

| Tag | Section | Payload | Required |
| --- | --- | --- | --- |
| 1 | Header | `Header` | Yes |
| 2 | Roots | `list<RootRecord>` | Yes |
| 3 | Functions | `list<FunctionRecord>` | Yes |
| 4 | Types | `list<TypeRecord>` | Yes |
| 5 | Descriptors | `list<DescriptorRecord>` | Yes |
| 6 | Instances | `list<InstanceRecord>` | Yes |
| 7 | Tasks | `list<TaskRecord>` | Yes |
| 8 | Exports | `list<ExportRecord>` | Yes |
| 9 | Slots | `list<SlotRecord>` | Yes |
| 10 | Plan | `Plan` | Yes when feature bit 1 is set |
| 11 | History | `History` | Yes |
| 12 | TypeMetadata | `list<TypeMetadataRecord>` | Yes |
| 13 | Aggregates | `list<AggregateRecord>` | Yes |

Sections appear in ascending tag order. An artifact missing a required section is invalid.
Empty tables are present with count zero. Table ordering:

- roots: `(key, got_id)` ascending bytes;
- functions: `(key, signature_key, got_id)`;
- types: `(key, got_id)`;
- descriptors: `(key, got_id)`;
- instances: `(key, got_id)`;
- tasks: `(key, got_id)`;
- exports: `(c_name, got_id)`;
- slots: canonical slot order from §6.2;
- plan actions: `got_id` ascending bytes;
- plan diagnostics: `(code, got_id-present, got_id, old_key, new_key, message)` ascending;
- history: `(retired_build_id, got_id)` ascending;
- type metadata: `type_id` ascending bytes;
- aggregates: `(package, path, qualifier-presence, qualifier)` ascending.

`previous_keys` are ordered oldest first, contain no duplicates, and never contain the current
key. They are the tracked rename/move history of one identity.

A non-canonical order is invalid. This lets validation and double-build tests compare bytes
directly and makes reordering tampering detectable.

### 9.4 Manifest digest

```text
manifest_digest = SHA-256("stc:manifest:v2\0" || body)[0..32]
```

`body` is the exact concatenation of the sections (tag, length, payload) in §9.3. The digest
does not include the outer header's magic, format, flags, body length, or digest field. The
`parent_manifest_digest` recorded by a child build is the parent's `manifest_digest`.

### 9.5 Validation

A manifest is accepted only when all of the following hold:

1. The outer magic matches, `format` is exactly 2, `flags` is zero, `body_length` exactly covers
   the body, and the digest matches.
2. The required sections are present exactly once, in ascending tag order, with lengths that do
   not exceed the body.
3. The header profile, target triple, endianness, pointer width, module format, and host ABI
   match the host or the build target.
4. `build_id` is nonzero; `parent_build_id` and `parent_manifest_digest` are both present or both
   absent; a present parent pair is nonzero.
5. `descriptor_digest`, `source_fingerprint`, `catalog_digest`, and `plan_digest` are nonzero.
6. Tables are canonically ordered; `GotId`s are unique within each table; keys are unique per
   `(kind, key)`. For root records, `Direct` is identity kind 9 and every other root kind is
   identity kind 1; the key already carries the instance path. Slot numbers are contiguous,
   grouped, sorted, and reference existing records.
7. Every key matches the grammar in §3.2, every type/signature key matches §4, and every string
   is valid UTF-8. Stable keys and C export names are printable ASCII.
8. Sizes and alignments are consistent: `align` is a power of two, `size > 0` for data roots,
   `offset + size` fits the owning type where a field record says so, and counts match the
   encoded list lengths.
9. `catalog_digest` and `plan_digest` recompute exactly from the corresponding sections.
10. Plan actions cover every active identity and every dropped persistent baseline identity;
    direct-variable identities and removed exports follow §10. `Preserve` and `Reset` actions
    reference a parent record with the same `GotId`; `Allocate` actions do not. A `Rejected`
    verdict is terminal and is never applied.
11. History entries are a subset of retired/dropped identities, carry nonzero build/manifest
    identities, and are canonically ordered.
12. Type metadata and aggregate records satisfy §9.2: metadata exists exactly for every
    `Enum`/`Subrange` type, enum members are normalized, unique, and sorted by `(value, name)`,
    subrange bounds satisfy `lower <= upper`, and only the matching member set or bounds is
    present. Aggregate package/path/qualifier spelling is normalized, layout hashes are nonzero,
    type shapes have no qualifier or fields and have split roots, program shapes match their
    instance's program type and have at least one root for every listed top-level field, and
    both tables are canonically ordered.
13. The artifact paired with a module satisfies §11.4, including `manifest_format = 2`.

Manifest data is untrusted. Validation never executes or interprets manifest text, never follows
a path from it, and never allocates from an unchecked count. A malformed, stale, tampered,
missing, or unpaired manifest selects the cold-download path; it is never partially applied.

## 10. Online-change plan semantics

The compiler emits the plan from the catalog comparison:

| Action | When emitted | Runtime effect |
| --- | --- | --- |
| `Preserve` | Baseline `GotId` exists and type key, layout hash, size, and align match | Reuse storage; never reinitialize; for non-root identities, retain identity continuity |
| `Allocate` | No baseline identity, or a direct-variable identity that is per-generation | Allocate host storage and initialize once |
| `Migrate` | Reserved for post-v1 layout migration; M4.5 never emits it | Reject in M4.5 |
| `Reset` | Reserved for an explicit per-field reset policy; M4.5 never emits it | Reject in M4.5 |
| `Drop` | Baseline data root has no match and untracked removal is allowed; non-root identity disappears | Quarantine root storage; warn; non-root retirement has no storage effect |
| `Reject` | Layout/type/signature/topology change outside policy | Reject the candidate atomically |

Direct-variable identities are per-generation cells, not persistent roots. A baseline direct
identity that is absent from the new build produces no `Drop` action, never requires
`--allow-untracked-removal`, and never quarantines storage. A direct identity that matches by key
but changes area/address is an `Allocate` with `STC-W-ONLINE-003`; the host reconfigures the
process-image cell. Export identities are never dropped: a missing export is a `Reject`.

Verdicts:

- `Cold`: no baseline; every identity is `Allocate`; parent fields are absent; the runtime must
  cold-download.
- `OnlineCandidate`: a valid exact parent; actions describe the change; the runtime applies the
  plan only after its own currentness check.
- `Rejected`: the change is unsafe or outside policy. The compiler reports an error and does not
  emit a deployable module by default.

The compiler's plan is a candidate, not an authorization. The runtime is the final authority:
it applies an online plan only when the manifest's parent build ID and parent manifest digest
equal the active generation's, the module/manifest pair validates, and every `Preserve` root
matches the active layout. Otherwise the only permitted operation is a cold download. A `Drop`
action requires `--allow-untracked-removal`; without it the compiler emits
`STC-E-MANIFEST-003` and fails the build.

An export whose C name exists in the baseline with a different signature, or whose C name
disappears from the new build, is a rejection (`STC-E-ONLINE-002`), never a silent drop. Export
removal is not an untracked-removal case and `--allow-untracked-removal` does not permit it. A
task/resource topology change, profile, target, or ABI change is a cold download, not an online
plan. A tracked rename/move of a type carries its field identities and the matching instance
roots; an untracked type rename is an untracked removal plus allocation for every affected root.

## 11. Build IDs and artifact pairing

### 11.1 Source fingerprint

The source fingerprint covers every production source file that contributes to the build,
including dependency packages, in a canonical order:

```text
source_fingerprint = SHA-256(
  "stc:source:v1\0" || file_count:u32 ||
  for each file sorted by path bytes:
    string(path) || SHA-256(file_bytes))[0..32]
```

`path` is `<normalized-package-name>/<package-relative-path>` with `/` separators and no `..`,
drive letter, or absolute prefix. Test-only sources excluded from a production build are not
included; a `stc test` build includes them and therefore has a different fingerprint. For the
manifest-free M4 bridge, paths are relative to the longest common parent directory of the input
list, or the file name plus CLI ordinal when no common parent exists. Source comments and
formatting are part of the file bytes and therefore part of the fingerprint.

### 11.2 Build ID

```text
build_id = SHA-256(
  "stc:build:v1\0" ||
  string(profile) ||
  string(target_triple) ||
  string(compiler_version) ||
  string(active_configuration) ||
  endianness:u8 || pointer_width:u8 ||
  module_format:u16 || host_abi_major:u16 || host_abi_minor:u16 ||
  generator_flags:u64 ||
  source_fingerprint ||
  parent_tag:u8 [ || parent_build_id:id128 || parent_manifest_digest:digest32 ] ||
  catalog_digest ||
  plan_digest)[0..16]
```

`parent_tag` is `0` for a cold build (no following fields) and `1` for an online build (two
following fields). The build ID is nonzero. It depends only on build inputs, never on emitted
bytes, the output directory, the clock, a host name, or the environment. Two builds with the
same source, configuration, target, compiler, flags, and exact baseline have the same build ID.

`target_triple` is the resolved target, not an empty string; the driver resolves the host triple
before computing the build ID.

### 11.3 Artifact identities

| Value | Definition |
| --- | --- |
| `descriptor_digest` | §7.2 |
| `manifest_digest` | §9.4 |
| `source_fingerprint` | §11.1 |
| `catalog_digest` | §6.3 (roots, functions, types, descriptors, instances, tasks, exports, slots, type metadata, and aggregates) |
| `plan_digest` | First 32 bytes of SHA-256 over `"stc:plan:v1\0"` and the plan payload |

The module file name is `<name>-<build_id-hex>.dll` or `.so`, where `<build_id-hex>` is the 32
lowercase hexadecimal digits without `0x`. The manifest file names are
`<name>.stc-got.json` and `<name>.stc-got.bin`. The metadata sidecar records `build_id`, the
manifest digest, the descriptor digest, and the parent build ID so tooling can identify the set
without loading the module.

### 11.4 Pairing check

A module and manifest are a valid pair when:

1. The descriptor's `manifest_digest` equals the computed manifest digest.
2. The manifest header's `descriptor_digest` equals the descriptor digest (§7.2).
3. The descriptor and manifest have equal `build_id`, profile, target, endianness, pointer width,
   module format, and host ABI, and the descriptor's `manifest_format` equals the manifest's
   outer format (2 for this version).
4. The descriptor's function set equals the manifest's function set (`GotId`, key, signature,
   and flags), and the descriptor's slot records equal the manifest's slot records exactly
   (`slot`, `kind`, `service`, `got_id`); every `Descriptor` slot resolves to a descriptor
   record in the manifest descriptor table.
5. The descriptor's `required_features` equals the manifest header's `required_features`, and
   the descriptor size, counts, and feature bits validate.

The host performs this check before attach. A mismatch is `STC-E-MANIFEST-001` or
`STC-E-MODULE-001` and selects the cold-download path; the active generation is untouched. A
format-1 manifest or a descriptor that pairs format 1 with the current host likewise fails the
pairing/version check and selects cold download; it is never upgraded in place.

The pairing binds the descriptor, its function/slot tables, and the manifest. It does not hash
the complete platform image, because loaders may rewrite headers and modules are trusted native
code in v1 (see [Architecture](Architecture.md) §8). Build IDs, immutable versioned file names,
and the manifest digest provide deployment identity; code authenticity is not a v1 security
boundary.

### 11.5 Runtime currentness

After pairing, the host compares the manifest's `parent_build_id` and `parent_manifest_digest`
with the active generation's build ID and manifest digest. An exact match permits an online
attempt. A missing parent, a mismatch, a missing/stale/corrupt active manifest, a format-1 or
otherwise undecodable manifest, or a target/ABI mismatch permits only a cold download.
Coincidentally equal `GotId`s never authorize an online change without the exact parent pair.
These rules are formatter agnostic: the persisted active manifest may be binary or JSON, and
both projections must decode to the same format-2 value.

## 12. M4.5 population and conformance

### 12.1 M4.5 contents

For the M4 scalar surface, the compiler populates:

- **Roots:** scalar globals and scalar program-instance fields as `ProgramField` roots; scalar
  `RETAIN` leaves keep their retention policy; direct `AT` variables as `Direct` roots with
  direct slots; one `TaskGuard` root per task.
- **Functions:** user functions, program bodies, and export wrappers, each with a function slot.
  Compiler helpers (checked arithmetic, conversions, init code) stay direct and have no identity.
  An export wrapper does not add a second callable identity: the wrapped callable keeps its
  function `GotId` and slot, and the kind-7 export record references it through
  `function_got_id`. The C-ABI wrapper is module-local code bound by that export record.
- **Types and descriptors:** both tables are empty for elementary scalar programs; named user
  types, vtables, and object/type descriptors appear in M5.
- **Instances and tasks:** the selected or synthesized configuration and the program instances it
  runs. Every task is projected with its interval/priority/`SINGLE` and guard identity (D-154);
  the M4.5 one-task-per-resource restriction is removed. The task record layout is unchanged.
- **Exports:** `{stc:export}` wrappers and their C names.
- **Slots:** data, function, direct, and import slots as assigned in §6.2; descriptor slots are
  empty.
- **Plan:** `Preserve`, `Allocate`, `Drop`, and `Reject`; `Migrate`/`Reset` are not emitted.
- **Host services:** trap, I/O image/size, and retained-state stubs. Clocks, transient
  `alloc`/`free`, and `assert` are in ABI 2.0 but unused by M4.5-generated code and are added as
  import slots when a later construct first needs them.

Direct-root records carry the identity, type, layout, and slot, but not the process-image
area/address; that stays in the `.stc-meta.json` sidecar. Address-remap detection is therefore a
host check against module metadata, not a manifest comparison.

Because `GotId` is content-derived, re-deriving a declaration reintroduced after its identity was
retired produces the same value. V1 has no explicit reuse mapping, so a build that would
resurrect a `GotId` present in the baseline history is rejected with `STC-E-MANIFEST-002` rather
than silently reusing quarantined storage; a key with a new spelling, or a forced cold build that
starts a fresh layout and history, avoids the case.

The manifest records every persistent scalar root separately. It does not record locals,
parameters, temporaries, compiler helpers, or module-local read-only data.

### 12.2 Schema conformance tests

The implementation phases added:

1. Golden descriptor and manifest snapshots for a small scalar program, including both JSON and
   binary projections and their digests.
2. JSON-to-binary conversion that reproduces the exact body bytes and digest.
3. Double builds of the same source and baseline producing byte-identical IR, descriptor,
   manifest, plan, metadata, and build ID.
4. A second build consuming the first manifest with unchanged `GotId`s for every unchanged
   declaration.
5. Tamper cases: altered digest, bad body length, duplicate `GotId`, duplicate key, duplicate
   slot, non-canonical table order, unknown critical feature, target/profile/ABI mismatch,
   missing parent, and descriptor/manifest slot mismatch; each is rejected.
6. A structural test that generated IR contains no direct reference to a persistent root or user
   function; every such access goes through a slot-table cell.
7. A no-persistent-pointer check over root records and generated IR, including direct variables
   and task guards.
8. Trap-boundary tests for prepare and cycle, including rollback-window recovery and executable
   exit code 70 outside a rollback window.

### 12.3 Phase status

Phase 0 is complete: the resolved decisions D-094–D-112 are recorded and the documentation
checklist in `update_plan_got.md` §12 is applied. This document is the phase 1 output, reviewed
and approved before phase 2. Phase 2 is implemented in the `stc-got` crate (stable keys, `GotId`,
catalog, binary/JSON manifest codecs, baseline matching, rename/move maps,
preserve/allocate/drop classification, slot assignment, and deterministic snapshots); the
implementation clarifications for elementary encoding tags, export wrappers, retired-identity
reintroduction, and direct-variable address metadata are recorded under D-118. Phase 2 and D-118
were reviewed and approved on 2026-10-24.

Phase 3 is implemented in `stc-codegen`: the `ir` builder emits GOT slot-table access (a
`getelementptr` of `ptr` by slot number followed by a pointer-sized cell load), indirect calls
through function/import cell values, exported `dso_local dllexport` entry points with the ABI 2.0
signatures, private descriptor blobs, and function-address tables. The `verify` module checks the
structural contract of §7 and §12.2 item 6: bodies reference no persistent root or user callable
directly, cell uses match their kinds, generated code never writes a cell, pre-attach entry
points do not touch the slot table, direct global access is limited to an allowlisted module-local
set, and the function-address table is the only initializer that names a user function. D-119 records the IR-level choices.

Phase 4 is implemented (2026-10-25) in `stc-got::descriptor` and `stc-codegen::module`. The descriptor format-1
codec encodes, decodes, validates, computes the zeroed-digest projection, and checks the
descriptor/manifest pairing of §11.4. The module projection builds the root catalog for the M4
surface (global, program-field, direct, and task-guard roots; user and program-body functions;
instances, tasks, and exports), obtains canonical slot assignments from the identity engine,
exposes declaration-to-slot bindings, and emits the descriptor, binary manifest, canonical JSON
manifest, and online-change plan from one deterministic model. Configuration-less builds
synthesize `_DEFAULT` configuration/resource/task identities; imports are trap, I/O image/size,
and retained-state load/save. Golden descriptor/manifest snapshots, double-build byte equality,
baseline reuse, rename/drop/reject/cold, and tamper rejection are tested. D-120–D-121 record the
choices.

Phase 5 is implemented (2026-10-25) in `stc-runtime`. The host `got` module owns a fixed
per-generation cell vector, validates canonical slot tables, exposes stable cell and slot-table
addresses, builds the §7.3 fresh-root mask, and binds data, function, descriptor, direct, and
import cells by `GotId` or service ID. The `memory` module owns stable zeroed aligned root
storage, exact-match preservation, quarantine, transactional
reset, failed-candidate discard, memory budgets, and per-generation transient allocations with
exact-base free and `free_all`. The `catalog` module validates descriptor/manifest pairs against host profile, target,
endianness, pointer width, ABI range, and implemented features, and parses both manifest
projections. The `migration` module validates a plan against the exact parent build/manifest
identity, preserve-shape compatibility, allocate reuse (including retired identities), drop
restrictions, and retirement coverage. Runtime unit tests cover malformed and tampered artifacts
and allocation lifetimes; loader, swap, and plan execution remain for phase 6 and later. D-122
records the choices. Host API bindings and runtime loading in the later phases followed these
formats exactly, and any further change returns to this document and the decision log before
code lands.

Phase 6 is implemented (2026-10-25) in `stc-runtime`. `loader` is the platform layer over
`LoadLibraryW`/`GetProcAddress`/`FreeLibrary` and `dlopen`/`dlsym`/`dlclose`; it resolves the seven
§7.3 entry points, copies the descriptor blob and function table, and treats load failures as
non-fatal module errors. `host_api` is the static host API table described in §8.1 (2.1
implemented; the 2.2 target appends ID 12): import cells
are bound to the addresses of its fields, transient allocation and free route to the generation
whose entry point is executing, process-image areas map to the configured image, and traps are
recorded before the non-returning trap path. `store` keeps the committed baseline as an immutable
versioned binary/JSON pair plus an atomically replaced `current` pointer; it reports a missing or
corrupt baseline instead of synthesizing one, and builds the §7.4 download envelope from the
manifest projections, deployment identifiers, and a value-free runtime layout. `host` validates
candidates (pairing, function-table count, entry flags), applies `Auto`/`Cold`/`Online` load
modes, gates online changes on the exact committed store and active generation, cold-installs
into a fresh root registry, prepares online changes through a root-transaction journal, switches
at an explicit cycle boundary, commits after one successful cycle, rolls back a failed first cycle
(restoring preserved records, dropped roots, and allocated roots), and delays retired-module
unload behind generation pins. Generation pins and IDs, the active-manifest store, and the
download artifact are covered by runtime tests; a real native no-op module exercises load,
attach, prepare, cycle, commit, rollback, and unload on Windows and Linux. The non-local
trap-boundary rollback and the C-facing launcher/embedding exports landed with the phase 7
execution conversion. D-123 records the phase choices.

Phase 7 is implemented (2026-10-25) across `stc-codegen`, `stc-runtime`, and `stc-driver`.
Generated IR is a complete ABI 2 module: data roots, per-instance program fields, direct `AT`
variables, retained leaves, and task guards are accessed through GOT cells; a data or direct cell
is read with the two loads of §7.3 (table entry to cell address, then cell value); program bodies
receive a per-instance `[N x ptr]` field-storage array; user calls are indirect through function
cells; host services are called through import cells whose values are the service entry points;
and the module exports the seven entry points, descriptor, function table, metadata, retain
helpers, and export wrappers. The host reads `<name>.stc-meta.json` for process-image sizes and
direct-binding `GotId`s, configures the image on cold install, and binds `Direct` cells before
prepare; phase 8 added the online reconfiguration and remap policy for a changed layout or
binding set. The driver emits `module`, `exe` (thin launcher), and `lib`
(embedding host archive) artifacts, and `exe`/`lib` may be combined; `--got-manifest`, `--cold`,
`--rename`, and `--allow-untracked-removal` feed the identity engine. The runtime installs a
non-local trap boundary around every module entry-point call (`__intrinsic_setjmpex` on Windows,
`_setjmp` on POSIX; generated functions are `uwtable`; a small assembly shim owns the second
`setjmp` return because Rust cannot mark the external call `returns_twice`), so a rollback-window
trap restores the previous generation and a trap outside the window exits 70. Existing execution tests, including
C harnesses, traps, the process image, exports, bindings, and retained leaves, run through
compiler-produced module pairs on Windows; a Rust host test covers cold install, online prepare,
activation, commit, and preserved roots. D-124 records the phase choices.

Phase 8 is implemented (2026-10-25) in `stc-runtime` and `stc-driver`. The `image` module
validates `<name>.stc-meta.json` against a host image limit, computes provisional and actual
direct-cell addresses, detects moved bindings, and preserves direct values by `GotId` across an
image reconfiguration. At activation the host snapshots the old image, applies the candidate
layout or remaps bindings in place, rebinds the candidate's `Direct` cells, and keeps the
snapshot plus old bindings in the rollback record; a failed first cycle restores both. Prepare
leaves the active image untouched. A changed image layout or binding set is therefore an online
change rather than a mandatory cold download; unsafe metadata, duplicate or missing bindings,
unaligned multi-byte ranges, and out-of-area bindings are rejected with `STC-E-ONLINE-IO-001`
(or `STC-E-ONLINE-006` for a missing direct-root binding). `Host::warnings` reports remapped
direct variables as `STC-W-ONLINE-IO-001` and dropped data roots as `STC-W-ONLINE-001`.
`Host::reset_quarantine` releases quarantined storage under the explicit reset policy;
`PlanAction::Reset` and `Migrate` remain reserved and rejected in M4.5. Cold installation
reinitializes roots and the process image, and online prepare initializes only fresh roots, so
preserved state, initializer changes, and the disabled retained-state file follow State and
RETAIN §4.1.1. Driver tests drive compiler-produced modules through state-preserving code-only
changes, new-root initialization, tracked rename, untracked removal, type-change rejection with a
cold reset, and direct-variable remap through generated code; runtime tests cover image
reconfiguration, same-layout remap, rollback, cold image reset, quarantine reset, and the warning
surface. D-125 records the phase choices.

Phase 9 is implemented (2026-10-25) in `fuzz/`, `.github/workflows/ci.yml`, and the driver and
runtime test suites. `fuzz_got_manifest` decodes binary and canonical-JSON manifests and checks
the §9 digest chain and projections; `fuzz_module_descriptor` decodes §7 descriptors and parses
the §12.1 metadata sidecar. Neither target loads native code, and CI caps inputs at one MiB.
End-to-end double builds are byte-identical for the module, IR, metadata, manifest, plan, and
build ID; Windows module and launcher links use `/Brepro` so PE timestamps are content-derived.
The build rejects missing, truncated, and malformed baselines with `STC-E-BUILD-002`, the combined
`exe,lib` artifacts are exercised against one adjacent module, and runtime tests cover concurrent
generation pinning and malformed metadata. The recorded switch measurement supports the O(1)
activation contract in §8. D-126 records the phase choices.

Phase 10 is the documentation and handover pass (2026-10-25). The schema text is unchanged:
format 1 remains what M4.5 emits, M5 extends the existing record tables without a format bump,
and the M4.5 implementation follows these formats exactly. (M6.5 phase 4 later raised only
the GOT manifest to format 2; the descriptor, module, and record tables remain format 1,
D-180.) The phase clarifies the debug flags
(§7.1): M4.5's default artifact form may set the entry-flag debug bit but emits no debug
information or checkpoint hooks, and the debug-checkpoint required-feature bit stays clear until
M6.5. D-127 records the pass; the M6.5 boundary is in
[Debug and Monitoring](Debug-and-Monitoring.md) §12.

M5 phase 4b.0 (2026-10-25) records host ABI 2.1 and §8.5. Minor 1 appends no service; it fixes
the TYPEINFO, VTABLE, object-record, and interface-value layouts that generated OOP code reads
inline, the hidden `HANDLE`/`OBJREC` root-key convention, interface-prototype function records
with trap-stub cells, and the inline access rules. Descriptor records keep the owning type's
type key, layout hash, size, and alignment; the record layout is separate and versioned. The
module and manifest formats remain 1. D-147–D-152 record the phase decisions; the catalog
projection, host registry, dispatch, and attempt code land in phases 4b.1–4b.5, and 4b.6
(2026-10-25) verifies the format end to end with real module warm swaps, preserved interface
values and class handles, descriptor identity, vtable-shape rejection, fresh records, rollback,
and OOP manifest/descriptor fuzz seeds.

M5 phase 4b.1 is implemented (2026-10-25) in `stc-codegen` and `stc-got`. `types` projects the
class value/instance layout split, `PTR_TO` handle keys for class-typed fields and leaves, the
object-record type key (`ARRAY[0..N]OF(PTR_TO(T:…))` with `N + 1` pointers), the base-first
interface map and per-interface vtable order, and per-object instance leaves. `module` emits one
TYPEINFO per class/FB (and a VTABLE when class slots exist), interface prototypes as method
function records with trap-stub symbols, one hidden `OBJREC` root per persistent instance and one
hidden `HANDLE` root per class value, and sets required-feature bits 2 and 3. `unit` emits the
trap stubs and fills fresh records and handles in `stc_module_prepare`; `stc-got` accepts the
markers while keeping format 1 and rejects a marker on a non-hidden root or a pointer type on an
ordinary root. Class-owned instance fields remain ordinary retained leaves; handle and record
roots are non-retained. Recursive class ownership has no finite persistent storage and is
rejected at the declaration.

M5 phase 4b.2 is implemented (2026-10-25) in `stc-runtime`. The host owns a descriptor registry:
TYPEINFO and class VTABLE records are keyed by the catalog descriptor `GotId`, and
per-(object type, interface) VTABLEs are keyed by the two type identities because §8.5 gives them
no catalog identity. Prepare validates descriptor size/alignment/type-layout identity, object
leaf counts against hidden `OBJREC` roots, class slot counts, contiguous interface prototype
slots, base-first interface maps, and prototype trap-stub cells; it binds the generation's
descriptor cells before `stc_module_prepare`. A compatible shape reuses its stable record
address, shadow contents are patched at activation, shape changes and removals that a preserved
object or interface value can depend on are rejected with `STC-E-ONLINE-003`, and removed
descriptors quarantine with their roots. The service table, `REQUIRED_SIZE`, and both artifact
formats remain unchanged.

M6 phase 0 (2026-10-26) fixed the next host minor at 2.2 and assigned service ID 12,
`set_cycle_time_ns(int64_t)`, for the test prelude (D-157). It changed no descriptor, manifest,
plan, note, or build-ID format, no existing service ID or signature, and no task record; the
2.2 append is implemented in phase 6 and is imported only by test builds. No debug service or
checkpoint hook is added in M6. The rest of M6 (manifest, state, I/O, scheduler, and online
workflow) consumed the existing formats without change.

M6.5 phase 0 (2026-10-27) fixes the next host minor at 2.3 and service ID 13, `debug_check`, for
checkpoint hooks (D-173–D-179). It appends no entry point, descriptor field, manifest record, or
format change; debug builds that emit hooks require minor 2.3, test builds stay at 2.2, and
hook-free debug and release builds stay at 2.1. The checkpoint table is in the metadata sidecar's
`debug` object, paired by build ID and excluded from descriptor/manifest identity and
online-change compatibility; `required_features` bit 4 and `entry_flags` bit 6 are set only by
hook-emitting debug builds. The debug protocol and session are specified in
[Debug and Monitoring](Debug-and-Monitoring.md) §13–§14 and use no host API entry other than
service 13.

M6.5 phase 4 (2026-10-27) raised only the GOT manifest to format 2 (D-180). The new
`TypeMetadata` (§9.3 tag 12) and `Aggregates` (§9.3 tag 13) sections carry enum member sets,
subrange bounds, and split-storage shape records; both are required in every format-2 manifest
and are part of `catalog_digest` and the build ID. The descriptor, module, state, metadata
sidecar, and host ABI formats are unchanged; the descriptor's `manifest_format` field is 2 and
pairs only with a format-2 manifest. `stc-codegen` projects the new records deterministically
(type metadata by type identity, aggregates by package/path/qualifier, program fields by path),
`stc-got` validates and round-trips both binary and canonical-JSON projections, and the host
rejects format-1 manifests through the normal version/pairing path. Fuzz seeds now include a
valid format-2 binary and JSON manifest; the older format-1 seeds remain explicit negative
inputs. The runtime uses the metadata for enum/subrange write validation and for whole-program
and array-of-aggregate shape resolution; see [Debug and Monitoring](Debug-and-Monitoring.md)
§15.

## 13. Related documents

- [Online Change and GOT](Online-Change-and-GOT.md) — host/module model, lifecycle, policy.
- [Runtime and Execution](Runtime-and-Execution.md) — runtime services and execution.
- [Architecture](Architecture.md) — pipeline, codegen, determinism, security.
- [CLI and Manifest](CLI-and-Manifest.md) — commands, options, artifact layout.
- [State and RETAIN](State-and-Retain.md) — retained-state file and cold/warm policy.
- [Testing and Verification](Testing-and-Verification.md) — hot-swap and manifest tests.
- [Decisions](../Decisions.md) — D-113–D-127, the prior online-change decisions, and M6.5
  debug decisions D-173–D-179.
