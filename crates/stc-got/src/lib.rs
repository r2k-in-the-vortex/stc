//! Persistent GOT identities, stable keys, catalog records, and manifest
//! emission/consumption for the M4.5 online-change foundation.
//!
//! The formats implemented here are normative in
//! `Specification/Architecture/GOT-Artifacts.md` (phase 1, format 1). This
//! crate is the compiler identity layer: it derives `GotId`s, matches a
//! catalog against a baseline manifest, applies tracked rename/move maps,
//! classifies preserve/allocate/drop, assigns canonical slots, and emits the
//! binary and canonical JSON manifest projections.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod descriptor;
pub mod digest;
pub mod engine;
pub mod error;
pub mod json;
pub mod keys;
pub mod layout;
pub mod manifest;
pub mod model;
mod model_json;
pub mod sha256;
pub mod wire;

pub use catalog::{
    Catalog, DescriptorDecl, ExportDecl, FieldDecl, FunctionDecl, Identity, ImportService,
    InstanceDecl, RootDecl, TaskDecl, TypeDecl, expression_initializer_digest, initializer_digest,
    root_kind_identity,
};
pub use descriptor::{
    DESCRIPTOR_FORMAT, DESCRIPTOR_MAGIC, Descriptor, DescriptorFunction, HOST_ABI_MAJOR,
    HOST_ABI_MINOR, MODULE_FORMAT, PRODUCTION_ABI_MINOR, TEST_ABI_MINOR,
};
pub use digest::{Digest32, GotId};
pub use engine::{BuildInput, BuildOptions, BuildOutcome, Rename, build, source_fingerprint};
pub use error::{BuildError, Diagnostic, Error, Result, Severity, codes};
pub use json::Json;
pub use keys::{
    HIDDEN_ROOT_MARKERS, IdentityKind, descriptor_identity_key, direct_root_key,
    export_identity_key, field_identity_key, field_root_key, function_key, global_root_key,
    hidden_root_key, hidden_root_marker, instance_identity_key, key_package, normalize_ident,
    normalize_instance_path, normalize_name_path, normalize_package, program_body_key,
    task_guard_root_key, task_identity_key, type_identity_key, validate_key,
};
pub use layout::{
    ArrayBound, Endianness, FieldLayout, Layout, ParamMode, Parameter, Signature, StringKind,
    Target, TypeKey, elementary_layout, elementary_layout_hash, layout_hash,
};
pub use manifest::{MANIFEST_FORMAT, MANIFEST_MAGIC, Manifest};
pub use model::{
    ActionRecord, Decode, DescriptorKind, DescriptorRecord, Encode, ExportRecord, FieldRecord,
    FunctionKind, FunctionRecord, Header, History, InitializerKind, InstanceRecord, Plan,
    PlanAction, PlanDiagnostic, PlanVerdict, Retention, RetiredCause, RetiredRecord, RootKind,
    RootRecord, SlotKind, SlotRecord, SourceLocation, TaskRecord, TypeKind, TypeRecord,
};
