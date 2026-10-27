//! JSON projection of the manifest records (GOT Artifacts §2.2, §9).

use std::collections::BTreeMap;

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result, Severity};
use crate::json::Json;
use crate::model::{
    ActionRecord, AggregateFieldRecord, AggregateKind, AggregateRecord, DescriptorKind,
    DescriptorRecord, EnumMemberRecord, ExportRecord, FieldRecord, FunctionKind, FunctionRecord,
    Header, History, InitializerKind, InstanceRecord, Plan, PlanAction, PlanDiagnostic,
    PlanVerdict, Retention, RetiredCause, RetiredRecord, RootKind, RootRecord, SlotKind,
    SlotRecord, SourceLocation, SubrangeBounds, TaskRecord, TypeKind, TypeMetadataRecord,
    TypeRecord,
};

fn object<'a>(value: &'a Json, what: &str) -> Result<&'a BTreeMap<String, Json>> {
    value
        .as_object()
        .ok_or_else(|| Error::manifest(format!("{what} must be a JSON object")))
}

fn get<'a>(object: &'a BTreeMap<String, Json>, name: &str, what: &str) -> Result<&'a Json> {
    object
        .get(name)
        .ok_or_else(|| Error::manifest(format!("{what} is missing field `{name}`")))
}

fn check_unknown(object: &BTreeMap<String, Json>, allowed: &[&str], what: &str) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::manifest(format!(
                "{what} contains unknown field `{key}`"
            )));
        }
    }
    Ok(())
}

fn string(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<String> {
    get(object, name, what)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a string")))
}

fn optional_string(
    object: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
) -> Result<Option<String>> {
    let value = get(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a string or null")))
    }
}

fn bool_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<bool> {
    get(object, name, what)?
        .as_bool()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a boolean")))
}

fn number(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<i128> {
    get(object, name, what)?
        .as_number()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be an integer")))
}

fn u8_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<u8> {
    u8::try_from(number(object, name, what)?)
        .map_err(|_| Error::manifest(format!("{what}.{name} is out of range")))
}

fn u16_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<u16> {
    u16::try_from(number(object, name, what)?)
        .map_err(|_| Error::manifest(format!("{what}.{name} is out of range")))
}

fn u32_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<u32> {
    u32::try_from(number(object, name, what)?)
        .map_err(|_| Error::manifest(format!("{what}.{name} is out of range")))
}

fn u64_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<u64> {
    u64::try_from(number(object, name, what)?)
        .map_err(|_| Error::manifest(format!("{what}.{name} is out of range")))
}

fn optional_u32(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Option<u32>> {
    let value = get(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_number()
            .and_then(|number| u32::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a u32 or null")))
    }
}

fn optional_i64(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Option<i64>> {
    let value = get(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_number()
            .and_then(|number| i64::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| Error::manifest(format!("{what}.{name} must be an i64 or null")))
    }
}

fn id_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<GotId> {
    let text = get(object, name, what)?
        .as_str()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a `0x` string")))?;
    GotId::parse(text).map_err(|error| Error::manifest(format!("{what}.{name}: {}", error.message)))
}

fn optional_id(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Option<GotId>> {
    let value = get(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        let text = value.as_str().ok_or_else(|| {
            Error::manifest(format!("{what}.{name} must be a `0x` string or null"))
        })?;
        GotId::parse(text)
            .map(Some)
            .map_err(|error| Error::manifest(format!("{what}.{name}: {}", error.message)))
    }
}

fn digest_value(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Digest32> {
    let text = get(object, name, what)?
        .as_str()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be a `0x` string")))?;
    Digest32::parse(text)
        .map_err(|error| Error::manifest(format!("{what}.{name}: {}", error.message)))
}

fn optional_digest(
    object: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
) -> Result<Option<Digest32>> {
    let value = get(object, name, what)?;
    if value.is_null() {
        Ok(None)
    } else {
        let text = value.as_str().ok_or_else(|| {
            Error::manifest(format!("{what}.{name} must be a `0x` string or null"))
        })?;
        Digest32::parse(text)
            .map(Some)
            .map_err(|error| Error::manifest(format!("{what}.{name}: {}", error.message)))
    }
}

fn string_list(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    for (index, value) in array(object, name, what)?.iter().enumerate() {
        values.push(
            value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                Error::manifest(format!("{what}.{name}[{index}] must be a string"))
            })?,
        );
    }
    Ok(values)
}

fn array<'a>(object: &'a BTreeMap<String, Json>, name: &str, what: &str) -> Result<&'a [Json]> {
    get(object, name, what)?
        .as_array()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be an array")))
}

fn id_list(object: &BTreeMap<String, Json>, name: &str, what: &str) -> Result<Vec<GotId>> {
    let mut values = Vec::new();
    for (index, value) in array(object, name, what)?.iter().enumerate() {
        let text = value.as_str().ok_or_else(|| {
            Error::manifest(format!("{what}.{name}[{index}] must be a `0x` string"))
        })?;
        values.push(GotId::parse(text).map_err(|error| {
            Error::manifest(format!("{what}.{name}[{index}]: {}", error.message))
        })?);
    }
    Ok(values)
}

fn enum_name<'a>(object: &'a BTreeMap<String, Json>, name: &str, what: &str) -> Result<&'a str> {
    get(object, name, what)?
        .as_str()
        .ok_or_else(|| Error::manifest(format!("{what}.{name} must be an enum name")))
}

fn enum_value<T>(
    object: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
    parse: impl Fn(&str) -> Option<T>,
    allowed: &str,
) -> Result<T> {
    let text = enum_name(object, name, what)?;
    parse(text).ok_or_else(|| {
        Error::manifest(format!(
            "{what}.{name} has unknown value `{text}` (expected {allowed})"
        ))
    })
}

fn source_to_json(source: &Option<SourceLocation>) -> Json {
    source.as_ref().map_or(Json::Null, |source| {
        Json::object([
            ("package".into(), Json::string(source.package.clone())),
            ("file".into(), Json::string(source.file.clone())),
            ("start".into(), Json::unsigned(u64::from(source.start))),
            ("end".into(), Json::unsigned(u64::from(source.end))),
        ])
    })
}

fn source_from_json(
    container: &BTreeMap<String, Json>,
    name: &str,
    what: &str,
) -> Result<Option<SourceLocation>> {
    let value = get(container, name, what)?;
    if value.is_null() {
        return Ok(None);
    }
    let source = object(value, &format!("{what}.{name}"))?;
    check_unknown(
        source,
        &["end", "file", "package", "start"],
        &format!("{what}.{name}"),
    )?;
    Ok(Some(SourceLocation {
        package: string(source, "package", &format!("{what}.{name}"))?,
        file: string(source, "file", &format!("{what}.{name}"))?,
        start: u32_value(source, "start", &format!("{what}.{name}"))?,
        end: u32_value(source, "end", &format!("{what}.{name}"))?,
    }))
}

impl SourceLocation {
    /// Projects the location to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        source_to_json(&Some(self.clone()))
    }

    /// Parses a location object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "source location")?;
        check_unknown(
            object,
            &["end", "file", "package", "start"],
            "source location",
        )?;
        Ok(Self {
            package: string(object, "package", "source location")?,
            file: string(object, "file", "source location")?,
            start: u32_value(object, "start", "source location")?,
            end: u32_value(object, "end", "source location")?,
        })
    }
}

impl Header {
    /// Projects the header to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "active_configuration".into(),
                Json::string(self.active_configuration.clone()),
            ),
            ("build_id".into(), Json::string(self.build_id.to_hex())),
            (
                "catalog_digest".into(),
                Json::string(self.catalog_digest.to_hex()),
            ),
            (
                "compiler_version".into(),
                Json::string(self.compiler_version.clone()),
            ),
            (
                "descriptor_digest".into(),
                Json::string(self.descriptor_digest.to_hex()),
            ),
            (
                "endianness".into(),
                Json::unsigned(u64::from(self.endianness)),
            ),
            (
                "generator_flags".into(),
                Json::unsigned(self.generator_flags),
            ),
            (
                "host_abi_major".into(),
                Json::unsigned(u64::from(self.host_abi_major)),
            ),
            (
                "host_abi_minor".into(),
                Json::unsigned(u64::from(self.host_abi_minor)),
            ),
            (
                "module_format".into(),
                Json::unsigned(u64::from(self.module_format)),
            ),
            ("module_name".into(), Json::string(self.module_name.clone())),
            (
                "parent_build_id".into(),
                Json::optional(self.parent_build_id.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "parent_manifest_digest".into(),
                Json::optional(
                    self.parent_manifest_digest
                        .map(|digest| Json::string(digest.to_hex())),
                ),
            ),
            (
                "plan_digest".into(),
                Json::string(self.plan_digest.to_hex()),
            ),
            (
                "pointer_width".into(),
                Json::unsigned(u64::from(self.pointer_width)),
            ),
            ("profile".into(), Json::string(self.profile.clone())),
            (
                "required_features".into(),
                Json::unsigned(self.required_features),
            ),
            (
                "source_fingerprint".into(),
                Json::string(self.source_fingerprint.to_hex()),
            ),
            (
                "target_triple".into(),
                Json::string(self.target_triple.clone()),
            ),
        ])
    }

    /// Parses a header object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "header")?;
        check_unknown(
            object,
            &[
                "active_configuration",
                "build_id",
                "catalog_digest",
                "compiler_version",
                "descriptor_digest",
                "endianness",
                "generator_flags",
                "host_abi_major",
                "host_abi_minor",
                "module_format",
                "module_name",
                "parent_build_id",
                "parent_manifest_digest",
                "plan_digest",
                "pointer_width",
                "profile",
                "required_features",
                "source_fingerprint",
                "target_triple",
            ],
            "header",
        )?;
        Ok(Self {
            profile: string(object, "profile", "header")?,
            compiler_version: string(object, "compiler_version", "header")?,
            target_triple: string(object, "target_triple", "header")?,
            module_name: string(object, "module_name", "header")?,
            active_configuration: string(object, "active_configuration", "header")?,
            endianness: u8_value(object, "endianness", "header")?,
            pointer_width: u8_value(object, "pointer_width", "header")?,
            module_format: u16_value(object, "module_format", "header")?,
            host_abi_major: u16_value(object, "host_abi_major", "header")?,
            host_abi_minor: u16_value(object, "host_abi_minor", "header")?,
            generator_flags: u64_value(object, "generator_flags", "header")?,
            required_features: u64_value(object, "required_features", "header")?,
            build_id: id_value(object, "build_id", "header")?,
            parent_build_id: optional_id(object, "parent_build_id", "header")?,
            parent_manifest_digest: optional_digest(object, "parent_manifest_digest", "header")?,
            descriptor_digest: digest_value(object, "descriptor_digest", "header")?,
            source_fingerprint: digest_value(object, "source_fingerprint", "header")?,
            catalog_digest: digest_value(object, "catalog_digest", "header")?,
            plan_digest: digest_value(object, "plan_digest", "header")?,
        })
    }
}

impl RootRecord {
    /// Projects the root to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("align".into(), Json::unsigned(u64::from(self.align))),
            ("field_path".into(), Json::string(self.field_path.clone())),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("initializer".into(), Json::string(self.initializer.name())),
            (
                "initializer_digest".into(),
                Json::optional(
                    self.initializer_digest
                        .map(|digest| Json::string(digest.to_hex())),
                ),
            ),
            ("key".into(), Json::string(self.key.clone())),
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            (
                "owner_type".into(),
                Json::optional(self.owner_type.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            ("retention".into(), Json::string(self.retention.name())),
            ("root_kind".into(), Json::string(self.root_kind.name())),
            ("size".into(), Json::unsigned(self.size)),
            ("slot".into(), Json::unsigned(u64::from(self.slot))),
            ("source".into(), source_to_json(&self.source)),
            ("type_key".into(), Json::string(self.type_key.clone())),
        ])
    }

    /// Parses a root object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "root record")?;
        check_unknown(
            object,
            &[
                "align",
                "field_path",
                "got_id",
                "initializer",
                "initializer_digest",
                "key",
                "layout_hash",
                "owner_type",
                "previous_keys",
                "retention",
                "root_kind",
                "size",
                "slot",
                "source",
                "type_key",
            ],
            "root record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "root record")?,
            key: string(object, "key", "root record")?,
            previous_keys: string_list(object, "previous_keys", "root record")?,
            root_kind: enum_value(
                object,
                "root_kind",
                "root record",
                RootKind::from_name,
                "global, program_field, nested_field, direct, task_guard",
            )?,
            owner_type: optional_id(object, "owner_type", "root record")?,
            field_path: string(object, "field_path", "root record")?,
            type_key: string(object, "type_key", "root record")?,
            layout_hash: id_value(object, "layout_hash", "root record")?,
            size: u64_value(object, "size", "root record")?,
            align: u32_value(object, "align", "root record")?,
            retention: enum_value(
                object,
                "retention",
                "root record",
                Retention::from_name,
                "none, retain",
            )?,
            initializer: enum_value(
                object,
                "initializer",
                "root record",
                InitializerKind::from_name,
                "none, zero, constant, expression",
            )?,
            initializer_digest: optional_digest(object, "initializer_digest", "root record")?,
            source: source_from_json(object, "source", "root record")?,
            slot: u32_value(object, "slot", "root record")?,
        })
    }
}

impl FunctionRecord {
    /// Projects the function to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "export_name".into(),
                Json::optional(self.export_name.clone().map(Json::string)),
            ),
            ("flags".into(), Json::unsigned(u64::from(self.flags))),
            (
                "function_kind".into(),
                Json::string(self.function_kind.name()),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("key".into(), Json::string(self.key.clone())),
            (
                "owner_type".into(),
                Json::optional(self.owner_type.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            (
                "signature_key".into(),
                Json::string(self.signature_key.clone()),
            ),
            ("slot".into(), Json::unsigned(u64::from(self.slot))),
            ("source".into(), source_to_json(&self.source)),
            (
                "vtable_slot".into(),
                Json::optional(self.vtable_slot.map(|slot| Json::unsigned(u64::from(slot)))),
            ),
        ])
    }

    /// Parses a function object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "function record")?;
        check_unknown(
            object,
            &[
                "export_name",
                "flags",
                "function_kind",
                "got_id",
                "key",
                "owner_type",
                "previous_keys",
                "signature_key",
                "slot",
                "source",
                "vtable_slot",
            ],
            "function record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "function record")?,
            key: string(object, "key", "function record")?,
            previous_keys: string_list(object, "previous_keys", "function record")?,
            signature_key: string(object, "signature_key", "function record")?,
            function_kind: enum_value(
                object,
                "function_kind",
                "function record",
                FunctionKind::from_name,
                "function, program_body, fb_body, method, export_wrapper, std_function",
            )?,
            owner_type: optional_id(object, "owner_type", "function record")?,
            flags: u32_value(object, "flags", "function record")?,
            slot: u32_value(object, "slot", "function record")?,
            export_name: optional_string(object, "export_name", "function record")?,
            vtable_slot: optional_u32(object, "vtable_slot", "function record")?,
            source: source_from_json(object, "source", "function record")?,
        })
    }
}

impl FieldRecord {
    /// Projects the field to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("align".into(), Json::unsigned(u64::from(self.align))),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            ("offset".into(), Json::unsigned(self.offset)),
            ("path".into(), Json::string(self.path.clone())),
            ("retention".into(), Json::string(self.retention.name())),
            ("size".into(), Json::unsigned(self.size)),
            ("source".into(), source_to_json(&self.source)),
            ("type_key".into(), Json::string(self.type_key.clone())),
        ])
    }

    /// Parses a field object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "field record")?;
        check_unknown(
            object,
            &[
                "align",
                "got_id",
                "layout_hash",
                "offset",
                "path",
                "retention",
                "size",
                "source",
                "type_key",
            ],
            "field record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "field record")?,
            path: string(object, "path", "field record")?,
            type_key: string(object, "type_key", "field record")?,
            layout_hash: id_value(object, "layout_hash", "field record")?,
            offset: u64_value(object, "offset", "field record")?,
            size: u64_value(object, "size", "field record")?,
            align: u32_value(object, "align", "field record")?,
            retention: enum_value(
                object,
                "retention",
                "field record",
                Retention::from_name,
                "none, retain",
            )?,
            source: source_from_json(object, "source", "field record")?,
        })
    }
}

impl TypeRecord {
    /// Projects the type to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("align".into(), Json::unsigned(u64::from(self.align))),
            (
                "base".into(),
                Json::optional(self.base.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "fields".into(),
                Json::Array(self.fields.iter().map(FieldRecord::to_json).collect()),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            (
                "interfaces".into(),
                Json::Array(
                    self.interfaces
                        .iter()
                        .map(|id| Json::string(id.to_hex()))
                        .collect(),
                ),
            ),
            ("key".into(), Json::string(self.key.clone())),
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            ("size".into(), Json::unsigned(self.size)),
            ("source".into(), source_to_json(&self.source)),
            ("type_kind".into(), Json::string(self.type_kind.name())),
            ("type_key".into(), Json::string(self.type_key.clone())),
            (
                "vtable_shape".into(),
                Json::optional(self.vtable_shape.map(|id| Json::string(id.to_hex()))),
            ),
        ])
    }

    /// Parses a type object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "type record")?;
        check_unknown(
            object,
            &[
                "align",
                "base",
                "fields",
                "got_id",
                "interfaces",
                "key",
                "layout_hash",
                "previous_keys",
                "size",
                "source",
                "type_kind",
                "type_key",
                "vtable_shape",
            ],
            "type record",
        )?;
        let mut fields = Vec::new();
        for field in array(object, "fields", "type record")? {
            fields.push(FieldRecord::from_json(field)?);
        }
        Ok(Self {
            got_id: id_value(object, "got_id", "type record")?,
            key: string(object, "key", "type record")?,
            previous_keys: string_list(object, "previous_keys", "type record")?,
            type_key: string(object, "type_key", "type record")?,
            layout_hash: id_value(object, "layout_hash", "type record")?,
            size: u64_value(object, "size", "type record")?,
            align: u32_value(object, "align", "type record")?,
            type_kind: enum_value(
                object,
                "type_kind",
                "type record",
                TypeKind::from_name,
                "struct, function_block, class, interface, enum, subrange, alias",
            )?,
            base: optional_id(object, "base", "type record")?,
            interfaces: id_list(object, "interfaces", "type record")?,
            vtable_shape: optional_id(object, "vtable_shape", "type record")?,
            fields,
            source: source_from_json(object, "source", "type record")?,
        })
    }
}

impl EnumMemberRecord {
    /// Projects the member to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.clone())),
            ("value".into(), Json::Number(self.value)),
        ])
    }

    /// Parses a member object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "enum member")?;
        check_unknown(object, &["name", "value"], "enum member")?;
        Ok(Self {
            name: string(object, "name", "enum member")?,
            value: number(object, "value", "enum member")?,
        })
    }
}

impl SubrangeBounds {
    /// Projects the bounds to JSON.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            ("lower".into(), Json::Number(self.lower)),
            ("upper".into(), Json::Number(self.upper)),
        ])
    }

    /// Parses a bounds object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "subrange")?;
        check_unknown(object, &["lower", "upper"], "subrange")?;
        Ok(Self {
            lower: number(object, "lower", "subrange")?,
            upper: number(object, "upper", "subrange")?,
        })
    }
}

impl TypeMetadataRecord {
    /// Projects the metadata record to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "enum_members".into(),
                Json::Array(
                    self.enum_members
                        .iter()
                        .map(EnumMemberRecord::to_json)
                        .collect(),
                ),
            ),
            (
                "subrange".into(),
                Json::optional(self.subrange.map(SubrangeBounds::to_json)),
            ),
            ("type_id".into(), Json::string(self.type_id.to_hex())),
        ])
    }

    /// Parses a metadata record.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "type metadata")?;
        check_unknown(
            object,
            &["enum_members", "subrange", "type_id"],
            "type metadata",
        )?;
        let mut enum_members = Vec::new();
        for member in array(object, "enum_members", "type metadata")? {
            enum_members.push(EnumMemberRecord::from_json(member)?);
        }
        let subrange_value = get(object, "subrange", "type metadata")?;
        let subrange = if subrange_value.is_null() {
            None
        } else {
            Some(SubrangeBounds::from_json(subrange_value)?)
        };
        Ok(Self {
            type_id: id_value(object, "type_id", "type metadata")?,
            enum_members,
            subrange,
        })
    }
}

impl AggregateFieldRecord {
    /// Projects the field to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            ("path".into(), Json::string(self.path.clone())),
            ("type_key".into(), Json::string(self.type_key.clone())),
        ])
    }

    /// Parses a field object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "aggregate field")?;
        check_unknown(
            object,
            &["layout_hash", "path", "type_key"],
            "aggregate field",
        )?;
        Ok(Self {
            path: string(object, "path", "aggregate field")?,
            type_key: string(object, "type_key", "aggregate field")?,
            layout_hash: id_value(object, "layout_hash", "aggregate field")?,
        })
    }
}

impl AggregateRecord {
    /// Projects the aggregate shape to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "fields".into(),
                Json::Array(
                    self.fields
                        .iter()
                        .map(AggregateFieldRecord::to_json)
                        .collect(),
                ),
            ),
            ("kind".into(), Json::string(self.kind.name())),
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            ("package".into(), Json::string(self.package.clone())),
            ("path".into(), Json::string(self.path.clone())),
            (
                "qualifier".into(),
                Json::optional(self.qualifier.clone().map(Json::string)),
            ),
            ("shape_key".into(), Json::string(self.shape_key.clone())),
        ])
    }

    /// Parses an aggregate shape.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "aggregate")?;
        check_unknown(
            object,
            &[
                "fields",
                "kind",
                "layout_hash",
                "package",
                "path",
                "qualifier",
                "shape_key",
            ],
            "aggregate",
        )?;
        let mut fields = Vec::new();
        for field in array(object, "fields", "aggregate")? {
            fields.push(AggregateFieldRecord::from_json(field)?);
        }
        Ok(Self {
            package: string(object, "package", "aggregate")?,
            path: string(object, "path", "aggregate")?,
            qualifier: optional_string(object, "qualifier", "aggregate")?,
            kind: enum_value(
                object,
                "kind",
                "aggregate",
                AggregateKind::from_name,
                "type or program",
            )?,
            shape_key: string(object, "shape_key", "aggregate")?,
            layout_hash: id_value(object, "layout_hash", "aggregate")?,
            fields,
        })
    }
}

impl DescriptorRecord {
    /// Projects the descriptor to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("align".into(), Json::unsigned(u64::from(self.align))),
            (
                "descriptor_kind".into(),
                Json::string(self.descriptor_kind.name()),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("key".into(), Json::string(self.key.clone())),
            (
                "layout_hash".into(),
                Json::string(self.layout_hash.to_hex()),
            ),
            (
                "owner_type".into(),
                Json::optional(self.owner_type.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            ("size".into(), Json::unsigned(self.size)),
            ("slot".into(), Json::unsigned(u64::from(self.slot))),
            ("source".into(), source_to_json(&self.source)),
            ("type_key".into(), Json::string(self.type_key.clone())),
        ])
    }

    /// Parses a descriptor object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "descriptor record")?;
        check_unknown(
            object,
            &[
                "align",
                "descriptor_kind",
                "got_id",
                "key",
                "layout_hash",
                "owner_type",
                "previous_keys",
                "size",
                "slot",
                "source",
                "type_key",
            ],
            "descriptor record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "descriptor record")?,
            key: string(object, "key", "descriptor record")?,
            previous_keys: string_list(object, "previous_keys", "descriptor record")?,
            descriptor_kind: enum_value(
                object,
                "descriptor_kind",
                "descriptor record",
                DescriptorKind::from_name,
                "type_info, vtable, object",
            )?,
            owner_type: optional_id(object, "owner_type", "descriptor record")?,
            type_key: string(object, "type_key", "descriptor record")?,
            layout_hash: id_value(object, "layout_hash", "descriptor record")?,
            size: u64_value(object, "size", "descriptor record")?,
            align: u32_value(object, "align", "descriptor record")?,
            slot: u32_value(object, "slot", "descriptor record")?,
            source: source_from_json(object, "source", "descriptor record")?,
        })
    }
}

impl InstanceRecord {
    /// Projects the instance to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "configuration".into(),
                Json::string(self.configuration.clone()),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            (
                "instance_path".into(),
                Json::string(self.instance_path.clone()),
            ),
            ("key".into(), Json::string(self.key.clone())),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            (
                "program_type".into(),
                Json::optional(self.program_type.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "program_type_key".into(),
                Json::string(self.program_type_key.clone()),
            ),
            ("resource".into(), Json::string(self.resource.clone())),
            ("source".into(), source_to_json(&self.source)),
            (
                "task_key".into(),
                Json::optional(self.task_key.clone().map(Json::string)),
            ),
        ])
    }

    /// Parses an instance object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "instance record")?;
        check_unknown(
            object,
            &[
                "configuration",
                "got_id",
                "instance_path",
                "key",
                "previous_keys",
                "program_type",
                "program_type_key",
                "resource",
                "source",
                "task_key",
            ],
            "instance record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "instance record")?,
            key: string(object, "key", "instance record")?,
            previous_keys: string_list(object, "previous_keys", "instance record")?,
            program_type: optional_id(object, "program_type", "instance record")?,
            program_type_key: string(object, "program_type_key", "instance record")?,
            configuration: string(object, "configuration", "instance record")?,
            resource: string(object, "resource", "instance record")?,
            instance_path: string(object, "instance_path", "instance record")?,
            task_key: optional_string(object, "task_key", "instance record")?,
            source: source_from_json(object, "source", "instance record")?,
        })
    }
}

impl TaskRecord {
    /// Projects the task to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "configuration".into(),
                Json::string(self.configuration.clone()),
            ),
            (
                "guard_got_id".into(),
                Json::optional(self.guard_got_id.map(|id| Json::string(id.to_hex()))),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            (
                "interval_ns".into(),
                Json::optional(
                    self.interval_ns
                        .map(|value| Json::Number(i128::from(value))),
                ),
            ),
            ("key".into(), Json::string(self.key.clone())),
            ("name".into(), Json::string(self.name.clone())),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            (
                "priority".into(),
                Json::optional(self.priority.map(|value| Json::Number(i128::from(value)))),
            ),
            ("resource".into(), Json::string(self.resource.clone())),
            ("single".into(), Json::Bool(self.single)),
            ("source".into(), source_to_json(&self.source)),
        ])
    }

    /// Parses a task object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "task record")?;
        check_unknown(
            object,
            &[
                "configuration",
                "guard_got_id",
                "got_id",
                "interval_ns",
                "key",
                "name",
                "previous_keys",
                "priority",
                "resource",
                "single",
                "source",
            ],
            "task record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "task record")?,
            key: string(object, "key", "task record")?,
            previous_keys: string_list(object, "previous_keys", "task record")?,
            configuration: string(object, "configuration", "task record")?,
            resource: string(object, "resource", "task record")?,
            name: string(object, "name", "task record")?,
            single: bool_value(object, "single", "task record")?,
            interval_ns: optional_i64(object, "interval_ns", "task record")?,
            priority: optional_i64(object, "priority", "task record")?,
            guard_got_id: optional_id(object, "guard_got_id", "task record")?,
            source: source_from_json(object, "source", "task record")?,
        })
    }
}

impl ExportRecord {
    /// Projects the export to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("c_name".into(), Json::string(self.c_name.clone())),
            (
                "function_got_id".into(),
                Json::string(self.function_got_id.to_hex()),
            ),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("key".into(), Json::string(self.key.clone())),
            ("no_mangle".into(), Json::Bool(self.no_mangle)),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            (
                "signature_key".into(),
                Json::string(self.signature_key.clone()),
            ),
            ("source".into(), source_to_json(&self.source)),
        ])
    }

    /// Parses an export object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "export record")?;
        check_unknown(
            object,
            &[
                "c_name",
                "function_got_id",
                "got_id",
                "key",
                "no_mangle",
                "previous_keys",
                "signature_key",
                "source",
            ],
            "export record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "export record")?,
            key: string(object, "key", "export record")?,
            previous_keys: string_list(object, "previous_keys", "export record")?,
            c_name: string(object, "c_name", "export record")?,
            function_got_id: id_value(object, "function_got_id", "export record")?,
            signature_key: string(object, "signature_key", "export record")?,
            no_mangle: bool_value(object, "no_mangle", "export record")?,
            source: source_from_json(object, "source", "export record")?,
        })
    }
}

impl SlotRecord {
    /// Projects the slot to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("kind".into(), Json::string(self.kind.name())),
            ("service".into(), Json::unsigned(u64::from(self.service))),
            ("slot".into(), Json::unsigned(u64::from(self.slot))),
        ])
    }

    /// Parses a slot object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "slot record")?;
        check_unknown(
            object,
            &["got_id", "kind", "service", "slot"],
            "slot record",
        )?;
        Ok(Self {
            slot: u32_value(object, "slot", "slot record")?,
            kind: enum_value(
                object,
                "kind",
                "slot record",
                SlotKind::from_name,
                "data, function, descriptor, direct, import",
            )?,
            service: u8_value(object, "service", "slot record")?,
            got_id: id_value(object, "got_id", "slot record")?,
        })
    }
}

impl ActionRecord {
    /// Projects the action to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("action".into(), Json::string(self.action.name())),
            ("detail".into(), Json::string(self.detail.clone())),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("key".into(), Json::string(self.key.clone())),
            ("kind".into(), Json::unsigned(u64::from(self.kind))),
            (
                "old_key".into(),
                Json::optional(self.old_key.clone().map(Json::string)),
            ),
            ("reason_code".into(), Json::string(self.reason_code.clone())),
        ])
    }

    /// Parses an action object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "plan action")?;
        check_unknown(
            object,
            &[
                "action",
                "detail",
                "got_id",
                "key",
                "kind",
                "old_key",
                "reason_code",
            ],
            "plan action",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "plan action")?,
            kind: u8_value(object, "kind", "plan action")?,
            key: string(object, "key", "plan action")?,
            old_key: optional_string(object, "old_key", "plan action")?,
            action: enum_value(
                object,
                "action",
                "plan action",
                PlanAction::from_name,
                "preserve, allocate, migrate, reset, drop, reject",
            )?,
            reason_code: string(object, "reason_code", "plan action")?,
            detail: string(object, "detail", "plan action")?,
        })
    }
}

impl PlanDiagnostic {
    /// Projects the diagnostic to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let severity = Severity::from_tag(self.severity).map_or("warning", Severity::name);
        Json::object([
            ("code".into(), Json::string(self.code.clone())),
            (
                "got_id".into(),
                Json::optional(self.got_id.map(|id| Json::string(id.to_hex()))),
            ),
            ("message".into(), Json::string(self.message.clone())),
            (
                "new_key".into(),
                Json::optional(self.new_key.clone().map(Json::string)),
            ),
            (
                "old_key".into(),
                Json::optional(self.old_key.clone().map(Json::string)),
            ),
            ("severity".into(), Json::string(severity)),
        ])
    }

    /// Parses a diagnostic object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "plan diagnostic")?;
        check_unknown(
            object,
            &[
                "code", "got_id", "message", "new_key", "old_key", "severity",
            ],
            "plan diagnostic",
        )?;
        let severity = enum_value(
            object,
            "severity",
            "plan diagnostic",
            Severity::from_name,
            "warning, error",
        )?;
        Ok(Self {
            code: string(object, "code", "plan diagnostic")?,
            severity: severity.tag(),
            message: string(object, "message", "plan diagnostic")?,
            got_id: optional_id(object, "got_id", "plan diagnostic")?,
            old_key: optional_string(object, "old_key", "plan diagnostic")?,
            new_key: optional_string(object, "new_key", "plan diagnostic")?,
        })
    }
}

impl Plan {
    /// Projects the plan to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "actions".into(),
                Json::Array(self.actions.iter().map(ActionRecord::to_json).collect()),
            ),
            (
                "diagnostics".into(),
                Json::Array(
                    self.diagnostics
                        .iter()
                        .map(PlanDiagnostic::to_json)
                        .collect(),
                ),
            ),
            (
                "parent_build_id".into(),
                Json::optional(self.parent_build_id.map(|id| Json::string(id.to_hex()))),
            ),
            (
                "parent_manifest_digest".into(),
                Json::optional(
                    self.parent_manifest_digest
                        .map(|digest| Json::string(digest.to_hex())),
                ),
            ),
            ("verdict".into(), Json::string(self.verdict.name())),
        ])
    }

    /// Parses a plan object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "plan")?;
        check_unknown(
            object,
            &[
                "actions",
                "diagnostics",
                "parent_build_id",
                "parent_manifest_digest",
                "verdict",
            ],
            "plan",
        )?;
        let mut actions = Vec::new();
        for action in array(object, "actions", "plan")? {
            actions.push(ActionRecord::from_json(action)?);
        }
        let mut diagnostics = Vec::new();
        for diagnostic in array(object, "diagnostics", "plan")? {
            diagnostics.push(PlanDiagnostic::from_json(diagnostic)?);
        }
        Ok(Self {
            verdict: enum_value(
                object,
                "verdict",
                "plan",
                PlanVerdict::from_name,
                "cold, online_candidate, rejected",
            )?,
            parent_build_id: optional_id(object, "parent_build_id", "plan")?,
            parent_manifest_digest: optional_digest(object, "parent_manifest_digest", "plan")?,
            actions,
            diagnostics,
        })
    }
}

impl RetiredRecord {
    /// Projects the retired record to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("cause".into(), Json::string(self.cause.name())),
            ("got_id".into(), Json::string(self.got_id.to_hex())),
            ("key".into(), Json::string(self.key.clone())),
            ("kind".into(), Json::unsigned(u64::from(self.kind))),
            (
                "previous_keys".into(),
                Json::Array(
                    self.previous_keys
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            ),
            (
                "retired_build_id".into(),
                Json::string(self.retired_build_id.to_hex()),
            ),
            (
                "retired_manifest_digest".into(),
                Json::string(self.retired_manifest_digest.to_hex()),
            ),
        ])
    }

    /// Parses a retired object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "retired record")?;
        check_unknown(
            object,
            &[
                "cause",
                "got_id",
                "key",
                "kind",
                "previous_keys",
                "retired_build_id",
                "retired_manifest_digest",
            ],
            "retired record",
        )?;
        Ok(Self {
            got_id: id_value(object, "got_id", "retired record")?,
            kind: u8_value(object, "kind", "retired record")?,
            key: string(object, "key", "retired record")?,
            previous_keys: string_list(object, "previous_keys", "retired record")?,
            retired_build_id: id_value(object, "retired_build_id", "retired record")?,
            retired_manifest_digest: digest_value(
                object,
                "retired_manifest_digest",
                "retired record",
            )?,
            cause: enum_value(
                object,
                "cause",
                "retired record",
                RetiredCause::from_name,
                "dropped, reset, replaced",
            )?,
        })
    }
}

impl History {
    /// Projects the history to JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([(
            "retired".into(),
            Json::Array(self.retired.iter().map(RetiredRecord::to_json).collect()),
        )])
    }

    /// Parses a history object.
    ///
    /// # Errors
    ///
    /// Rejects unknown or malformed fields.
    pub fn from_json(value: &Json) -> Result<Self> {
        let object = object(value, "history")?;
        check_unknown(object, &["retired"], "history")?;
        let mut retired = Vec::new();
        for record in array(object, "retired", "history")? {
            retired.push(RetiredRecord::from_json(record)?);
        }
        Ok(Self { retired })
    }
}
