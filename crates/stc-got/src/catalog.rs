//! The compiler catalog input consumed by the identity engine.
//!
//! The catalog is the single model produced after semantic analysis. Codegen
//! and the manifest emitter are projections of it; the identity engine fills
//! in persistent `GotId`s, previous keys, slots, and plan actions.

use crate::digest::{Digest32, GotId};
use crate::error::{Error, Result};
use crate::keys::IdentityKind;
use crate::model::{
    DescriptorKind, FunctionKind, InitializerKind, Retention, RootKind, SourceLocation, TypeKind,
};
use crate::sha256::Sha256;

/// The identity tuple from which a `GotId` is derived.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identity {
    /// Identity kind.
    pub kind: IdentityKind,
    /// Normalized package name.
    pub package: String,
    /// Kind-specific path.
    pub path: String,
    /// Kind-specific qualifier.
    pub qualifier: String,
}

impl Identity {
    /// Creates an identity tuple.
    #[must_use]
    pub fn new(
        kind: IdentityKind,
        package: impl Into<String>,
        path: impl Into<String>,
        qualifier: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            package: package.into(),
            path: path.into(),
            qualifier: qualifier.into(),
        }
    }

    /// Derives the identity, returning the truncated `GotId` and the full
    /// SHA-256 digest used for collision checking.
    #[must_use]
    pub fn derive(&self) -> (GotId, Digest32) {
        GotId::derive(
            self.kind.as_u8(),
            &self.package,
            &self.path,
            &self.qualifier,
        )
    }
}

/// One persistent root declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
    /// Root shape.
    pub root_kind: RootKind,
    /// Owning type key, when the root is a type field.
    pub owner_type: Option<String>,
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
}

/// One callable declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
    /// Signature key.
    pub signature_key: String,
    /// Callable kind.
    pub function_kind: FunctionKind,
    /// Owning type key.
    pub owner_type: Option<String>,
    /// Descriptive flags.
    pub flags: u32,
    /// C export symbol, when the callable backs an export.
    pub export_name: Option<String>,
    /// M5 vtable slot.
    pub vtable_slot: Option<u32>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One field of a named type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDecl {
    /// Identity tuple.
    pub identity: Identity,
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

/// One named user-type declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
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
    /// Base type key.
    pub base: Option<String>,
    /// Implemented or extended type keys.
    pub interfaces: Vec<String>,
    /// Vtable shape hash.
    pub vtable_shape: Option<GotId>,
    /// Fields in declaration order.
    pub fields: Vec<FieldDecl>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One descriptor declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
    /// Descriptor kind.
    pub descriptor_kind: DescriptorKind,
    /// Owning type key.
    pub owner_type: Option<String>,
    /// Canonical type key.
    pub type_key: String,
    /// Target layout hash.
    pub layout_hash: GotId,
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u32,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One configured program instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
    /// Program type key.
    pub program_type: Option<String>,
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

/// One task declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
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
    /// Task-guard root key.
    pub guard_key: Option<String>,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// One C export declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDecl {
    /// Identity tuple.
    pub identity: Identity,
    /// Current stable key.
    pub key: String,
    /// C symbol name.
    pub c_name: String,
    /// Wrapped function key.
    pub function_key: String,
    /// Wrapped function signature key.
    pub signature_key: String,
    /// `{stc:no_mangle}` marker.
    pub no_mangle: bool,
    /// Source declaration.
    pub source: Option<SourceLocation>,
}

/// A host service referenced by an import slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportService {
    /// `trap`.
    Trap = 1,
    /// `cycle_now_ns`.
    CycleNowNs = 2,
    /// `monotonic_now_ns`.
    MonotonicNowNs = 3,
    /// `local_date_ns`.
    LocalDateNs = 4,
    /// `alloc`.
    Alloc = 5,
    /// `free`.
    Free = 6,
    /// `io_image`.
    IoImage = 7,
    /// `io_size`.
    IoSize = 8,
    /// `state_load`.
    StateLoad = 9,
    /// `state_save`.
    StateSave = 10,
    /// `assert`.
    Assert = 11,
}

impl ImportService {
    /// Returns the host API service ID.
    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// Returns the host API field name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Trap => "trap",
            Self::CycleNowNs => "cycle_now_ns",
            Self::MonotonicNowNs => "monotonic_now_ns",
            Self::LocalDateNs => "local_date_ns",
            Self::Alloc => "alloc",
            Self::Free => "free",
            Self::IoImage => "io_image",
            Self::IoSize => "io_size",
            Self::StateLoad => "state_load",
            Self::StateSave => "state_save",
            Self::Assert => "assert",
        }
    }
}

/// The complete compiler catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// Persistent roots.
    pub roots: Vec<RootDecl>,
    /// Callables.
    pub functions: Vec<FunctionDecl>,
    /// Named types.
    pub types: Vec<TypeDecl>,
    /// Runtime descriptors.
    pub descriptors: Vec<DescriptorDecl>,
    /// Configured instances.
    pub instances: Vec<InstanceDecl>,
    /// Tasks.
    pub tasks: Vec<TaskDecl>,
    /// C exports.
    pub exports: Vec<ExportDecl>,
    /// Host services used by generated code.
    pub imports: Vec<ImportService>,
}

impl Catalog {
    /// Validates declaration keys, kinds, and cross-references that do not
    /// depend on baseline matching.
    ///
    /// # Errors
    ///
    /// Rejects a mismatched identity kind, a malformed or duplicate key, and
    /// malformed type/signature keys.
    pub fn validate(&self) -> Result<()> {
        let mut keys: std::collections::BTreeMap<IdentityKind, std::collections::BTreeSet<&str>> =
            std::collections::BTreeMap::new();
        for root in &self.roots {
            let kind = root_kind_identity(root.root_kind);
            require_identity(&root.identity, kind, &root.key)?;
            validate_key_into(&mut keys, kind, &root.key)?;
            let guard_key = root
                .key
                .split_once('#')
                .is_some_and(|(_, suffix)| suffix.ends_with(".GUARD"));
            let keyed = root.key.contains('#');
            let shape_ok = match root.root_kind {
                RootKind::Global => !keyed,
                RootKind::ProgramField | RootKind::NestedField => keyed && !guard_key,
                RootKind::TaskGuard => guard_key,
                RootKind::Direct => true,
            };
            if !shape_ok {
                return Err(Error::manifest(format!(
                    "root `{}` key form does not match {:?}",
                    root.key, root.root_kind
                )));
            }
            crate::layout::TypeKey::parse(&root.type_key).map_err(|error| {
                Error::manifest(format!("root `{}` type key: {}", root.key, error.message))
            })?;
        }
        for function in &self.functions {
            require_identity(&function.identity, IdentityKind::Function, &function.key)?;
            validate_key_into(&mut keys, IdentityKind::Function, &function.key)?;
            crate::layout::Signature::parse(&function.signature_key).map_err(|error| {
                Error::manifest(format!(
                    "function `{}` signature: {}",
                    function.key, error.message
                ))
            })?;
        }
        for ty in &self.types {
            require_identity(&ty.identity, IdentityKind::Type, &ty.key)?;
            validate_key_into(&mut keys, IdentityKind::Type, &ty.key)?;
            crate::layout::TypeKey::parse(&ty.type_key).map_err(|error| {
                Error::manifest(format!("type `{}` type key: {}", ty.key, error.message))
            })?;
            let mut field_keys = std::collections::BTreeSet::new();
            for field in &ty.fields {
                if field.identity.kind != IdentityKind::Field {
                    return Err(Error::manifest(format!(
                        "type `{}` field `{}` has identity kind {:?}, expected field",
                        ty.key, field.path, field.identity.kind
                    )));
                }
                let expected_path = format!("{}.{}", ty.identity.path, field.path);
                if field.identity.package != ty.identity.package
                    || field.identity.path != expected_path
                    || !field.identity.qualifier.is_empty()
                {
                    return Err(Error::manifest(format!(
                        "type `{}` field `{}` identity tuple must be ({}, {}, \"\")",
                        ty.key, field.path, ty.identity.package, expected_path
                    )));
                }
                let field_key = crate::keys::field_identity_key(
                    &field.identity.package,
                    ty.identity.path.as_str(),
                    &field.path,
                )?;
                if !field_keys.insert(field_key) {
                    return Err(Error::manifest(format!(
                        "type `{}` has duplicate field `{}`",
                        ty.key, field.path
                    )));
                }
                crate::layout::TypeKey::parse(&field.type_key).map_err(|error| {
                    Error::manifest(format!(
                        "type `{}` field `{}` type key: {}",
                        ty.key, field.path, error.message
                    ))
                })?;
            }
        }
        for descriptor in &self.descriptors {
            require_identity(
                &descriptor.identity,
                IdentityKind::Descriptor,
                &descriptor.key,
            )?;
            validate_key_into(&mut keys, IdentityKind::Descriptor, &descriptor.key)?;
            crate::layout::TypeKey::parse(&descriptor.type_key).map_err(|error| {
                Error::manifest(format!(
                    "descriptor `{}` type key: {}",
                    descriptor.key, error.message
                ))
            })?;
        }
        for instance in &self.instances {
            require_identity(&instance.identity, IdentityKind::Instance, &instance.key)?;
            validate_key_into(&mut keys, IdentityKind::Instance, &instance.key)?;
        }
        for task in &self.tasks {
            require_identity(&task.identity, IdentityKind::Task, &task.key)?;
            validate_key_into(&mut keys, IdentityKind::Task, &task.key)?;
        }
        for export in &self.exports {
            require_identity(&export.identity, IdentityKind::Export, &export.key)?;
            validate_key_into(&mut keys, IdentityKind::Export, &export.key)?;
            crate::layout::Signature::parse(&export.signature_key).map_err(|error| {
                Error::manifest(format!(
                    "export `{}` signature: {}",
                    export.key, error.message
                ))
            })?;
        }
        Ok(())
    }
}

/// Returns the identity kind used by a root shape.
#[must_use]
pub const fn root_kind_identity(kind: RootKind) -> IdentityKind {
    match kind {
        RootKind::Direct => IdentityKind::Direct,
        RootKind::Global | RootKind::ProgramField | RootKind::NestedField | RootKind::TaskGuard => {
            IdentityKind::DataRoot
        }
    }
}

fn require_identity(identity: &Identity, kind: IdentityKind, key: &str) -> Result<()> {
    if identity.kind != kind {
        return Err(Error::manifest(format!(
            "`{key}` has identity kind {:?} but a {:?} declaration",
            identity.kind, kind
        )));
    }
    crate::keys::validate_key(kind, key)?;
    let package = crate::keys::normalize_package(&identity.package)?;
    if package != identity.package {
        return Err(Error::manifest(format!(
            "`{key}` identity package `{}` is not normalized",
            identity.package
        )));
    }
    let Some((key_package, rest)) = key.split_once("::") else {
        return Err(Error::manifest(format!("`{key}` has no package")));
    };
    if key_package != identity.package {
        return Err(Error::manifest(format!(
            "`{key}` identity package `{}` does not match the key",
            identity.package
        )));
    }
    let consistent = match kind {
        IdentityKind::Type | IdentityKind::Instance | IdentityKind::Field => rest == identity.path,
        IdentityKind::Function => {
            rest.split_once('@').map(|(path, _)| path) == Some(identity.path.as_str())
        }
        IdentityKind::Descriptor => rest.split_once("#DESC:").is_some_and(|(path, descriptor)| {
            path == identity.path && descriptor == identity.qualifier
        }),
        IdentityKind::Task => rest
            .split_once('#')
            .is_some_and(|(path, name)| path == identity.path && name == identity.qualifier),
        IdentityKind::Export => rest.split_once('@').is_some_and(|(path, signature)| {
            path == identity.path && signature == identity.qualifier
        }),
        IdentityKind::DataRoot | IdentityKind::Direct => match rest.split_once('#') {
            Some((path, qualifier)) => {
                (path == identity.path && qualifier == identity.qualifier)
                    || (kind == IdentityKind::DataRoot
                        && identity.qualifier.is_empty()
                        && qualifier.ends_with(".GUARD")
                        && rest == identity.path)
            }
            None => rest == identity.path && identity.qualifier.is_empty(),
        },
    };
    if !consistent {
        return Err(Error::manifest(format!(
            "`{key}` does not match its identity tuple ({:?}, {}, {}, {})",
            kind, identity.package, identity.path, identity.qualifier
        )));
    }
    Ok(())
}

fn validate_key_into<'a>(
    keys: &mut std::collections::BTreeMap<IdentityKind, std::collections::BTreeSet<&'a str>>,
    kind: IdentityKind,
    key: &'a str,
) -> Result<()> {
    crate::keys::validate_key(kind, key)?;
    if keys.entry(kind).or_default().insert(key) {
        Ok(())
    } else {
        Err(Error::manifest(format!(
            "duplicate {} key `{key}`",
            kind.name()
        )))
    }
}

/// Computes an initializer fingerprint for a constant value.
///
/// The value bytes are the canonical little-endian encoding of the constant.
#[must_use]
pub fn initializer_digest(type_key: &str, value_bytes: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(b"stc:init:v1");
    hasher.update(&[0]);
    hasher.update(type_key.as_bytes());
    hasher.update(&[0]);
    hasher.update(value_bytes);
    Digest32::from_bytes(hasher.finish())
}

/// Computes an initializer fingerprint for a deterministic expression.
#[must_use]
pub fn expression_initializer_digest(fingerprint: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(b"stc:init:v1");
    hasher.update(&[0]);
    hasher.update(fingerprint);
    Digest32::from_bytes(hasher.finish())
}
