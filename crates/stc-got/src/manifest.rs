//! The persistent GOT manifest: binary and canonical JSON projections,
//! digests, and structural validation (GOT Artifacts §9).

use std::collections::{BTreeMap, BTreeSet};

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result};
use crate::json::{self, Json};
use crate::keys::{IdentityKind, validate_key};
use crate::layout::{Signature, TypeKey};
use crate::model::{
    Decode, DescriptorRecord, Encode, ExportRecord, FunctionRecord, Header, History,
    InstanceRecord, Plan, PlanAction, PlanVerdict, RootKind, RootRecord, SlotKind, SlotRecord,
    TaskRecord, TypeRecord, compare_plan_diagnostics, encode_records,
};
use crate::sha256::domain_hash;
use crate::wire::{Decoder, Encoder};

/// The binary manifest magic.
pub const MANIFEST_MAGIC: [u8; 8] = *b"STCGOT\0\0";
/// The supported manifest format.
pub const MANIFEST_FORMAT: u16 = 1;
/// The manifest digest domain.
pub const MANIFEST_DOMAIN: &str = "stc:manifest:v1";
/// The catalog digest domain.
pub const CATALOG_DOMAIN: &str = "stc:catalog:v1";
/// The plan digest domain.
pub const PLAN_DOMAIN: &str = "stc:plan:v1";

/// Feature bit: host ABI 2.0 base services.
pub const FEATURE_HOST_ABI: u64 = 1 << 0;
/// Feature bit: an online-change plan is present.
pub const FEATURE_ONLINE_PLAN: u64 = 1 << 1;
/// Feature bits defined by manifest/module format 1.
pub const FEATURE_MASK: u64 = 0b1_1111;

/// Section tags.
pub mod section {
    /// Header.
    pub const HEADER: u16 = 1;
    /// Roots.
    pub const ROOTS: u16 = 2;
    /// Functions.
    pub const FUNCTIONS: u16 = 3;
    /// Types.
    pub const TYPES: u16 = 4;
    /// Descriptors.
    pub const DESCRIPTORS: u16 = 5;
    /// Instances.
    pub const INSTANCES: u16 = 6;
    /// Tasks.
    pub const TASKS: u16 = 7;
    /// Exports.
    pub const EXPORTS: u16 = 8;
    /// Slots.
    pub const SLOTS: u16 = 9;
    /// Online-change plan.
    pub const PLAN: u16 = 10;
    /// History.
    pub const HISTORY: u16 = 11;
}

/// The complete manifest body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Header section.
    pub header: Header,
    /// Root table.
    pub roots: Vec<RootRecord>,
    /// Function table.
    pub functions: Vec<FunctionRecord>,
    /// Type table.
    pub types: Vec<TypeRecord>,
    /// Descriptor table.
    pub descriptors: Vec<DescriptorRecord>,
    /// Instance table.
    pub instances: Vec<InstanceRecord>,
    /// Task table.
    pub tasks: Vec<TaskRecord>,
    /// Export table.
    pub exports: Vec<ExportRecord>,
    /// Slot table.
    pub slots: Vec<SlotRecord>,
    /// Online-change plan.
    pub plan: Plan,
    /// Build history.
    pub history: History,
}

impl Manifest {
    /// Encodes the body sections in ascending tag order.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn encode_body(&self) -> Result<Vec<u8>> {
        let sections = self.section_payloads()?;
        let mut out = Encoder::with_capacity(
            sections
                .iter()
                .map(|(_, payload)| payload.len() + 6)
                .sum::<usize>(),
        );
        for (tag, payload) in sections {
            out.u16(tag);
            out.u32(
                u32::try_from(payload.len()).map_err(|_| {
                    Error::manifest("manifest section exceeds the u32 length limit")
                })?,
            );
            out.raw(&payload);
        }
        Ok(out.into_bytes())
    }

    fn section_payloads(&self) -> Result<Vec<(u16, Vec<u8>)>> {
        let mut header = Encoder::new();
        self.header.encode(&mut header)?;
        let mut plan = Encoder::new();
        self.plan.encode(&mut plan)?;
        let mut history = Encoder::new();
        self.history.encode(&mut history)?;
        Ok(vec![
            (section::HEADER, header.into_bytes()),
            (section::ROOTS, encode_records(&self.roots)?),
            (section::FUNCTIONS, encode_records(&self.functions)?),
            (section::TYPES, encode_records(&self.types)?),
            (section::DESCRIPTORS, encode_records(&self.descriptors)?),
            (section::INSTANCES, encode_records(&self.instances)?),
            (section::TASKS, encode_records(&self.tasks)?),
            (section::EXPORTS, encode_records(&self.exports)?),
            (section::SLOTS, encode_records(&self.slots)?),
            (section::PLAN, plan.into_bytes()),
            (section::HISTORY, history.into_bytes()),
        ])
    }

    /// Computes the catalog digest.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn catalog_digest(&self) -> Result<Digest32> {
        let roots = encode_records(&self.roots)?;
        let functions = encode_records(&self.functions)?;
        let types = encode_records(&self.types)?;
        let descriptors = encode_records(&self.descriptors)?;
        let instances = encode_records(&self.instances)?;
        let tasks = encode_records(&self.tasks)?;
        let exports = encode_records(&self.exports)?;
        let slots = encode_records(&self.slots)?;
        Ok(Digest32::from_bytes(domain_hash(
            CATALOG_DOMAIN,
            &[
                &roots,
                &functions,
                &types,
                &descriptors,
                &instances,
                &tasks,
                &exports,
                &slots,
            ],
        )))
    }

    /// Computes the plan digest.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn plan_digest(&self) -> Result<Digest32> {
        let mut plan = Encoder::new();
        self.plan.encode(&mut plan)?;
        Ok(Digest32::from_bytes(domain_hash(
            PLAN_DOMAIN,
            &[plan.as_bytes()],
        )))
    }

    /// Computes the manifest digest.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn digest(&self) -> Result<Digest32> {
        let body = self.encode_body()?;
        Ok(Digest32::from_bytes(domain_hash(MANIFEST_DOMAIN, &[&body])))
    }

    /// Encodes the complete binary manifest.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn encode_binary(&self) -> Result<Vec<u8>> {
        let body = self.encode_body()?;
        let digest = Digest32::from_bytes(domain_hash(MANIFEST_DOMAIN, &[&body]));
        let mut out = Encoder::with_capacity(body.len() + 54);
        out.raw(&MANIFEST_MAGIC);
        out.u16(MANIFEST_FORMAT);
        out.u16(0);
        out.u64(
            u64::try_from(body.len())
                .map_err(|_| Error::manifest("manifest body exceeds the u64 length limit"))?,
        );
        out.digest32(digest);
        out.raw(&body);
        Ok(out.into_bytes())
    }

    /// Decodes and validates a binary manifest.
    ///
    /// # Errors
    ///
    /// Rejects bad magic/format/flags, truncation, trailing bytes, digest
    /// mismatches, unknown critical sections, and every structural violation.
    pub fn decode_binary(bytes: &[u8]) -> Result<Self> {
        let mut input = Decoder::new(bytes);
        let magic = input.take(8)?;
        if magic != MANIFEST_MAGIC {
            return Err(Error::manifest("manifest magic is `STCGOT\\0\\0`"));
        }
        let format = input.u16()?;
        if format != MANIFEST_FORMAT {
            return Err(Error::manifest(format!(
                "unsupported manifest format {format}"
            )));
        }
        let flags = input.u16()?;
        if flags != 0 {
            return Err(Error::manifest("manifest flags must be zero in format 1"));
        }
        let body_length = input.u64()?;
        let body_length = usize::try_from(body_length)
            .map_err(|_| Error::manifest("manifest body length exceeds the platform limit"))?;
        let expected = input.digest32()?;
        let body = input.take(body_length)?;
        input.finish()?;
        let actual = Digest32::from_bytes(domain_hash(MANIFEST_DOMAIN, &[body]));
        if actual != expected {
            return Err(Error::manifest(format!(
                "manifest digest mismatch: header {}, computed {}",
                expected, actual
            )));
        }

        let mut payloads: [Option<&[u8]>; 12] = [None; 12];
        let mut contents = Decoder::new(body);
        let mut last_tag = 0_u16;
        while !contents.is_empty() {
            let tag = contents.u16()?;
            let length = contents.u32()?;
            let payload = contents.take(length as usize)?;
            if tag <= last_tag {
                return Err(Error::manifest(
                    "manifest sections must appear in strictly ascending tag order",
                ));
            }
            last_tag = tag;
            if tag & 0x8000 != 0 {
                return Err(Error::manifest(format!(
                    "unknown critical manifest section {tag:#06x}"
                )));
            }
            if (1..=11).contains(&tag) {
                payloads[tag as usize] = Some(payload);
            }
        }
        for (tag, payload) in payloads.iter().enumerate().skip(1) {
            if payload.is_none() {
                return Err(Error::manifest(format!(
                    "manifest is missing required section {tag}"
                )));
            }
        }

        let header = decode_record::<Header>(payloads[section::HEADER as usize].unwrap())?;
        let manifest = Self {
            header,
            roots: decode_payload(payloads[section::ROOTS as usize].unwrap())?,
            functions: decode_payload(payloads[section::FUNCTIONS as usize].unwrap())?,
            types: decode_payload(payloads[section::TYPES as usize].unwrap())?,
            descriptors: decode_payload(payloads[section::DESCRIPTORS as usize].unwrap())?,
            instances: decode_payload(payloads[section::INSTANCES as usize].unwrap())?,
            tasks: decode_payload(payloads[section::TASKS as usize].unwrap())?,
            exports: decode_payload(payloads[section::EXPORTS as usize].unwrap())?,
            slots: decode_payload(payloads[section::SLOTS as usize].unwrap())?,
            plan: decode_record::<Plan>(payloads[section::PLAN as usize].unwrap())?,
            history: decode_record::<History>(payloads[section::HISTORY as usize].unwrap())?,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Projects the manifest to canonical JSON text.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn to_json_text(&self) -> Result<String> {
        Ok(self.to_json_value()?.render())
    }

    /// Projects the manifest to a JSON value.
    ///
    /// # Errors
    ///
    /// Rejects unrepresentable lengths.
    pub fn to_json_value(&self) -> Result<Json> {
        let body = Json::object([
            (
                "descriptors".into(),
                Json::Array(
                    self.descriptors
                        .iter()
                        .map(DescriptorRecord::to_json)
                        .collect(),
                ),
            ),
            (
                "exports".into(),
                Json::Array(self.exports.iter().map(ExportRecord::to_json).collect()),
            ),
            (
                "functions".into(),
                Json::Array(self.functions.iter().map(FunctionRecord::to_json).collect()),
            ),
            ("header".into(), self.header.to_json()),
            ("history".into(), self.history.to_json()),
            (
                "instances".into(),
                Json::Array(self.instances.iter().map(InstanceRecord::to_json).collect()),
            ),
            ("plan".into(), self.plan.to_json()),
            (
                "roots".into(),
                Json::Array(self.roots.iter().map(RootRecord::to_json).collect()),
            ),
            (
                "slots".into(),
                Json::Array(self.slots.iter().map(SlotRecord::to_json).collect()),
            ),
            (
                "tasks".into(),
                Json::Array(self.tasks.iter().map(TaskRecord::to_json).collect()),
            ),
            (
                "types".into(),
                Json::Array(self.types.iter().map(TypeRecord::to_json).collect()),
            ),
        ]);
        Ok(Json::object([
            ("body".into(), body),
            ("digest".into(), Json::string(self.digest()?.to_hex())),
            ("flags".into(), Json::Number(0)),
            ("format".into(), Json::Number(i128::from(MANIFEST_FORMAT))),
        ]))
    }

    /// Parses a canonical JSON manifest and validates its digests.
    ///
    /// # Errors
    ///
    /// Rejects malformed JSON, unknown fields, a digest mismatch, and every
    /// structural violation.
    pub fn decode_json(text: &str) -> Result<Self> {
        let value = json::parse(text)?;
        let root = value
            .as_object()
            .ok_or_else(|| Error::manifest("manifest JSON must be an object"))?;
        check_keys(root, &["body", "digest", "flags", "format"], "manifest")?;
        let format = number(root, "format", "manifest")?;
        if format != i128::from(MANIFEST_FORMAT) {
            return Err(Error::manifest(format!(
                "unsupported manifest JSON format {format}"
            )));
        }
        let flags = number(root, "flags", "manifest")?;
        if flags != 0 {
            return Err(Error::manifest("manifest flags must be zero in format 1"));
        }
        let digest_text = string(root, "digest", "manifest")?;
        let digest = Digest32::parse(&digest_text)?;
        let body = root
            .get("body")
            .and_then(Json::as_object)
            .ok_or_else(|| Error::manifest("manifest body must be a JSON object"))?;
        check_keys(
            body,
            &[
                "descriptors",
                "exports",
                "functions",
                "header",
                "history",
                "instances",
                "plan",
                "roots",
                "slots",
                "tasks",
                "types",
            ],
            "manifest body",
        )?;
        let header = Header::from_json(body.get("header").unwrap())?;
        let roots = array_records(body, "roots", RootRecord::from_json)?;
        let functions = array_records(body, "functions", FunctionRecord::from_json)?;
        let types = array_records(body, "types", TypeRecord::from_json)?;
        let descriptors = array_records(body, "descriptors", DescriptorRecord::from_json)?;
        let instances = array_records(body, "instances", InstanceRecord::from_json)?;
        let tasks = array_records(body, "tasks", TaskRecord::from_json)?;
        let exports = array_records(body, "exports", ExportRecord::from_json)?;
        let slots = array_records(body, "slots", SlotRecord::from_json)?;
        let plan = Plan::from_json(body.get("plan").unwrap())?;
        let history = History::from_json(body.get("history").unwrap())?;
        let manifest = Self {
            header,
            roots,
            functions,
            types,
            descriptors,
            instances,
            tasks,
            exports,
            slots,
            plan,
            history,
        };
        let computed = manifest.digest()?;
        if computed != digest {
            return Err(Error::manifest(format!(
                "JSON manifest digest mismatch: header {digest}, computed {computed}"
            )));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks the manifest against a build target and profile.
    ///
    /// # Errors
    ///
    /// Rejects a target, profile, module format, endianness, pointer width, or
    /// host ABI mismatch.
    pub fn validate_for(
        &self,
        profile: &str,
        target: &crate::layout::Target,
        host_abi_major: u16,
        host_abi_minor: u16,
    ) -> Result<()> {
        let header = &self.header;
        if header.profile != profile {
            return Err(Error::manifest(format!(
                "manifest profile `{}` does not match `{profile}`",
                header.profile
            )));
        }
        if header.target_triple != target.triple {
            return Err(Error::manifest(format!(
                "manifest target `{}` does not match `{}`",
                header.target_triple, target.triple
            )));
        }
        if header.endianness != target.endianness.tag() {
            return Err(Error::manifest(
                "manifest endianness does not match the target",
            ));
        }
        if header.pointer_width != target.pointer_width {
            return Err(Error::manifest(
                "manifest pointer width does not match the target",
            ));
        }
        if header.host_abi_major != host_abi_major {
            return Err(Error::manifest(format!(
                "manifest host ABI major {} does not match {host_abi_major}",
                header.host_abi_major
            )));
        }
        if header.host_abi_minor > host_abi_minor {
            return Err(Error::manifest(format!(
                "manifest requires host ABI minor {} but the host provides {host_abi_minor}",
                header.host_abi_minor
            )));
        }
        Ok(())
    }

    /// Validates the manifest body against format 1.
    ///
    /// # Errors
    ///
    /// Reports the first structural violation.
    pub fn validate(&self) -> Result<()> {
        self.validate_header()?;
        validate_table_order(
            &self.roots,
            |left, right| (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id)),
            "roots",
        )?;
        validate_table_order(
            &self.functions,
            |left, right| {
                (left.key.as_str(), left.signature_key.as_str(), left.got_id).cmp(&(
                    right.key.as_str(),
                    right.signature_key.as_str(),
                    right.got_id,
                ))
            },
            "functions",
        )?;
        validate_table_order(
            &self.types,
            |left, right| (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id)),
            "types",
        )?;
        validate_table_order(
            &self.descriptors,
            |left, right| (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id)),
            "descriptors",
        )?;
        validate_table_order(
            &self.instances,
            |left, right| (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id)),
            "instances",
        )?;
        validate_table_order(
            &self.tasks,
            |left, right| (left.key.as_str(), left.got_id).cmp(&(right.key.as_str(), right.got_id)),
            "tasks",
        )?;
        validate_table_order(
            &self.exports,
            |left, right| {
                (left.c_name.as_str(), left.got_id).cmp(&(right.c_name.as_str(), right.got_id))
            },
            "exports",
        )?;
        validate_table_order(
            &self.plan.actions,
            |left, right| left.got_id.cmp(&right.got_id),
            "plan actions",
        )?;
        validate_table_order(
            &self.plan.diagnostics,
            compare_plan_diagnostics,
            "plan diagnostics",
        )?;
        validate_table_order(
            &self.history.retired,
            |left, right| {
                (left.retired_build_id, left.got_id).cmp(&(right.retired_build_id, right.got_id))
            },
            "history",
        )?;

        let active = self.validate_records()?;
        self.validate_slots()?;
        self.validate_plan(&active)?;
        self.validate_history()?;

        let catalog = self.catalog_digest()?;
        if catalog != self.header.catalog_digest {
            return Err(Error::manifest(format!(
                "catalog digest mismatch: header {}, computed {}",
                self.header.catalog_digest, catalog
            )));
        }
        let plan = self.plan_digest()?;
        if plan != self.header.plan_digest {
            return Err(Error::manifest(format!(
                "plan digest mismatch: header {}, computed {}",
                self.header.plan_digest, plan
            )));
        }
        Ok(())
    }

    fn validate_header(&self) -> Result<()> {
        let header = &self.header;
        if header.profile.is_empty() {
            return Err(Error::manifest("header profile is empty"));
        }
        if header.target_triple.is_empty() {
            return Err(Error::manifest("header target triple is empty"));
        }
        if header.module_format != 1 {
            return Err(Error::manifest("header module_format must be 1"));
        }
        if header.endianness > 1 {
            return Err(Error::manifest("header endianness must be 0 or 1"));
        }
        if header.pointer_width != 8 {
            return Err(Error::manifest(
                "header pointer_width must be 8 for format 1",
            ));
        }
        if header.generator_flags & !0x7F != 0 {
            return Err(Error::manifest(
                "header generator_flags has reserved bits set",
            ));
        }
        if header.required_features & !FEATURE_MASK != 0 {
            return Err(Error::manifest(format!(
                "manifest requires unsupported feature bits {:#x}",
                header.required_features & !FEATURE_MASK
            )));
        }
        if header.build_id.is_zero() {
            return Err(Error::manifest("header build_id is zero"));
        }
        match (header.parent_build_id, header.parent_manifest_digest) {
            (None, None) => {}
            (Some(build), Some(digest)) => {
                if build.is_zero() {
                    return Err(Error::manifest("header parent_build_id is zero"));
                }
                if digest.is_zero() {
                    return Err(Error::manifest("header parent_manifest_digest is zero"));
                }
            }
            _ => {
                return Err(Error::manifest(
                    "header parent_build_id and parent_manifest_digest must both be present or absent",
                ));
            }
        }
        if header.descriptor_digest.is_zero() {
            return Err(Error::manifest("header descriptor_digest is zero"));
        }
        if header.source_fingerprint.is_zero() {
            return Err(Error::manifest("header source_fingerprint is zero"));
        }
        if header.catalog_digest.is_zero() {
            return Err(Error::manifest("header catalog_digest is zero"));
        }
        if header.plan_digest.is_zero() {
            return Err(Error::manifest("header plan_digest is zero"));
        }
        Ok(())
    }

    /// Validates records and returns the active identity map.
    fn validate_records(&self) -> Result<BTreeMap<GotId, (u8, String)>> {
        let mut active = BTreeMap::new();
        let mut keys: BTreeMap<IdentityKind, BTreeSet<&str>> = BTreeMap::new();

        for root in &self.roots {
            let kind = if root.root_kind == RootKind::Direct {
                IdentityKind::Direct
            } else {
                IdentityKind::DataRoot
            };
            validate_key(kind, &root.key)?;
            validate_previous_keys(kind, &root.key, &root.previous_keys)?;
            insert_key(&mut keys, kind, &root.key)?;
            insert_identity(&mut active, root.got_id, kind, &root.key)?;
            if root.layout_hash.is_zero() {
                return Err(Error::manifest(format!(
                    "root `{}` has a zero layout hash",
                    root.key
                )));
            }
            let type_key = TypeKey::parse(&root.type_key).map_err(|error| {
                Error::manifest(format!("root `{}` type key: {}", root.key, error.message))
            })?;
            if type_key.contains_pointer() {
                return Err(Error::manifest(format!(
                    "persistent root `{}` has a PTR_TO type",
                    root.key
                )));
            }
            if root.size == 0 {
                return Err(Error::manifest(format!(
                    "root `{}` has zero size",
                    root.key
                )));
            }
            validate_align(root.align, "root", &root.key)?;
            validate_field_path(&root.field_path, &root.key)?;
            if let Some(owner) = root.owner_type {
                require_type(&self.types, owner, "root owner")?;
            }
        }

        for function in &self.functions {
            validate_key(IdentityKind::Function, &function.key)?;
            validate_previous_keys(
                IdentityKind::Function,
                &function.key,
                &function.previous_keys,
            )?;
            insert_key(&mut keys, IdentityKind::Function, &function.key)?;
            insert_identity(
                &mut active,
                function.got_id,
                IdentityKind::Function,
                &function.key,
            )?;
            let signature = Signature::parse(&function.signature_key).map_err(|error| {
                Error::manifest(format!(
                    "function `{}` signature: {}",
                    function.key, error.message
                ))
            })?;
            if !key_signature_matches(&function.key, &signature.to_string())? {
                return Err(Error::manifest(format!(
                    "function `{}` signature_key does not match its key",
                    function.key
                )));
            }
            if function.flags & !0x7F != 0 {
                return Err(Error::manifest(format!(
                    "function `{}` has reserved flag bits set",
                    function.key
                )));
            }
            if let Some(owner) = function.owner_type {
                require_type(&self.types, owner, "function owner")?;
            }
        }

        for ty in &self.types {
            validate_key(IdentityKind::Type, &ty.key)?;
            validate_previous_keys(IdentityKind::Type, &ty.key, &ty.previous_keys)?;
            insert_key(&mut keys, IdentityKind::Type, &ty.key)?;
            insert_identity(&mut active, ty.got_id, IdentityKind::Type, &ty.key)?;
            if ty.layout_hash.is_zero() {
                return Err(Error::manifest(format!(
                    "type `{}` has a zero layout hash",
                    ty.key
                )));
            }
            TypeKey::parse(&ty.type_key).map_err(|error| {
                Error::manifest(format!("type `{}` type key: {}", ty.key, error.message))
            })?;
            if ty.size == 0 {
                return Err(Error::manifest(format!("type `{}` has zero size", ty.key)));
            }
            validate_align(ty.align, "type", &ty.key)?;
            validate_fields(&self.types, ty)?;
        }

        for descriptor in &self.descriptors {
            validate_key(IdentityKind::Descriptor, &descriptor.key)?;
            validate_previous_keys(
                IdentityKind::Descriptor,
                &descriptor.key,
                &descriptor.previous_keys,
            )?;
            insert_key(&mut keys, IdentityKind::Descriptor, &descriptor.key)?;
            insert_identity(
                &mut active,
                descriptor.got_id,
                IdentityKind::Descriptor,
                &descriptor.key,
            )?;
            if descriptor.layout_hash.is_zero() {
                return Err(Error::manifest(format!(
                    "descriptor `{}` has a zero layout hash",
                    descriptor.key
                )));
            }
            TypeKey::parse(&descriptor.type_key).map_err(|error| {
                Error::manifest(format!(
                    "descriptor `{}` type key: {}",
                    descriptor.key, error.message
                ))
            })?;
            if descriptor.size == 0 {
                return Err(Error::manifest(format!(
                    "descriptor `{}` has zero size",
                    descriptor.key
                )));
            }
            validate_align(descriptor.align, "descriptor", &descriptor.key)?;
            if let Some(owner) = descriptor.owner_type {
                require_type(&self.types, owner, "descriptor owner")?;
            }
        }

        for instance in &self.instances {
            validate_key(IdentityKind::Instance, &instance.key)?;
            validate_previous_keys(
                IdentityKind::Instance,
                &instance.key,
                &instance.previous_keys,
            )?;
            insert_key(&mut keys, IdentityKind::Instance, &instance.key)?;
            insert_identity(
                &mut active,
                instance.got_id,
                IdentityKind::Instance,
                &instance.key,
            )?;
            if let Some(program) = instance.program_type {
                require_type(&self.types, program, "instance program type")?;
            }
            if instance.program_type_key.is_empty() {
                return Err(Error::manifest(format!(
                    "instance `{}` has an empty program_type_key",
                    instance.key
                )));
            }
            if instance.instance_path.is_empty() {
                return Err(Error::manifest(format!(
                    "instance `{}` has an empty instance_path",
                    instance.key
                )));
            }
            if let Some(task) = &instance.task_key
                && !self.tasks.iter().any(|record| &record.key == task)
            {
                return Err(Error::manifest(format!(
                    "instance `{}` references a missing task `{task}`",
                    instance.key
                )));
            }
        }

        for task in &self.tasks {
            validate_key(IdentityKind::Task, &task.key)?;
            validate_previous_keys(IdentityKind::Task, &task.key, &task.previous_keys)?;
            insert_key(&mut keys, IdentityKind::Task, &task.key)?;
            insert_identity(&mut active, task.got_id, IdentityKind::Task, &task.key)?;
            if let Some(guard) = task.guard_got_id {
                let Some(root) = self.roots.iter().find(|root| root.got_id == guard) else {
                    return Err(Error::manifest(format!(
                        "task `{}` references a missing guard root",
                        task.key
                    )));
                };
                if root.root_kind != RootKind::TaskGuard {
                    return Err(Error::manifest(format!(
                        "task `{}` guard is not a task-guard root",
                        task.key
                    )));
                }
            }
        }

        for export in &self.exports {
            validate_key(IdentityKind::Export, &export.key)?;
            validate_previous_keys(IdentityKind::Export, &export.key, &export.previous_keys)?;
            insert_key(&mut keys, IdentityKind::Export, &export.key)?;
            insert_identity(
                &mut active,
                export.got_id,
                IdentityKind::Export,
                &export.key,
            )?;
            let signature = Signature::parse(&export.signature_key).map_err(|error| {
                Error::manifest(format!(
                    "export `{}` signature: {}",
                    export.key, error.message
                ))
            })?;
            if !key_signature_matches(&export.key, &signature.to_string())? {
                return Err(Error::manifest(format!(
                    "export `{}` signature_key does not match its key",
                    export.key
                )));
            }
            if !self
                .functions
                .iter()
                .any(|function| function.got_id == export.function_got_id)
            {
                return Err(Error::manifest(format!(
                    "export `{}` references a missing function",
                    export.key
                )));
            }
        }

        Ok(active)
    }

    fn validate_slots(&self) -> Result<()> {
        let mut last_kind = SlotKind::Data;
        let mut seen_data = BTreeSet::new();
        let mut seen_functions = BTreeSet::new();
        let mut seen_descriptors = BTreeSet::new();
        let mut seen_direct = BTreeSet::new();
        let mut seen_services = BTreeSet::new();
        let mut last_service = 0_u8;
        let mut last_got_id = GotId::ZERO;
        let mut first_in_kind = true;
        for (index, slot) in self.slots.iter().enumerate() {
            let expected = u32::try_from(index)
                .map_err(|_| Error::manifest("slot table exceeds the u32 limit"))?;
            if slot.slot != expected {
                return Err(Error::manifest(format!(
                    "slot table must be contiguous from zero (expected {expected}, found {})",
                    slot.slot
                )));
            }
            if slot.kind != last_kind {
                if slot.kind.tag() < last_kind.tag() {
                    return Err(Error::manifest(
                        "slot groups must appear in data, function, descriptor, direct, import order",
                    ));
                }
                last_kind = slot.kind;
                first_in_kind = true;
            }
            match slot.kind {
                SlotKind::Import => {
                    if slot.service == 0 || slot.service > 11 {
                        return Err(Error::manifest(format!(
                            "import slot service ID {} is outside ABI 2.0",
                            slot.service
                        )));
                    }
                    if !slot.got_id.is_zero() {
                        return Err(Error::manifest("import slot must have a zero GotId"));
                    }
                    if !seen_services.insert(slot.service) {
                        return Err(Error::manifest(format!(
                            "duplicate import service ID {}",
                            slot.service
                        )));
                    }
                    if !first_in_kind && slot.service <= last_service {
                        return Err(Error::manifest("import slots must be sorted by service ID"));
                    }
                    last_service = slot.service;
                }
                SlotKind::Data | SlotKind::Function | SlotKind::Descriptor | SlotKind::Direct => {
                    if slot.service != 0 {
                        return Err(Error::manifest(
                            "non-import slots must have service ID zero",
                        ));
                    }
                    if slot.got_id.is_zero() {
                        return Err(Error::manifest(
                            "non-import slots must have a nonzero GotId",
                        ));
                    }
                    if !first_in_kind && slot.got_id <= last_got_id {
                        return Err(Error::manifest(
                            "slots within a group must be strictly sorted by GotId",
                        ));
                    }
                    match slot.kind {
                        SlotKind::Data => {
                            let Some(root) =
                                self.roots.iter().find(|root| root.got_id == slot.got_id)
                            else {
                                return Err(Error::manifest(format!(
                                    "data slot {} references a missing root",
                                    slot.slot
                                )));
                            };
                            if root.root_kind == RootKind::Direct {
                                return Err(Error::manifest(
                                    "a direct root must use a direct slot",
                                ));
                            }
                            seen_data.insert(slot.got_id);
                        }
                        SlotKind::Direct => {
                            let Some(root) =
                                self.roots.iter().find(|root| root.got_id == slot.got_id)
                            else {
                                return Err(Error::manifest(format!(
                                    "direct slot {} references a missing root",
                                    slot.slot
                                )));
                            };
                            if root.root_kind != RootKind::Direct {
                                return Err(Error::manifest(
                                    "a non-direct root must use a data slot",
                                ));
                            }
                            seen_direct.insert(slot.got_id);
                        }
                        SlotKind::Function => {
                            if !self
                                .functions
                                .iter()
                                .any(|function| function.got_id == slot.got_id)
                            {
                                return Err(Error::manifest(format!(
                                    "function slot {} references a missing function",
                                    slot.slot
                                )));
                            }
                            seen_functions.insert(slot.got_id);
                        }
                        SlotKind::Descriptor => {
                            if !self
                                .descriptors
                                .iter()
                                .any(|descriptor| descriptor.got_id == slot.got_id)
                            {
                                return Err(Error::manifest(format!(
                                    "descriptor slot {} references a missing descriptor",
                                    slot.slot
                                )));
                            }
                            seen_descriptors.insert(slot.got_id);
                        }
                        SlotKind::Import => {}
                    }
                    last_got_id = slot.got_id;
                }
            }
            first_in_kind = false;
        }
        for root in &self.roots {
            let target = if root.root_kind == RootKind::Direct {
                &seen_direct
            } else {
                &seen_data
            };
            if !target.contains(&root.got_id) {
                return Err(Error::manifest(format!(
                    "root `{}` has no GOT slot",
                    root.key
                )));
            }
        }
        for function in &self.functions {
            if !seen_functions.contains(&function.got_id) {
                return Err(Error::manifest(format!(
                    "function `{}` has no GOT slot",
                    function.key
                )));
            }
        }
        for descriptor in &self.descriptors {
            if !seen_descriptors.contains(&descriptor.got_id) {
                return Err(Error::manifest(format!(
                    "descriptor `{}` has no GOT slot",
                    descriptor.key
                )));
            }
        }
        Ok(())
    }

    fn validate_plan(&self, active: &BTreeMap<GotId, (u8, String)>) -> Result<()> {
        let plan = &self.plan;
        let parent_present =
            plan.parent_build_id.is_some() && plan.parent_manifest_digest.is_some();
        if plan.parent_build_id.is_some() != plan.parent_manifest_digest.is_some() {
            return Err(Error::plan(
                "plan parent build ID and parent manifest digest must both be present or absent",
            ));
        }
        if plan.parent_build_id != self.header.parent_build_id
            || plan.parent_manifest_digest != self.header.parent_manifest_digest
        {
            return Err(Error::plan(
                "plan parent identity must equal the manifest header parent identity",
            ));
        }
        match plan.verdict {
            PlanVerdict::Cold => {
                if parent_present {
                    return Err(Error::plan("a cold plan must not have a parent"));
                }
            }
            PlanVerdict::OnlineCandidate => {
                if !parent_present {
                    return Err(Error::plan("an online-candidate plan must have a parent"));
                }
            }
            PlanVerdict::Rejected => {}
        }
        let mut seen = BTreeSet::new();
        for action in &plan.actions {
            if !seen.insert(action.got_id) {
                return Err(Error::plan(format!(
                    "duplicate plan action for {}",
                    action.got_id
                )));
            }
            let kind = IdentityKind::from_tag(action.kind).ok_or_else(|| {
                Error::plan(format!(
                    "invalid identity kind {} in plan action",
                    action.kind
                ))
            })?;
            validate_key(kind, &action.key)?;
            if let Some(old_key) = &action.old_key {
                validate_key(kind, old_key)?;
            }
            let active_entry = active.get(&action.got_id);
            if let Some((active_kind, active_key)) = active_entry
                && (*active_kind != action.kind || active_key != &action.key)
            {
                return Err(Error::plan(format!(
                    "action for {} does not match the active record",
                    action.got_id
                )));
            }
            match action.action {
                PlanAction::Drop => {
                    if active_entry.is_some() {
                        return Err(Error::plan(format!(
                            "drop action for active identity {}",
                            action.got_id
                        )));
                    }
                }
                PlanAction::Allocate | PlanAction::Preserve => {
                    if active_entry.is_none() {
                        return Err(Error::plan(format!(
                            "{} action for inactive identity {}",
                            action.action.name(),
                            action.got_id
                        )));
                    }
                }
                PlanAction::Migrate | PlanAction::Reset | PlanAction::Reject => {}
            }
            if plan.verdict == PlanVerdict::Cold && action.action != PlanAction::Allocate {
                return Err(Error::plan(
                    "a cold plan must classify every identity as allocate",
                ));
            }
        }
        for got_id in active.keys() {
            if !seen.contains(got_id) {
                return Err(Error::plan(format!(
                    "plan does not cover active identity {got_id}"
                )));
            }
        }
        for diagnostic in &plan.diagnostics {
            if crate::error::Severity::from_tag(diagnostic.severity).is_none() {
                return Err(Error::plan(format!(
                    "invalid diagnostic severity {}",
                    diagnostic.severity
                )));
            }
        }
        Ok(())
    }

    fn validate_history(&self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for record in &self.history.retired {
            if !seen.insert(record.got_id) {
                return Err(Error::manifest(format!(
                    "duplicate retired identity {}",
                    record.got_id
                )));
            }
            if record.retired_build_id.is_zero() || record.retired_manifest_digest.is_zero() {
                return Err(Error::manifest(
                    "history record must carry nonzero build and manifest identities",
                ));
            }
            let kind = IdentityKind::from_tag(record.kind).ok_or_else(|| {
                Error::manifest(format!("invalid identity kind {} in history", record.kind))
            })?;
            validate_key(kind, &record.key)?;
            validate_previous_keys(kind, &record.key, &record.previous_keys)?;
        }
        Ok(())
    }
}

fn decode_payload<T: Decode>(payload: &[u8]) -> Result<Vec<T>> {
    crate::model::decode_records(payload)
}

fn decode_record<T: Decode>(payload: &[u8]) -> Result<T> {
    let mut input = Decoder::new(payload);
    let value = T::decode(&mut input)?;
    input.finish()?;
    Ok(value)
}

fn validate_table_order<T>(
    values: &[T],
    compare: impl Fn(&T, &T) -> std::cmp::Ordering,
    what: &str,
) -> Result<()> {
    for pair in values.windows(2) {
        if compare(&pair[0], &pair[1]) != std::cmp::Ordering::Less {
            return Err(Error::manifest(format!(
                "{what} table is not in canonical order"
            )));
        }
    }
    Ok(())
}

fn insert_key<'a>(
    keys: &mut BTreeMap<IdentityKind, BTreeSet<&'a str>>,
    kind: IdentityKind,
    key: &'a str,
) -> Result<()> {
    if keys.entry(kind).or_default().insert(key) {
        Ok(())
    } else {
        Err(Error::manifest(format!(
            "duplicate {} key `{key}`",
            kind.name()
        )))
    }
}

fn insert_identity(
    active: &mut BTreeMap<GotId, (u8, String)>,
    got_id: GotId,
    kind: IdentityKind,
    key: &str,
) -> Result<()> {
    if got_id.is_zero() {
        return Err(Error::manifest(format!(
            "{} `{key}` has a zero GotId",
            kind.name()
        )));
    }
    match active.get(&got_id) {
        None => {
            active.insert(got_id, (kind.as_u8(), key.to_owned()));
            Ok(())
        }
        Some((existing_kind, existing_key)) => {
            if *existing_kind == kind.as_u8() && existing_key == key {
                Ok(())
            } else {
                Err(Error::manifest(format!(
                    "GotId {got_id} is shared by different identities"
                )))
            }
        }
    }
}

fn validate_previous_keys(kind: IdentityKind, key: &str, previous: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for old in previous {
        validate_key(kind, old)?;
        if old == key {
            return Err(Error::manifest(format!(
                "previous_keys for `{key}` contains the current key"
            )));
        }
        if !seen.insert(old) {
            return Err(Error::manifest(format!(
                "previous_keys for `{key}` contains a duplicate"
            )));
        }
    }
    Ok(())
}

fn validate_align(align: u32, what: &str, key: &str) -> Result<()> {
    if !align.is_power_of_two() {
        return Err(Error::manifest(format!(
            "{what} `{key}` alignment {align} is not a power of two"
        )));
    }
    Ok(())
}

fn validate_field_path(path: &str, key: &str) -> Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    crate::keys::normalize_name_path(path)
        .map_err(|error| Error::manifest(format!("root `{key}` field_path: {}", error.message)))?;
    Ok(())
}

fn validate_fields(types: &[TypeRecord], ty: &TypeRecord) -> Result<()> {
    let mut previous: Option<(u64, &str)> = None;
    let mut ids = BTreeSet::new();
    for field in &ty.fields {
        if !ids.insert(field.got_id) {
            return Err(Error::manifest(format!(
                "type `{}` has duplicate field identities",
                ty.key
            )));
        }
        if field.got_id.is_zero() {
            return Err(Error::manifest(format!(
                "type `{}` has a zero field identity",
                ty.key
            )));
        }
        let normalized = crate::keys::normalize_name_path(&field.path).map_err(|error| {
            Error::manifest(format!("type `{}` field path: {}", ty.key, error.message))
        })?;
        if normalized != field.path {
            return Err(Error::manifest(format!(
                "type `{}` field path `{}` is not normalized",
                ty.key, field.path
            )));
        }
        TypeKey::parse(&field.type_key).map_err(|error| {
            Error::manifest(format!(
                "type `{}` field `{}` type key: {}",
                ty.key, field.path, error.message
            ))
        })?;
        if field.layout_hash.is_zero() {
            return Err(Error::manifest(format!(
                "type `{}` field `{}` has a zero layout hash",
                ty.key, field.path
            )));
        }
        validate_align(field.align, "field", &field.path)?;
        let end = field.offset.checked_add(field.size).ok_or_else(|| {
            Error::manifest(format!(
                "type `{}` field `{}` offset overflows",
                ty.key, field.path
            ))
        })?;
        if end > ty.size {
            return Err(Error::manifest(format!(
                "type `{}` field `{}` exceeds the type size",
                ty.key, field.path
            )));
        }
        if let Some((offset, path)) = previous
            && (offset, path) >= (field.offset, field.path.as_str())
        {
            return Err(Error::manifest(format!(
                "type `{}` fields are not sorted by (offset, path)",
                ty.key
            )));
        }
        previous = Some((field.offset, field.path.as_str()));
    }
    if let Some(base) = ty.base {
        require_type(types, base, "type base")?;
    }
    for interface in &ty.interfaces {
        require_type(types, *interface, "type interface")?;
    }
    Ok(())
}

fn require_type(types: &[TypeRecord], got_id: GotId, what: &str) -> Result<()> {
    if types.iter().any(|ty| ty.got_id == got_id) {
        Ok(())
    } else {
        Err(Error::manifest(format!(
            "{what} references a missing type {got_id}"
        )))
    }
}

/// Returns the signature portion of a function or export key.
fn key_signature_matches(key: &str, signature: &str) -> Result<bool> {
    let Some((_, rest)) = key.rsplit_once('@') else {
        return Ok(false);
    };
    let rest = rest.split("#C:").next().unwrap_or(rest);
    Ok(rest == signature)
}

fn check_keys(object: &BTreeMap<String, Json>, allowed: &[&str], what: &str) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::manifest(format!(
                "{what} contains unknown field `{key}`"
            )));
        }
    }
    for key in allowed {
        if !object.contains_key(*key) {
            return Err(Error::manifest(format!("{what} is missing field `{key}`")));
        }
    }
    Ok(())
}

fn number(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<i128> {
    object
        .get(name)
        .and_then(Json::as_number)
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be an integer")))
}

fn string(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<String> {
    object
        .get(name)
        .and_then(Json::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a string")))
}

fn array_records<T>(
    object: &BTreeMap<String, Json>,
    name: &str,
    parse: impl Fn(&Json) -> Result<T>,
) -> Result<Vec<T>> {
    let values = object
        .get(name)
        .and_then(Json::as_array)
        .ok_or_else(|| Error::manifest(format!("manifest body.{name} must be an array")))?;
    let mut out = Vec::new();
    for value in values {
        out.push(parse(value)?);
    }
    Ok(out)
}
