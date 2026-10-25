# GOT Artifacts and Schemas

Status: M4.5 phase 1 design finalized (2026-10-24)  
Profile: `stc-2013-oop-utf8-v1`  
Scope: Normative formats for stable keys, `GotId`, the compiler catalog, the module descriptor,
the persistent GOT manifest, the online-change plan, host ABI 2.0, and build IDs.

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
| Host API | 2.0 | `STC_HOST_ABI_MAJOR=2`, `STC_HOST_ABI_MINOR=0` |
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
`#DESC:`, and `.GUARD` markers use uppercase spellings. A renamed declaration keeps its `GotId`
through a tracked rename or move even though its key changes; the old key is retained in
`previous_keys`.

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
  `PTR_TO` type key.

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
  persistent roots; they use direct cells and a direct identity record.
- **Functions:** one record per user function, program body, method, FB body, standard function
  that is called indirectly, and export wrapper. Compiler-generated helpers are module-local and
  do not get identities.
- **Types:** one record per named user-defined type that owns persistent layout or descriptors.
  Elementary types are implicit and are not listed.
- **Descriptors:** one record per runtime-owned type descriptor, vtable, or object descriptor.
  M4.5 emits none; M5 populates them.
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
  "stc:catalog:v1\0" ||
  roots_section_payload || functions_section_payload || types_section_payload ||
  descriptors_section_payload || instances_section_payload || tasks_section_payload ||
  exports_section_payload || slots_section_payload)[0..32]
```

Each `*_section_payload` is the encoded payload defined in §9.2, without its section tag and
length. The digest covers source locations because they are fields of the records. It does not
cover the manifest header, plan, or history.

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
host_abi_minor       u16   = 0
module_format        u16   = 1
endianness           u8    = 0   (0 = little, 1 = big)
pointer_width        u8    = 8
profile              string       exact profile ID
compiler_version     string       compiler package version, ASCII
target_triple        string       resolved target triple
build_id             id128
manifest_digest      digest32
manifest_format      u16   = 1
entry_flags          u32
function_count       u32
slot_count           u32
functions            list<FunctionRecord>
slots                list<SlotRecord>
```

`descriptor_size` must equal the blob length exactly. `descriptor_flags` is zero in format 1;
nonzero values are invalid. `entry_flags` bits:

| Bit | Meaning |
| --- | --- |
| 0 | `stc_module_prepare` performs root initialization |
| 1 | `stc_module_cycle` is callable |
| 2 | `stc_module_shutdown` performs retained-state work |
| 3 | `stc_module_detach` releases module-local resources |
| 4 | Debug build; debug information and checkpoint hooks are present |
| 5 | The module uses retained-state host services |
| 6 | Debug hooks are active (M6.5) |
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
| 0 | Host ABI 2.0 base services |
| 1 | Online-change plan present in the paired manifest |
| 2 | OOP descriptors/vtables in use (M5) |
| 3 | Place-descriptor references in use (M5) |
| 4 | Debug checkpoint hooks in use (M6.5) |
| 5–63 | Reserved; a host that does not implement a set bit rejects the module |

M4.5 sets bit 0 and bit 1.

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

## 8. Host API 2.0

### 8.1 Table

```c
typedef struct stc_host_api {
    uint32_t abi_version;   /* (major << 16) | minor; 2.0 is 0x00020000 */
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
} stc_host_api;
```

The table is host-owned and read-only to the module. The module receives it in
`stc_module_attach` and caches it. Fields are appended only within a major version, and
`struct_size` grows accordingly. The host and module use the same target C ABI, so field offsets
are identical; `struct_size` protects against a module built against a newer minor version.

An import slot's host cell holds the address of the corresponding table entry. The host binds
import cells when it builds the generation's slot table; the module reads them like function
cells. The module may also use its cached host API pointer for a service that has no import
slot, but M4.5-generated code uses import slots for every service it calls.

### 8.2 Services

Service IDs are used by import slot records and are fixed for ABI 2.0:

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

`alloc` blocks are zeroed, stable for the lifetime of the requesting module generation, and freed
on failed prepare, retire, or unload. The host rejects an attempt to register a transient block
as a persistent root. `free` accepts only pointers returned by `alloc` in the same generation.

`io_image` areas are process-image areas, not direct-variable cells. Direct cells are separately
resolved by the host per generation. Retained-state status values follow
[State and RETAIN](State-and-Retain.md); M4.5's file-backed state remains M6, so the M4.5 host
returns the documented "disabled" status.

The `assert` entry exists for the M6 test runner and M5 hosted checks. M4.5 codegen does not emit
assertions in production artifacts.

### 8.3 Compatibility

`abi_version` is `(major << 16) | minor`, with major 2 and minor 0 for M4.5. A module's
descriptor carries the required major/minor. The host accepts a module when:

1. majors are equal, and
2. host minor is greater than or equal to the module's required minor, and
3. host `struct_size` covers every service the module uses.

A module built for 2.0 runs on a 2.1 host. A 2.1 module is rejected by a 2.0 host. A breaking
change to a service signature or the struct prefix increments the major version. M4 ABI 1
artifacts are not loadable.

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

## 9. GOT manifest format 1

### 9.1 Files and outer header

The binary manifest is `<name>.stc-got.bin`. Its outer header is:

```text
magic        [8]u8 = "STCGOT\0\0"
format       u16   = 1
flags        u16   = 0
body_length  u64
digest       digest32
body         [body_length]u8
```

`flags` is zero in format 1; nonzero values are invalid. `body` is the concatenation of the
sections in §9.3. `digest` is defined in §9.5. Bytes after `body_length` are invalid.

The JSON manifest is `<name>.stc-got.json`:

```json
{
  "body": {
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
    "types": []
  },
  "digest": "0x…",
  "flags": 0,
  "format": 1
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
one-byte or one-word completion flags keyed by task identity. `size` is the root's storage size
in bytes; `align` is a power of two. `field_path` is relative to the owner type and is empty for
globals and direct variables that are not fields. A retained root has `retention = Retain`;
other roots use `None`. `initializer_digest` hashes the canonical initializer description
(`"stc:init:v1\0" || type_key || 0x00 || canonical value bytes` for constants, or the
deterministic init-expression fingerprint for expressions); the value itself is not stored in
the manifest.

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

Function flags reuse the descriptor bits in §7.1. `vtable_slot` is reserved for M5 method
records. `export_name` is the C symbol for wrapper and no-mangle records.

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
emits no descriptors.

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
- history: `(retired_build_id, got_id)` ascending.

`previous_keys` are ordered oldest first, contain no duplicates, and never contain the current
key. They are the tracked rename/move history of one identity.

A non-canonical order is invalid. This lets validation and double-build tests compare bytes
directly and makes reordering tampering detectable.

### 9.4 Manifest digest

```text
manifest_digest = SHA-256("stc:manifest:v1\0" || body)[0..32]
```

`body` is the exact concatenation of the sections (tag, length, payload) in §9.3. The digest
does not include the outer header's magic, format, flags, body length, or digest field. The
`parent_manifest_digest` recorded by a child build is the parent's `manifest_digest`.

### 9.5 Validation

A manifest is accepted only when all of the following hold:

1. The outer magic, format, and flags match; `body_length` exactly covers the body; the digest
   matches.
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
12. The artifact paired with a module satisfies §11.4.

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
| `catalog_digest` | §6.3 |
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
   module format, and host ABI.
4. The descriptor's function set equals the manifest's function set (`GotId`, key, signature,
   and flags), and the descriptor's slot records equal the manifest's slot records exactly
   (`slot`, `kind`, `service`, `got_id`); every `Descriptor` slot resolves to a descriptor
   record in the manifest descriptor table.
5. The descriptor's `required_features` equals the manifest header's `required_features`, and
   the descriptor size, counts, and feature bits validate.

The host performs this check before attach. A mismatch is `STC-E-MANIFEST-001` or
`STC-E-MODULE-001` and selects the cold-download path; the active generation is untouched.

The pairing binds the descriptor, its function/slot tables, and the manifest. It does not hash
the complete platform image, because loaders may rewrite headers and modules are trusted native
code in v1 (see [Architecture](Architecture.md) §8). Build IDs, immutable versioned file names,
and the manifest digest provide deployment identity; code authenticity is not a v1 security
boundary.

### 11.5 Runtime currentness

After pairing, the host compares the manifest's `parent_build_id` and `parent_manifest_digest`
with the active generation's build ID and manifest digest. An exact match permits an online
attempt. A missing parent, a mismatch, a missing/stale/corrupt active manifest, or a target/ABI
mismatch permits only a cold download. Coincidentally equal `GotId`s never authorize an online
change without the exact parent pair.

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
- **Instances and tasks:** the selected or synthesized configuration, one resource, one task,
  and the program instances it runs. Multi-task topologies remain rejected at build time.
- **Exports:** `{stc:export}` wrappers and their C names.
- **Slots:** data, function, direct, and import slots as assigned in §6.2; descriptor slots are
  empty.
- **Plan:** `Preserve`, `Allocate`, `Drop`, and `Reject`; `Migrate`/`Reset` are not emitted.
- **Host services:** trap, clocks, I/O image, and retained-state stubs. Transient `alloc`/`free`
  and `assert` are in ABI 2.0 but unused by M4.5-generated code.

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

The implementation phases that follow must add:

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
records the choices. Implementation of host API bindings and runtime loading in later phases must
follow these formats exactly, and any further change discovered during implementation returns to
this document and the decision log before code lands.

Phase 6 is implemented (2026-10-25) in `stc-runtime`. `loader` is the platform layer over
`LoadLibraryW`/`GetProcAddress`/`FreeLibrary` and `dlopen`/`dlsym`/`dlclose`; it resolves the seven
§7.3 entry points, copies the descriptor blob and function table, and treats load failures as
non-fatal module errors. `host_api` is the static ABI 2.0 table described in §8.1: import cells
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
trap-boundary rollback and the C-facing launcher/embedding exports remain with phase 7 execution
conversion. D-123 records the phase choices.

## 13. Related documents

- [Online Change and GOT](Online-Change-and-GOT.md) — host/module model, lifecycle, policy.
- [Runtime and Execution](Runtime-and-Execution.md) — runtime services and execution.
- [Architecture](Architecture.md) — pipeline, codegen, determinism, security.
- [CLI and Manifest](CLI-and-Manifest.md) — commands, options, artifact layout.
- [State and RETAIN](State-and-Retain.md) — retained-state file and cold/warm policy.
- [Testing and Verification](Testing-and-Verification.md) — hot-swap and manifest tests.
- [Decisions](../Decisions.md) — D-113–D-118 and the prior online-change decisions.
