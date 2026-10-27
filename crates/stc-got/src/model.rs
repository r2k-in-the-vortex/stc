//! Manifest record model and its binary projection (GOT Artifacts §9.2).

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result};
use crate::wire::{Decoder, Encoder};

macro_rules! tagged_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $variant:ident = $tag:literal => $json:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $(
                #[doc = concat!("`", $json, "`.")]
                $variant,
            )+
        }

        impl $name {
            /// Returns the binary tag.
            #[must_use]
            pub const fn tag(self) -> u8 {
                match self {
                    $( Self::$variant => $tag, )+
                }
            }

            /// Returns the canonical JSON name.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $( Self::$variant => $json, )+
                }
            }

            /// Parses the binary tag.
            #[must_use]
            pub const fn from_tag(tag: u8) -> Option<Self> {
                match tag {
                    $( $tag => Some(Self::$variant), )+
                    _ => None,
                }
            }

            /// Parses the canonical JSON name.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $( $json => Some(Self::$variant), )+
                    _ => None,
                }
            }
        }
    };
}

tagged_enum! {
    /// The storage shape of a persistent root.
    pub enum RootKind {
        Global = 1 => "global",
        ProgramField = 2 => "program_field",
        NestedField = 3 => "nested_field",
        Direct = 4 => "direct",
        TaskGuard = 5 => "task_guard",
    }
}

tagged_enum! {
    /// The retention policy of a root or field.
    pub enum Retention {
        None = 0 => "none",
        Retain = 1 => "retain",
    }
}

tagged_enum! {
    /// How a root's initial value is described.
    pub enum InitializerKind {
        None = 0 => "none",
        Zero = 1 => "zero",
        Constant = 2 => "constant",
        Expression = 3 => "expression",
    }
}

tagged_enum! {
    /// The kind of a callable record.
    pub enum FunctionKind {
        Function = 1 => "function",
        ProgramBody = 2 => "program_body",
        FbBody = 3 => "fb_body",
        Method = 4 => "method",
        ExportWrapper = 5 => "export_wrapper",
        StdFunction = 6 => "std_function",
    }
}

tagged_enum! {
    /// The kind of a named user type.
    pub enum TypeKind {
        Struct = 1 => "struct",
        FunctionBlock = 2 => "function_block",
        Class = 3 => "class",
        Interface = 4 => "interface",
        Enum = 5 => "enum",
        Subrange = 6 => "subrange",
        Alias = 7 => "alias",
    }
}

tagged_enum! {
    /// How an aggregate shape is described.
    pub enum AggregateKind {
        Type = 1 => "type",
        Program = 2 => "program",
    }
}

tagged_enum! {
    /// The kind of a runtime descriptor.
    pub enum DescriptorKind {
        TypeInfo = 1 => "type_info",
        Vtable = 2 => "vtable",
        Object = 3 => "object",
    }
}

tagged_enum! {
    /// The kind of a GOT cell.
    pub enum SlotKind {
        Data = 1 => "data",
        Function = 2 => "function",
        Descriptor = 3 => "descriptor",
        Direct = 4 => "direct",
        Import = 5 => "import",
    }
}

tagged_enum! {
    /// The compiler's verdict for an online-change plan.
    pub enum PlanVerdict {
        Cold = 1 => "cold",
        OnlineCandidate = 2 => "online_candidate",
        Rejected = 3 => "rejected",
    }
}

tagged_enum! {
    /// One per-identity plan action.
    pub enum PlanAction {
        Preserve = 1 => "preserve",
        Allocate = 2 => "allocate",
        Migrate = 3 => "migrate",
        Reset = 4 => "reset",
        Drop = 5 => "drop",
        Reject = 6 => "reject",
    }
}

tagged_enum! {
    /// Why an identity left the active manifest.
    pub enum RetiredCause {
        Dropped = 1 => "dropped",
        Reset = 2 => "reset",
        Replaced = 3 => "replaced",
    }
}

/// A package-relative source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    /// Package identity scope.
    pub package: String,
    /// Package-relative path with `/` separators.
    pub file: String,
    /// Start byte offset.
    pub start: u32,
    /// End byte offset.
    pub end: u32,
}

/// The manifest header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Profile identifier.
    pub profile: String,
    /// Compiler package version.
    pub compiler_version: String,
    /// Resolved target triple.
    pub target_triple: String,
    /// Module name.
    pub module_name: String,
    /// Active configuration name (empty when implicit).
    pub active_configuration: String,
    /// Byte-order tag.
    pub endianness: u8,
    /// Pointer width in bytes.
    pub pointer_width: u8,
    /// Module format.
    pub module_format: u16,
    /// Host ABI major.
    pub host_abi_major: u16,
    /// Host ABI minor.
    pub host_abi_minor: u16,
    /// Generator flags.
    pub generator_flags: u64,
    /// Required feature bits.
    pub required_features: u64,
    /// Content-derived build identity.
    pub build_id: GotId,
    /// Parent build identity.
    pub parent_build_id: Option<GotId>,
    /// Parent manifest digest.
    pub parent_manifest_digest: Option<Digest32>,
    /// Paired descriptor digest.
    pub descriptor_digest: Digest32,
    /// Source fingerprint.
    pub source_fingerprint: Digest32,
    /// Catalog digest.
    pub catalog_digest: Digest32,
    /// Plan digest.
    pub plan_digest: Digest32,
}

/// One persistent data-root record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Root shape.
    pub root_kind: RootKind,
    /// Owning type identity.
    pub owner_type: Option<GotId>,
    /// Field path relative to the owner type.
    pub field_path: String,
    /// Canonical type key.
    pub type_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Storage size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Retention policy.
    pub retention: Retention,
    /// Initializer category.
    pub initializer: InitializerKind,
    /// Initializer fingerprint.
    pub initializer_digest: Option<Digest32>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
    /// GOT slot.
    pub slot: u32,
}

/// One callable record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Signature key.
    pub signature_key: String,
    /// Callable kind.
    pub function_kind: FunctionKind,
    /// Owning type identity.
    pub owner_type: Option<GotId>,
    /// Descriptive flags.
    pub flags: u32,
    /// Function cell slot.
    pub slot: u32,
    /// C export symbol, when this callable backs an export.
    pub export_name: Option<String>,
    /// M5 vtable slot.
    pub vtable_slot: Option<u32>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One field inside a named type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRecord {
    /// Field identity.
    pub got_id: GotId,
    /// Field path.
    pub path: String,
    /// Canonical type key.
    pub type_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Byte offset.
    pub offset: u64,
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Retention policy.
    pub retention: Retention,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One named user type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Canonical type key.
    pub type_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Type kind.
    pub type_kind: TypeKind,
    /// Base type identity.
    pub base: Option<GotId>,
    /// Implemented or extended interfaces.
    pub interfaces: Vec<GotId>,
    /// Vtable shape hash.
    pub vtable_shape: Option<GotId>,
    /// Fields, sorted by `(offset, path)`.
    pub fields: Vec<FieldRecord>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One enum member in the format-2 type metadata section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumMemberRecord {
    /// Normalized member name.
    pub name: String,
    /// Integer member value.
    pub value: i128,
}

/// Subrange bounds in the format-2 type metadata section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubrangeBounds {
    /// Inclusive lower bound.
    pub lower: i128,
    /// Inclusive upper bound.
    pub upper: i128,
}

/// Type metadata that the format-1 type table does not carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeMetadataRecord {
    /// Identity of the named type record this metadata belongs to.
    pub type_id: GotId,
    /// Enum member set, sorted by `(value, name)`.
    pub enum_members: Vec<EnumMemberRecord>,
    /// Subrange bounds.
    pub subrange: Option<SubrangeBounds>,
}

/// One top-level field of a program aggregate shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateFieldRecord {
    /// Top-level field path, normalized.
    pub path: String,
    /// Declared canonical type key.
    pub type_key: String,
    /// Target layout hash of the declared type.
    pub layout_hash: GotId,
}

/// One aggregate shape used to resolve split storage.
///
/// A `type` shape names the declared type at `path`. A `program` shape lists
/// the top-level fields of one configured program instance; its `shape_key`
/// is the program path from the instance record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateRecord {
    /// Package name.
    pub package: String,
    /// Package-relative aggregate path.
    pub path: String,
    /// Instance qualifier for a program shape.
    pub qualifier: Option<String>,
    /// Shape kind.
    pub kind: AggregateKind,
    /// Declared type key (`type`) or program path (`program`).
    pub shape_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Top-level fields, sorted by path; empty for a `type` shape.
    pub fields: Vec<AggregateFieldRecord>,
}

/// One runtime descriptor or vtable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Descriptor kind.
    pub descriptor_kind: DescriptorKind,
    /// Owning type identity.
    pub owner_type: Option<GotId>,
    /// Canonical type key.
    pub type_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Descriptor slot.
    pub slot: u32,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One configured program instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Program type identity.
    pub program_type: Option<GotId>,
    /// Program type path or identity key.
    pub program_type_key: String,
    /// Configuration name.
    pub configuration: String,
    /// Resource name.
    pub resource: String,
    /// Instance path.
    pub instance_path: String,
    /// Bound task key.
    pub task_key: Option<String>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Configuration name.
    pub configuration: String,
    /// Resource name.
    pub resource: String,
    /// Task name.
    pub name: String,
    /// `SINGLE` attribute.
    pub single: bool,
    /// Interval in nanoseconds.
    pub interval_ns: Option<i64>,
    /// Priority.
    pub priority: Option<i64>,
    /// Task-guard root identity.
    pub guard_got_id: Option<GotId>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One C export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRecord {
    /// Persistent identity.
    pub got_id: GotId,
    /// Current stable key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// C symbol name.
    pub c_name: String,
    /// Wrapped function identity.
    pub function_got_id: GotId,
    /// Wrapped function signature key.
    pub signature_key: String,
    /// `{stc:no_mangle}` marker.
    pub no_mangle: bool,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One GOT cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotRecord {
    /// Slot number.
    pub slot: u32,
    /// Cell kind.
    pub kind: SlotKind,
    /// Host service ID for import slots, zero otherwise.
    pub service: u8,
    /// Bound identity, zero for import slots.
    pub got_id: GotId,
}

/// One plan action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRecord {
    /// Affected identity.
    pub got_id: GotId,
    /// Identity kind tag.
    pub kind: u8,
    /// New key.
    pub key: String,
    /// Baseline key when it differs from `key`.
    pub old_key: Option<String>,
    /// Action.
    pub action: PlanAction,
    /// Stable reason code.
    pub reason_code: String,
    /// Human-readable detail.
    pub detail: String,
}

/// One plan diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanDiagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// Warning or error tag.
    pub severity: u8,
    /// Human-readable message.
    pub message: String,
    /// Affected identity.
    pub got_id: Option<GotId>,
    /// Baseline key.
    pub old_key: Option<String>,
    /// New key.
    pub new_key: Option<String>,
}

/// The online-change plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Compiler verdict.
    pub verdict: PlanVerdict,
    /// Parent build identity.
    pub parent_build_id: Option<GotId>,
    /// Parent manifest digest.
    pub parent_manifest_digest: Option<Digest32>,
    /// Actions sorted by `(got_id)`.
    pub actions: Vec<ActionRecord>,
    /// Diagnostics in canonical order.
    pub diagnostics: Vec<PlanDiagnostic>,
}

/// One retired identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetiredRecord {
    /// Retired identity.
    pub got_id: GotId,
    /// Identity kind tag.
    pub kind: u8,
    /// Last current key.
    pub key: String,
    /// Previous keys, oldest first.
    pub previous_keys: Vec<String>,
    /// Build in which the identity was active.
    pub retired_build_id: GotId,
    /// Manifest in which the identity was active.
    pub retired_manifest_digest: Digest32,
    /// Retirement cause.
    pub cause: RetiredCause,
}

/// Build history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// Retired records.
    pub retired: Vec<RetiredRecord>,
}

/// A type that has a binary encoding.
pub trait Encode {
    /// Appends the encoding to `out`.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    fn encode(&self, out: &mut Encoder) -> Result<()>;
}

/// A type that has a binary decoding.
pub trait Decode: Sized {
    /// Decodes one value.
    ///
    /// # Errors
    ///
    /// Rejects malformed or truncated input.
    fn decode(input: &mut Decoder<'_>) -> Result<Self>;
}

fn encode_string_list(out: &mut Encoder, values: &[String]) -> Result<()> {
    out.list_header(values.len())?;
    for value in values {
        out.string(value)?;
    }
    Ok(())
}

fn decode_string_list(input: &mut Decoder<'_>) -> Result<Vec<String>> {
    let count = input.count(4)?;
    let mut values = Vec::new();
    for _ in 0..count {
        values.push(input.string()?);
    }
    Ok(values)
}

fn encode_id_list(out: &mut Encoder, values: &[GotId]) -> Result<()> {
    out.list_header(values.len())?;
    for value in values {
        out.id128(*value);
    }
    Ok(())
}

fn decode_id_list(input: &mut Decoder<'_>) -> Result<Vec<GotId>> {
    let count = input.count(16)?;
    let mut values = Vec::new();
    for _ in 0..count {
        values.push(input.id128()?);
    }
    Ok(values)
}

fn encode_option<T: Encode>(out: &mut Encoder, value: &Option<T>) -> Result<()> {
    match value {
        Some(value) => {
            out.u8(1);
            value.encode(out)
        }
        None => {
            out.u8(0);
            Ok(())
        }
    }
}

fn decode_option<T: Decode>(input: &mut Decoder<'_>) -> Result<Option<T>> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(T::decode(input)?)),
        tag => Err(Error::manifest(format!(
            "invalid option tag {tag} at offset {}",
            input.position() - 1
        ))),
    }
}

fn encode_enum<T: Tagged>(out: &mut Encoder, value: T) {
    out.tag(value.tag());
}

fn decode_enum<T: Tagged>(input: &mut Decoder<'_>, what: &str) -> Result<T> {
    let tag = input.tag()?;
    T::from_tag(tag).ok_or_else(|| Error::manifest(format!("invalid {what} tag {tag}")))
}

/// Internal enum access used by the generic codecs.
pub trait Tagged: Copy {
    /// Returns the binary tag.
    fn tag(self) -> u8;
    /// Parses the binary tag.
    fn from_tag(tag: u8) -> Option<Self>;
}

macro_rules! tagged_impl {
    ($name:ident) => {
        impl Tagged for $name {
            fn tag(self) -> u8 {
                $name::tag(self)
            }

            fn from_tag(tag: u8) -> Option<Self> {
                $name::from_tag(tag)
            }
        }
    };
}

tagged_impl!(RootKind);
tagged_impl!(Retention);
tagged_impl!(InitializerKind);
tagged_impl!(FunctionKind);
tagged_impl!(TypeKind);
tagged_impl!(AggregateKind);
tagged_impl!(DescriptorKind);
tagged_impl!(SlotKind);
tagged_impl!(PlanVerdict);
tagged_impl!(PlanAction);
tagged_impl!(RetiredCause);

impl Encode for SourceLocation {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.string(&self.package)?;
        out.string(&self.file)?;
        out.u32(self.start);
        out.u32(self.end);
        Ok(())
    }
}

impl Decode for SourceLocation {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            package: input.string()?,
            file: input.string()?,
            start: input.u32()?,
            end: input.u32()?,
        })
    }
}

impl Encode for GotId {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(*self);
        Ok(())
    }
}

impl Decode for GotId {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        input.id128()
    }
}

impl Encode for Digest32 {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.digest32(*self);
        Ok(())
    }
}

impl Decode for Digest32 {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        input.digest32()
    }
}

impl Encode for String {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.string(self)
    }
}

impl Decode for String {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        input.string()
    }
}

impl Encode for u32 {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.u32(*self);
        Ok(())
    }
}

impl Decode for u32 {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        input.u32()
    }
}

impl Encode for i64 {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.i64(*self);
        Ok(())
    }
}

impl Decode for i64 {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        input.i64()
    }
}

impl Encode for Header {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.string(&self.profile)?;
        out.string(&self.compiler_version)?;
        out.string(&self.target_triple)?;
        out.string(&self.module_name)?;
        out.string(&self.active_configuration)?;
        out.u8(self.endianness);
        out.u8(self.pointer_width);
        out.u16(self.module_format);
        out.u16(self.host_abi_major);
        out.u16(self.host_abi_minor);
        out.u64(self.generator_flags);
        out.u64(self.required_features);
        out.id128(self.build_id);
        encode_option(out, &self.parent_build_id)?;
        encode_option(out, &self.parent_manifest_digest)?;
        out.digest32(self.descriptor_digest);
        out.digest32(self.source_fingerprint);
        out.digest32(self.catalog_digest);
        out.digest32(self.plan_digest);
        Ok(())
    }
}

impl Decode for Header {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            profile: input.string()?,
            compiler_version: input.string()?,
            target_triple: input.string()?,
            module_name: input.string()?,
            active_configuration: input.string()?,
            endianness: input.u8()?,
            pointer_width: input.u8()?,
            module_format: input.u16()?,
            host_abi_major: input.u16()?,
            host_abi_minor: input.u16()?,
            generator_flags: input.u64()?,
            required_features: input.u64()?,
            build_id: input.id128()?,
            parent_build_id: decode_option(input)?,
            parent_manifest_digest: decode_option(input)?,
            descriptor_digest: input.digest32()?,
            source_fingerprint: input.digest32()?,
            catalog_digest: input.digest32()?,
            plan_digest: input.digest32()?,
        })
    }
}

impl Encode for RootRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        encode_enum(out, self.root_kind);
        encode_option(out, &self.owner_type)?;
        out.string(&self.field_path)?;
        out.string(&self.type_key)?;
        out.id128(self.layout_hash);
        out.u64(self.size);
        out.u32(self.align);
        encode_enum(out, self.retention);
        encode_enum(out, self.initializer);
        encode_option(out, &self.initializer_digest)?;
        encode_option(out, &self.source)?;
        out.u32(self.slot);
        Ok(())
    }
}

impl Decode for RootRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            root_kind: decode_enum(input, "root kind")?,
            owner_type: decode_option(input)?,
            field_path: input.string()?,
            type_key: input.string()?,
            layout_hash: input.id128()?,
            size: input.u64()?,
            align: input.u32()?,
            retention: decode_enum(input, "retention")?,
            initializer: decode_enum(input, "initializer")?,
            initializer_digest: decode_option(input)?,
            source: decode_option(input)?,
            slot: input.u32()?,
        })
    }
}

impl Encode for FunctionRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        out.string(&self.signature_key)?;
        encode_enum(out, self.function_kind);
        encode_option(out, &self.owner_type)?;
        out.u32(self.flags);
        out.u32(self.slot);
        encode_option(out, &self.export_name)?;
        encode_option(out, &self.vtable_slot)?;
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for FunctionRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            signature_key: input.string()?,
            function_kind: decode_enum(input, "function kind")?,
            owner_type: decode_option(input)?,
            flags: input.u32()?,
            slot: input.u32()?,
            export_name: decode_option(input)?,
            vtable_slot: decode_option(input)?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for FieldRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.path)?;
        out.string(&self.type_key)?;
        out.id128(self.layout_hash);
        out.u64(self.offset);
        out.u64(self.size);
        out.u32(self.align);
        encode_enum(out, self.retention);
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for FieldRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            path: input.string()?,
            type_key: input.string()?,
            layout_hash: input.id128()?,
            offset: input.u64()?,
            size: input.u64()?,
            align: input.u32()?,
            retention: decode_enum(input, "retention")?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for TypeRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        out.string(&self.type_key)?;
        out.id128(self.layout_hash);
        out.u64(self.size);
        out.u32(self.align);
        encode_enum(out, self.type_kind);
        encode_option(out, &self.base)?;
        encode_id_list(out, &self.interfaces)?;
        encode_option(out, &self.vtable_shape)?;
        out.list_header(self.fields.len())?;
        for field in &self.fields {
            field.encode(out)?;
        }
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for TypeRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        let got_id = input.id128()?;
        let key = input.string()?;
        let previous_keys = decode_string_list(input)?;
        let type_key = input.string()?;
        let layout_hash = input.id128()?;
        let size = input.u64()?;
        let align = input.u32()?;
        let type_kind = decode_enum(input, "type kind")?;
        let base = decode_option(input)?;
        let interfaces = decode_id_list(input)?;
        let vtable_shape = decode_option(input)?;
        let field_count = input.count(1)?;
        let mut fields = Vec::new();
        for _ in 0..field_count {
            fields.push(FieldRecord::decode(input)?);
        }
        let source = decode_option(input)?;
        Ok(Self {
            got_id,
            key,
            previous_keys,
            type_key,
            layout_hash,
            size,
            align,
            type_kind,
            base,
            interfaces,
            vtable_shape,
            fields,
            source,
        })
    }
}

impl Encode for SubrangeBounds {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.i128(self.lower);
        out.i128(self.upper);
        Ok(())
    }
}

impl Decode for SubrangeBounds {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            lower: input.i128()?,
            upper: input.i128()?,
        })
    }
}

impl Encode for TypeMetadataRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.type_id);
        out.list_header(self.enum_members.len())?;
        for member in &self.enum_members {
            out.string(&member.name)?;
            out.i128(member.value);
        }
        encode_option(out, &self.subrange)?;
        Ok(())
    }
}

impl Decode for TypeMetadataRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        let type_id = input.id128()?;
        let count = input.count(1)?;
        let mut enum_members = Vec::new();
        for _ in 0..count {
            enum_members.push(EnumMemberRecord {
                name: input.string()?,
                value: input.i128()?,
            });
        }
        let subrange = decode_option(input)?;
        Ok(Self {
            type_id,
            enum_members,
            subrange,
        })
    }
}

impl Encode for AggregateRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.string(&self.package)?;
        out.string(&self.path)?;
        encode_option(out, &self.qualifier)?;
        encode_enum(out, self.kind);
        out.string(&self.shape_key)?;
        out.id128(self.layout_hash);
        out.list_header(self.fields.len())?;
        for field in &self.fields {
            out.string(&field.path)?;
            out.string(&field.type_key)?;
            out.id128(field.layout_hash);
        }
        Ok(())
    }
}

impl Decode for AggregateRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        let package = input.string()?;
        let path = input.string()?;
        let qualifier = decode_option(input)?;
        let kind = decode_enum(input, "aggregate kind")?;
        let shape_key = input.string()?;
        let layout_hash = input.id128()?;
        let count = input.count(1)?;
        let mut fields = Vec::new();
        for _ in 0..count {
            fields.push(AggregateFieldRecord {
                path: input.string()?,
                type_key: input.string()?,
                layout_hash: input.id128()?,
            });
        }
        Ok(Self {
            package,
            path,
            qualifier,
            kind,
            shape_key,
            layout_hash,
            fields,
        })
    }
}

impl Encode for DescriptorRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        encode_enum(out, self.descriptor_kind);
        encode_option(out, &self.owner_type)?;
        out.string(&self.type_key)?;
        out.id128(self.layout_hash);
        out.u64(self.size);
        out.u32(self.align);
        out.u32(self.slot);
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for DescriptorRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            descriptor_kind: decode_enum(input, "descriptor kind")?,
            owner_type: decode_option(input)?,
            type_key: input.string()?,
            layout_hash: input.id128()?,
            size: input.u64()?,
            align: input.u32()?,
            slot: input.u32()?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for InstanceRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        encode_option(out, &self.program_type)?;
        out.string(&self.program_type_key)?;
        out.string(&self.configuration)?;
        out.string(&self.resource)?;
        out.string(&self.instance_path)?;
        encode_option(out, &self.task_key)?;
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for InstanceRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            program_type: decode_option(input)?,
            program_type_key: input.string()?,
            configuration: input.string()?,
            resource: input.string()?,
            instance_path: input.string()?,
            task_key: decode_option(input)?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for TaskRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        out.string(&self.configuration)?;
        out.string(&self.resource)?;
        out.string(&self.name)?;
        out.bool(self.single);
        encode_option(out, &self.interval_ns)?;
        encode_option(out, &self.priority)?;
        encode_option(out, &self.guard_got_id)?;
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for TaskRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            configuration: input.string()?,
            resource: input.string()?,
            name: input.string()?,
            single: input.bool()?,
            interval_ns: decode_option(input)?,
            priority: decode_option(input)?,
            guard_got_id: decode_option(input)?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for ExportRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        out.string(&self.c_name)?;
        out.id128(self.function_got_id);
        out.string(&self.signature_key)?;
        out.bool(self.no_mangle);
        encode_option(out, &self.source)?;
        Ok(())
    }
}

impl Decode for ExportRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            c_name: input.string()?,
            function_got_id: input.id128()?,
            signature_key: input.string()?,
            no_mangle: input.bool()?,
            source: decode_option(input)?,
        })
    }
}

impl Encode for SlotRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.u32(self.slot);
        encode_enum(out, self.kind);
        out.u8(self.service);
        out.id128(self.got_id);
        Ok(())
    }
}

impl Decode for SlotRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            slot: input.u32()?,
            kind: decode_enum(input, "slot kind")?,
            service: input.u8()?,
            got_id: input.id128()?,
        })
    }
}

impl Encode for ActionRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.u8(self.kind);
        out.string(&self.key)?;
        encode_option(out, &self.old_key)?;
        encode_enum(out, self.action);
        out.string(&self.reason_code)?;
        out.string(&self.detail)?;
        Ok(())
    }
}

impl Decode for ActionRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            kind: input.u8()?,
            key: input.string()?,
            old_key: decode_option(input)?,
            action: decode_enum(input, "plan action")?,
            reason_code: input.string()?,
            detail: input.string()?,
        })
    }
}

impl Encode for PlanDiagnostic {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.string(&self.code)?;
        out.u8(self.severity);
        out.string(&self.message)?;
        encode_option(out, &self.got_id)?;
        encode_option(out, &self.old_key)?;
        encode_option(out, &self.new_key)?;
        Ok(())
    }
}

impl Decode for PlanDiagnostic {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            code: input.string()?,
            severity: input.u8()?,
            message: input.string()?,
            got_id: decode_option(input)?,
            old_key: decode_option(input)?,
            new_key: decode_option(input)?,
        })
    }
}

impl Encode for Plan {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        encode_enum(out, self.verdict);
        encode_option(out, &self.parent_build_id)?;
        encode_option(out, &self.parent_manifest_digest)?;
        out.list_header(self.actions.len())?;
        for action in &self.actions {
            action.encode(out)?;
        }
        out.list_header(self.diagnostics.len())?;
        for diagnostic in &self.diagnostics {
            diagnostic.encode(out)?;
        }
        Ok(())
    }
}

impl Decode for Plan {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        let verdict = decode_enum(input, "plan verdict")?;
        let parent_build_id = decode_option(input)?;
        let parent_manifest_digest = decode_option(input)?;
        let action_count = input.count(1)?;
        let mut actions = Vec::new();
        for _ in 0..action_count {
            actions.push(ActionRecord::decode(input)?);
        }
        let diagnostic_count = input.count(1)?;
        let mut diagnostics = Vec::new();
        for _ in 0..diagnostic_count {
            diagnostics.push(PlanDiagnostic::decode(input)?);
        }
        Ok(Self {
            verdict,
            parent_build_id,
            parent_manifest_digest,
            actions,
            diagnostics,
        })
    }
}

impl Encode for RetiredRecord {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.id128(self.got_id);
        out.u8(self.kind);
        out.string(&self.key)?;
        encode_string_list(out, &self.previous_keys)?;
        out.id128(self.retired_build_id);
        out.digest32(self.retired_manifest_digest);
        encode_enum(out, self.cause);
        Ok(())
    }
}

impl Decode for RetiredRecord {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            got_id: input.id128()?,
            kind: input.u8()?,
            key: input.string()?,
            previous_keys: decode_string_list(input)?,
            retired_build_id: input.id128()?,
            retired_manifest_digest: input.digest32()?,
            cause: decode_enum(input, "retired cause")?,
        })
    }
}

impl Encode for History {
    fn encode(&self, out: &mut Encoder) -> Result<()> {
        out.list_header(self.retired.len())?;
        for record in &self.retired {
            record.encode(out)?;
        }
        Ok(())
    }
}

impl Decode for History {
    fn decode(input: &mut Decoder<'_>) -> Result<Self> {
        let count = input.count(1)?;
        let mut retired = Vec::new();
        for _ in 0..count {
            retired.push(RetiredRecord::decode(input)?);
        }
        Ok(Self { retired })
    }
}

/// Encodes a list of records.
///
/// # Errors
///
/// Rejects unrepresentable counts or lengths.
pub fn encode_records<T: Encode>(values: &[T]) -> Result<Vec<u8>> {
    let mut out = Encoder::new();
    out.list_header(values.len())?;
    for value in values {
        value.encode(&mut out)?;
    }
    Ok(out.into_bytes())
}

/// Decodes a list of records and requires the payload to be fully consumed.
///
/// # Errors
///
/// Rejects malformed or truncated payloads and trailing bytes.
pub fn decode_records<T: Decode>(payload: &[u8]) -> Result<Vec<T>> {
    let mut input = Decoder::new(payload);
    let count = input.count(1)?;
    let mut values = Vec::new();
    for _ in 0..count {
        values.push(T::decode(&mut input)?);
    }
    input.finish()?;
    Ok(values)
}

/// Compares two plan diagnostics in the canonical table order.
#[must_use]
pub fn compare_plan_diagnostics(
    left: &PlanDiagnostic,
    right: &PlanDiagnostic,
) -> std::cmp::Ordering {
    (
        left.code.as_str(),
        left.got_id.is_some(),
        left.got_id.unwrap_or(GotId::ZERO),
        left.old_key.as_deref().unwrap_or(""),
        left.new_key.as_deref().unwrap_or(""),
        left.message.as_str(),
    )
        .cmp(&(
            right.code.as_str(),
            right.got_id.is_some(),
            right.got_id.unwrap_or(GotId::ZERO),
            right.old_key.as_deref().unwrap_or(""),
            right.new_key.as_deref().unwrap_or(""),
            right.message.as_str(),
        ))
}
