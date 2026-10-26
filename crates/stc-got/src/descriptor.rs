//! Module descriptor format 1 (GOT Artifacts §7).
//!
//! The descriptor is the minimal, pointer-free binding blob a loadable module
//! exposes to the host. It carries the build identity, the paired manifest
//! digest, the function-cell bindings, and the canonical GOT slot table. The
//! host validates it and the paired manifest together before attach; the
//! pairing rules are implemented by [`Descriptor::validate_against_manifest`].

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result, codes};
use crate::keys::{IdentityKind, validate_key};
use crate::layout::Signature;
use crate::manifest::{MANIFEST_FORMAT, Manifest};
use crate::model::{SlotKind, SlotRecord};
use crate::sha256::domain_hash;
use crate::wire::{Decoder, Encoder};

/// Descriptor magic.
pub const DESCRIPTOR_MAGIC: [u8; 8] = *b"STCMOD\0\0";
/// Descriptor format number.
pub const DESCRIPTOR_FORMAT: u16 = 1;
/// Module container format number.
pub const MODULE_FORMAT: u16 = 1;
/// Descriptor format 1 is defined for host ABI major version 2.
pub const HOST_ABI_MAJOR: u16 = 2;
/// Host ABI minor version provided by the runtime host.
///
/// Minor 1 (host ABI 2.1) records the inline TYPEINFO, VTABLE, and
/// object-record layout generated code reads directly. Minor 2 (host ABI 2.2)
/// appends the test-clock service (ID 12, `set_cycle_time_ns`); only test
/// builds require it, so production modules keep [`PRODUCTION_ABI_MINOR`].
pub const HOST_ABI_MINOR: u16 = 2;

/// Host ABI minor version required by production modules.
///
/// Production modules never import service 12 and therefore run on a 2.1 host.
pub const PRODUCTION_ABI_MINOR: u16 = 1;

/// Host ABI minor version required by test-build modules.
pub const TEST_ABI_MINOR: u16 = 2;
/// Descriptor digest domain string.
pub const DESCRIPTOR_DOMAIN: &str = "stc:descriptor:v1";
/// Byte offset of `descriptor_size` in the encoded header.
const SIZE_OFFSET: usize = 12;

/// `entry_flags` bits (GOT Artifacts §7.1).
pub mod entry_flag {
    /// `stc_module_prepare` initializes roots.
    pub const PREPARE_INITIALIZES: u32 = 1 << 0;
    /// `stc_module_cycle` is callable.
    pub const CYCLE_CALLABLE: u32 = 1 << 1;
    /// `stc_module_shutdown` performs retained-state work.
    pub const SHUTDOWN_RETAINED: u32 = 1 << 2;
    /// `stc_module_detach` releases module-local resources.
    pub const DETACH_RELEASES: u32 = 1 << 3;
    /// Debug build.
    pub const DEBUG_BUILD: u32 = 1 << 4;
    /// Retained-state host services are used.
    pub const RETAINED_SERVICES: u32 = 1 << 5;
    /// Debug checkpoint hooks are active.
    pub const DEBUG_HOOKS: u32 = 1 << 6;
    /// Bits defined by format 1.
    pub const MASK: u32 = 0x7F;
}

/// `required_features` bits (GOT Artifacts §7.1).
pub mod feature {
    /// Host ABI 2 base services.
    pub const HOST_ABI: u64 = 1 << 0;
    /// The paired manifest carries an online-change plan.
    pub const ONLINE_PLAN: u64 = 1 << 1;
    /// OOP descriptors/vtables are in use (M5).
    pub const DESCRIPTORS: u64 = 1 << 2;
    /// Place-descriptor references are in use (M5).
    pub const PLACES: u64 = 1 << 3;
    /// Debug checkpoint hooks are in use (M6.5).
    pub const DEBUG_HOOKS: u64 = 1 << 4;
    /// Bits defined by format 1.
    pub const MASK: u64 = 0b1_1111;
}

/// `FunctionRecord` flags (GOT Artifacts §7.1).
pub mod function_flag {
    /// User function.
    pub const USER: u32 = 1 << 0;
    /// Program body.
    pub const PROGRAM_BODY: u32 = 1 << 1;
    /// FB body.
    pub const FB_BODY: u32 = 1 << 2;
    /// Method.
    pub const METHOD: u32 = 1 << 3;
    /// Export wrapper.
    pub const EXPORT_WRAPPER: u32 = 1 << 4;
    /// Standard function called indirectly.
    pub const STD_FUNCTION: u32 = 1 << 5;
    /// Host may bind a function cell to this entry.
    pub const INDIRECT_CALLABLE: u32 = 1 << 6;
    /// Bits defined by format 1.
    pub const MASK: u32 = 0x7F;
}

/// One descriptor function record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorFunction {
    /// Function identity.
    pub got_id: GotId,
    /// Stable function key.
    pub key: String,
    /// Signature key.
    pub signature_key: String,
    /// Descriptive flags.
    pub flags: u32,
}

/// A module descriptor (format 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// Feature bits the host must implement.
    pub required_features: u64,
    /// Host ABI major required by the module.
    pub host_abi_major: u16,
    /// Host ABI minor required by the module.
    pub host_abi_minor: u16,
    /// Module container format.
    pub module_format: u16,
    /// Target byte order.
    pub endianness: u8,
    /// Pointer width in bytes.
    pub pointer_width: u8,
    /// Exact profile identifier.
    pub profile: String,
    /// Compiler package version.
    pub compiler_version: String,
    /// Resolved target triple.
    pub target_triple: String,
    /// Content-derived build identity.
    pub build_id: GotId,
    /// Digest of the paired manifest.
    pub manifest_digest: Digest32,
    /// Manifest format number.
    pub manifest_format: u16,
    /// Entry-point behavior flags.
    pub entry_flags: u32,
    /// Function records sorted by `GotId`.
    pub functions: Vec<DescriptorFunction>,
    /// Canonical slot table.
    pub slots: Vec<SlotRecord>,
}

impl Descriptor {
    /// Creates an unsealed descriptor with a zero manifest digest.
    #[must_use]
    pub fn new(build_id: GotId) -> Self {
        Self {
            required_features: feature::HOST_ABI | feature::ONLINE_PLAN,
            host_abi_major: HOST_ABI_MAJOR,
            host_abi_minor: PRODUCTION_ABI_MINOR,
            module_format: MODULE_FORMAT,
            endianness: 0,
            pointer_width: 8,
            profile: String::new(),
            compiler_version: String::new(),
            target_triple: String::new(),
            build_id,
            manifest_digest: Digest32::ZERO,
            manifest_format: MANIFEST_FORMAT,
            entry_flags: 0,
            functions: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Returns the number of function records.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// Returns the number of slot records.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Encodes the descriptor blob.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable counts or string lengths.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        self.encode_with_digest(self.manifest_digest)
    }

    /// Encodes the descriptor with `manifest_digest` replaced by `digest`.
    ///
    /// The descriptor digest is computed over the zero-digest projection, so
    /// this is the primitive behind [`Self::digest`].
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable counts or string lengths.
    pub fn encode_with_digest(&self, digest: Digest32) -> Result<Vec<u8>> {
        let mut out = Encoder::new();
        out.raw(&DESCRIPTOR_MAGIC);
        out.u16(DESCRIPTOR_FORMAT);
        out.u16(0);
        out.u32(0);
        out.u64(self.required_features);
        out.u16(self.host_abi_major);
        out.u16(self.host_abi_minor);
        out.u16(self.module_format);
        out.u8(self.endianness);
        out.u8(self.pointer_width);
        out.string(&self.profile)?;
        out.string(&self.compiler_version)?;
        out.string(&self.target_triple)?;
        out.id128(self.build_id);
        out.digest32(digest);
        out.u16(self.manifest_format);
        out.u32(self.entry_flags);
        out.u32(
            u32::try_from(self.functions.len())
                .map_err(|_| Error::new(codes::MODULE_INVALID, "too many descriptor functions"))?,
        );
        out.u32(
            u32::try_from(self.slots.len())
                .map_err(|_| Error::new(codes::MODULE_INVALID, "too many descriptor slots"))?,
        );
        for function in &self.functions {
            out.id128(function.got_id);
            out.string(&function.key)?;
            out.string(&function.signature_key)?;
            out.u32(function.flags);
        }
        for slot in &self.slots {
            out.u32(slot.slot);
            out.u8(slot.kind.tag());
            out.u8(slot.service);
            out.id128(slot.got_id);
        }
        let mut bytes = out.into_bytes();
        let size = u32::try_from(bytes.len())
            .map_err(|_| Error::new(codes::MODULE_INVALID, "descriptor exceeds the size limit"))?;
        bytes[SIZE_OFFSET..SIZE_OFFSET + 4].copy_from_slice(&size.to_le_bytes());
        Ok(bytes)
    }

    /// Computes the descriptor digest with `manifest_digest` zeroed.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable counts or string lengths.
    pub fn digest(&self) -> Result<Digest32> {
        let bytes = self.encode_with_digest(Digest32::ZERO)?;
        Ok(Digest32::from_bytes(domain_hash(
            DESCRIPTOR_DOMAIN,
            &[&bytes],
        )))
    }

    /// Decodes and validates a descriptor blob.
    ///
    /// # Errors
    ///
    /// Rejects bad magic/format/flags, truncation, trailing bytes, a bad size,
    /// non-canonical tables, malformed keys, and reserved flag bits.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut input = Decoder::new(bytes);
        if input.take(8)? != DESCRIPTOR_MAGIC {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor magic is `STCMOD\\0\\0`",
            ));
        }
        let format = input.u16()?;
        if format != DESCRIPTOR_FORMAT {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!("unsupported descriptor format {format}"),
            ));
        }
        let flags = input.u16()?;
        if flags != 0 {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor flags must be zero in format 1",
            ));
        }
        let declared_size = input.u32()?;
        let size = usize::try_from(declared_size).map_err(|_| {
            Error::new(
                codes::MODULE_INVALID,
                "descriptor size exceeds the platform limit",
            )
        })?;
        if size != bytes.len() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor_size {declared_size} does not match the {} byte blob",
                    bytes.len()
                ),
            ));
        }
        let required_features = input.u64()?;
        let host_abi_major = input.u16()?;
        let host_abi_minor = input.u16()?;
        let module_format = input.u16()?;
        let endianness = input.u8()?;
        let pointer_width = input.u8()?;
        let profile = input.string()?;
        let compiler_version = input.string()?;
        let target_triple = input.string()?;
        let build_id = input.id128()?;
        let manifest_digest = input.digest32()?;
        let manifest_format = input.u16()?;
        let entry_flags = input.u32()?;
        let function_count = input.count(1)?;
        let slot_count = input.count(1)?;
        let mut functions = Vec::new();
        for _ in 0..function_count {
            functions.push(DescriptorFunction {
                got_id: input.id128()?,
                key: input.string()?,
                signature_key: input.string()?,
                flags: input.u32()?,
            });
        }
        let mut slots = Vec::new();
        for _ in 0..slot_count {
            slots.push(SlotRecord {
                slot: input.u32()?,
                kind: SlotKind::from_tag(input.tag()?).ok_or_else(|| {
                    Error::new(codes::MODULE_INVALID, "invalid slot kind in descriptor")
                })?,
                service: input.u8()?,
                got_id: input.id128()?,
            });
        }
        input.finish()?;
        let descriptor = Self {
            required_features,
            host_abi_major,
            host_abi_minor,
            module_format,
            endianness,
            pointer_width,
            profile,
            compiler_version,
            target_triple,
            build_id,
            manifest_digest,
            manifest_format,
            entry_flags,
            functions,
            slots,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Validates the descriptor's format-level invariants.
    ///
    /// # Errors
    ///
    /// Reports the first violation of GOT Artifacts §7.1.
    pub fn validate(&self) -> Result<()> {
        if self.required_features & !feature::MASK != 0 {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor requires unknown feature bits {:#x}",
                    self.required_features & !feature::MASK
                ),
            ));
        }
        if self.host_abi_major != HOST_ABI_MAJOR {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor host ABI major {} is not {HOST_ABI_MAJOR}",
                    self.host_abi_major
                ),
            ));
        }
        if self.module_format != MODULE_FORMAT {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor module format {} is not {MODULE_FORMAT}",
                    self.module_format
                ),
            ));
        }
        if self.endianness > 1 {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor endianness must be 0 or 1",
            ));
        }
        if self.pointer_width != 8 {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor pointer width must be 8 for format 1",
            ));
        }
        if self.manifest_format != MANIFEST_FORMAT {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor manifest format {} is not {MANIFEST_FORMAT}",
                    self.manifest_format
                ),
            ));
        }
        if self.entry_flags & !entry_flag::MASK != 0 {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "descriptor entry_flags has reserved bits {:#x}",
                    self.entry_flags & !entry_flag::MASK
                ),
            ));
        }
        if self.build_id.is_zero() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor build_id is zero",
            ));
        }
        if self.manifest_digest.is_zero() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor manifest_digest is zero",
            ));
        }
        self.validate_functions()?;
        self.validate_slots()
    }

    fn validate_functions(&self) -> Result<()> {
        let mut previous = None;
        for function in &self.functions {
            if function.got_id.is_zero() {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    "descriptor function has a zero GotId",
                ));
            }
            if previous.is_some_and(|previous| previous >= function.got_id) {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    "descriptor functions are not sorted by GotId or contain duplicates",
                ));
            }
            previous = Some(function.got_id);
            if function.flags & !function_flag::MASK != 0 {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!(
                        "descriptor function `{}` has reserved flag bits {:#x}",
                        function.key,
                        function.flags & !function_flag::MASK
                    ),
                ));
            }
            validate_key(IdentityKind::Function, &function.key)?;
            let signature = Signature::parse(&function.signature_key)?;
            let key_signature = function
                .key
                .rsplit_once('@')
                .map(|(_, signature)| signature)
                .ok_or_else(|| {
                    Error::new(
                        codes::MODULE_INVALID,
                        format!("function key `{}` has no signature", function.key),
                    )
                })?;
            if Signature::parse(key_signature)? != signature {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!(
                        "descriptor function `{}` signature_key does not match its key",
                        function.key
                    ),
                ));
            }
        }
        Ok(())
    }

    fn validate_slots(&self) -> Result<()> {
        let mut previous_kind = 0_u8;
        let mut previous_identity = None;
        let mut previous_service = 0_u8;
        let mut function_slots = std::collections::BTreeSet::new();
        for (index, slot) in self.slots.iter().enumerate() {
            if usize::try_from(slot.slot).ok() != Some(index) {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!(
                        "descriptor slot table is not contiguous at index {index} (slot {})",
                        slot.slot
                    ),
                ));
            }
            let tag = slot.kind.tag();
            if tag < previous_kind {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    "descriptor slot kinds are not grouped in canonical order",
                ));
            }
            if tag != previous_kind {
                previous_kind = tag;
                previous_identity = None;
                previous_service = 0;
            }
            match slot.kind {
                SlotKind::Import => {
                    if slot.service == 0 || slot.service > 12 {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            format!(
                                "import slot {} service ID {} is outside the ABI 2 service table",
                                slot.slot, slot.service
                            ),
                        ));
                    }
                    if !slot.got_id.is_zero() {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            format!("import slot {} has a nonzero GotId", slot.slot),
                        ));
                    }
                    if slot.service <= previous_service {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            "import slots are not sorted by service ID or contain duplicates",
                        ));
                    }
                    previous_service = slot.service;
                }
                _ => {
                    if slot.service != 0 {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            format!("non-import slot {} has a service ID", slot.slot),
                        ));
                    }
                    if slot.got_id.is_zero() {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            format!("slot {} has a zero GotId", slot.slot),
                        ));
                    }
                    if previous_identity.is_some_and(|previous| previous >= slot.got_id) {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            "descriptor slots are not sorted by GotId or contain duplicates",
                        ));
                    }
                    previous_identity = Some(slot.got_id);
                    if slot.kind == SlotKind::Function && !function_slots.insert(slot.got_id) {
                        return Err(Error::new(
                            codes::MODULE_INVALID,
                            "descriptor has duplicate function slots",
                        ));
                    }
                }
            }
        }
        for function in &self.functions {
            if !function_slots.remove(&function.got_id) {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!(
                        "descriptor function `{}` has no function slot",
                        function.key
                    ),
                ));
            }
        }
        if let Some(got_id) = function_slots.into_iter().next() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!("descriptor function slot {got_id} has no function record"),
            ));
        }
        Ok(())
    }

    /// Checks the descriptor against its paired manifest (GOT Artifacts
    /// §11.4).
    ///
    /// # Errors
    ///
    /// Rejects a digest, build identity, header, function-set, or slot-table
    /// mismatch.
    pub fn validate_against_manifest(&self, manifest: &Manifest) -> Result<()> {
        self.validate()?;
        manifest.validate()?;
        let manifest_digest = manifest.digest()?;
        if self.manifest_digest != manifest_digest {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                format!(
                    "descriptor manifest digest {} does not match the manifest digest {manifest_digest}",
                    self.manifest_digest
                ),
            ));
        }
        let descriptor_digest = self.digest()?;
        if manifest.header.descriptor_digest != descriptor_digest {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                format!(
                    "manifest descriptor digest {} does not match the descriptor digest {descriptor_digest}",
                    manifest.header.descriptor_digest
                ),
            ));
        }
        let header = &manifest.header;
        if self.build_id != header.build_id {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor and manifest build IDs differ",
            ));
        }
        if self.profile != header.profile {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor and manifest profiles differ",
            ));
        }
        if self.target_triple != header.target_triple {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor and manifest targets differ",
            ));
        }
        if self.endianness != header.endianness
            || self.pointer_width != header.pointer_width
            || self.module_format != header.module_format
            || self.host_abi_major != header.host_abi_major
            || self.host_abi_minor != header.host_abi_minor
        {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor and manifest target or ABI headers differ",
            ));
        }
        if self.required_features != header.required_features {
            return Err(Error::new(
                codes::MODULE_INVALID,
                "descriptor and manifest required_features differ",
            ));
        }
        if self.slots != manifest.slots {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "descriptor and manifest slot tables differ",
            ));
        }
        if self.functions.len() != manifest.functions.len() {
            return Err(Error::new(
                codes::MANIFEST_PAIRING,
                "descriptor and manifest function counts differ",
            ));
        }
        let mut manifest_functions = manifest.functions.iter().collect::<Vec<_>>();
        manifest_functions.sort_by_key(|function| function.got_id);
        for (ours, theirs) in self.functions.iter().zip(manifest_functions) {
            if ours.got_id != theirs.got_id
                || ours.key != theirs.key
                || ours.signature_key != theirs.signature_key
                || ours.flags != theirs.flags
            {
                return Err(Error::new(
                    codes::MANIFEST_PAIRING,
                    format!(
                        "descriptor and manifest function records differ for `{}`",
                        ours.key
                    ),
                ));
            }
        }
        let root_ids = manifest
            .roots
            .iter()
            .map(|root| root.got_id)
            .collect::<std::collections::BTreeSet<_>>();
        let descriptor_ids = manifest
            .descriptors
            .iter()
            .map(|descriptor| descriptor.got_id)
            .collect::<std::collections::BTreeSet<_>>();
        let function_ids = manifest
            .functions
            .iter()
            .map(|function| function.got_id)
            .collect::<std::collections::BTreeSet<_>>();
        for slot in &self.slots {
            let found = match slot.kind {
                SlotKind::Data | SlotKind::Direct => root_ids.contains(&slot.got_id),
                SlotKind::Function => function_ids.contains(&slot.got_id),
                SlotKind::Descriptor => descriptor_ids.contains(&slot.got_id),
                SlotKind::Import => true,
            };
            if !found {
                return Err(Error::new(
                    codes::MANIFEST_PAIRING,
                    format!(
                        "descriptor slot {} ({:?}) does not resolve to a manifest record",
                        slot.slot, slot.kind
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_abi_minor_records_the_inline_oop_layout_and_test_clock() {
        assert_eq!(HOST_ABI_MAJOR, 2);
        assert_eq!(PRODUCTION_ABI_MINOR, 1);
        assert_eq!(TEST_ABI_MINOR, 2);
        assert_eq!(HOST_ABI_MINOR, 2);
        let descriptor = Descriptor::new(GotId::from_bytes([0x11; 16]));
        assert_eq!(descriptor.host_abi_major, 2);
        assert_eq!(descriptor.host_abi_minor, PRODUCTION_ABI_MINOR);
    }
}
