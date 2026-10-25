//! Canonical M5 type keys, layouts, LLVM representations, and root leaves.
//!
//! This is the single type/layout source for codegen, the catalog projection,
//! and retained-state metadata (D-129, D-131). Type keys follow GOT Artifacts
//! §4.1; named types are identified by their content-derived type `GotId`, so
//! layout changes remain visible to online-change classification without
//! changing the type identity.
//!
//! Layouts use natural alignment and the target pointer width. They are
//! compiler metadata only: emitted IR uses LLVM types and GEP and never
//! embeds a target datalayout, pointer size, struct offset, or alignment.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use stc_got::layout::{
    ArrayBound, TypeKey, array_layout_bytes, elementary_encoding, elementary_layout_bytes,
    elementary_layout_hash, field_list_hash, layout_hash, overlap_layout_bytes,
    reference_layout_bytes, struct_layout_bytes,
};
use stc_got::sha256::domain_hash;
use stc_got::{
    FieldLayout, GotId, Identity, IdentityKind, Layout, Target, normalize_ident, normalize_package,
};
use stc_hir::{
    ArgDirection, ArrayTy, DefId, DefKind, EnumValueHir, FieldId, HirCrate, RetainPolicy,
    StringKind, StringTy, Ty, TyId,
};
use stc_span::{FileId, Span};

use crate::CodegenError;
use crate::codes;
use crate::ir::IrType;
use crate::layout::{ScalarKind, scalar_kind};

/// Fixed place-descriptor size for `REF_TO` values (D-132).
pub const PLACE_DESCRIPTOR_SIZE: u64 = 48;

/// Fixed place-descriptor alignment.
pub const PLACE_DESCRIPTOR_ALIGN: u32 = 8;

/// Rendered LLVM type of a place descriptor.
pub const PLACE_DESCRIPTOR_LLVM: &str = "{ i32, i32, i64, i64, i64, ptr, ptr }";

/// Descriptor kind tags used by generated code and the host resolver.
pub mod place_kind {
    /// The null descriptor.
    pub const NULL: i32 = 0;
    /// A host-owned data root.
    pub const ROOT: i32 = 1;
    /// A process-image direct variable.
    pub const DIRECT: i32 = 2;
    /// Transient caller storage.
    pub const STACK: i32 = 3;
    /// An object record handle plus descriptor.
    pub const OBJECT: i32 = 4;
}

/// A byte size and natural alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorageLayout {
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes (a power of two).
    pub align: u32,
}

impl StorageLayout {
    /// Creates a layout.
    #[must_use]
    pub const fn new(size: u64, align: u32) -> Self {
        Self { size, align }
    }
}

/// Structural category of one canonical type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeCategory {
    /// An elementary scalar (including bit strings, characters, time, dates).
    Scalar(ScalarKind),
    /// An enumeration with its underlying scalar.
    Enum {
        /// Enumeration definition.
        def: DefId,
        /// Underlying scalar.
        base: ScalarKind,
    },
    /// A subrange with its underlying scalar.
    Subrange {
        /// Subrange definition.
        def: DefId,
        /// Underlying scalar.
        base: ScalarKind,
        /// Inclusive lower bound.
        lower: i128,
        /// Inclusive upper bound.
        upper: i128,
    },
    /// A named-value type with its underlying scalar.
    NamedValue {
        /// Named-value definition.
        def: DefId,
        /// Underlying scalar.
        base: ScalarKind,
    },
    /// A struct type.
    Struct {
        /// Struct definition.
        def: DefId,
        /// `OVERLAP` marker.
        overlap: bool,
    },
    /// A static or conformant array.
    Array {
        /// Element type.
        element: TyId,
        /// Dimension bounds, outermost first.
        dims: Vec<ArrayDim>,
        /// `ARRAY [*]` marker.
        conformant: bool,
    },
    /// A fixed-capacity string.
    String {
        /// Encoding category.
        kind: StringKind,
        /// Capacity in code units.
        capacity: u32,
        /// Bytes per code unit.
        unit_bytes: u8,
    },
    /// A validated reference (`REF_TO`).
    Reference {
        /// Referenced type.
        target: TyId,
    },
    /// A transient raw pointer (`STC-POINTER`).
    Pointer {
        /// Pointee type.
        target: TyId,
    },
    /// An interface value.
    Interface {
        /// Interface definition.
        def: DefId,
    },
    /// A class or function-block object.
    Object {
        /// Class/FB definition.
        def: DefId,
    },
    /// The internal type of `NULL`.
    Null,
}

/// One static array dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayDim {
    /// Inclusive lower bound.
    pub lower: i128,
    /// Inclusive upper bound.
    pub upper: i128,
    /// Element count.
    pub length: u64,
}

/// Everything codegen needs to know about one canonical type.
#[derive(Clone, Debug)]
pub struct TypeFacts {
    /// Canonical type key (GOT Artifacts §4.1).
    pub key: String,
    /// LLVM value type in SSA and calls (`BOOL` is `i1`, objects are handles).
    pub llvm_value: IrType,
    /// LLVM storage type for allocas and roots (`BOOL` is `i8`, class values
    /// are pointer-sized handles, FB values are inline instances).
    pub llvm_storage: IrType,
    /// Size and natural alignment of the value.
    pub layout: StorageLayout,
    /// Target-specific layout hash.
    pub layout_hash: GotId,
    /// Structural category.
    pub category: TypeCategory,
    /// `true` for a class value (a pointer-sized handle); `false` for every
    /// other type, including inline function-block values.
    pub class_value: bool,
}

impl TypeFacts {
    /// Returns the scalar kind for elementary, enum, subrange, and named-value
    /// types.
    #[must_use]
    pub fn scalar(&self) -> Option<ScalarKind> {
        match &self.category {
            TypeCategory::Scalar(kind) => Some(*kind),
            TypeCategory::Enum { base, .. }
            | TypeCategory::Subrange { base, .. }
            | TypeCategory::NamedValue { base, .. } => Some(*base),
            _ => None,
        }
    }

    /// Returns `true` for class/FB object values.
    #[must_use]
    pub const fn is_object(&self) -> bool {
        matches!(self.category, TypeCategory::Object { .. })
    }

    /// Returns `true` for class values (handles), `false` for inline FBs.
    #[must_use]
    pub const fn is_class(&self) -> bool {
        self.class_value
    }

    /// Returns `true` for string values.
    #[must_use]
    pub const fn is_string(&self) -> bool {
        matches!(self.category, TypeCategory::String { .. })
    }

    /// Returns `true` for reference values.
    #[must_use]
    pub const fn is_reference(&self) -> bool {
        matches!(self.category, TypeCategory::Reference { .. })
    }

    /// Returns `true` for transient raw pointer values.
    #[must_use]
    pub const fn is_pointer(&self) -> bool {
        matches!(self.category, TypeCategory::Pointer { .. })
    }

    /// Returns `true` for interface values.
    #[must_use]
    pub const fn is_interface(&self) -> bool {
        matches!(self.category, TypeCategory::Interface { .. })
    }

    /// Returns `true` when the type is a contiguous aggregate value.
    #[must_use]
    pub fn is_contiguous_aggregate(&self) -> bool {
        matches!(
            self.category,
            TypeCategory::Struct { .. }
                | TypeCategory::Array { .. }
                | TypeCategory::String { .. }
                | TypeCategory::Interface { .. }
                | TypeCategory::Reference { .. }
        )
    }

    /// Returns `true` for scalar-shaped types (elementary, enums, subranges,
    /// and named-value types).
    #[must_use]
    pub fn is_scalar_shaped(&self) -> bool {
        self.scalar().is_some()
    }
}

/// One direct field of a named type.
#[derive(Clone, Debug)]
pub struct NamedField {
    /// HIR field.
    pub field: FieldId,
    /// Normalized field path relative to the declaring type.
    pub path: String,
    /// Field type.
    pub ty: TyId,
    /// Canonical type key.
    pub key: String,
    /// Byte offset in the contiguous view.
    pub offset: u64,
    /// Field size/alignment.
    pub layout: StorageLayout,
    /// Field layout hash.
    pub layout_hash: GotId,
    /// LLVM storage type inside the contiguous view.
    pub llvm_storage: IrType,
    /// Effective retention policy of the declaration.
    pub retain: RetainPolicy,
    /// Source span.
    pub span: Span,
}

/// One vtable entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodSlot {
    /// Slot index in the deterministic vtable order.
    pub slot: u32,
    /// Method or prototype definition.
    pub def: DefId,
    /// Canonical signature key.
    pub signature_key: String,
}

/// A named user type.
#[derive(Clone, Debug)]
pub struct NamedType {
    /// Definition.
    pub def: DefId,
    /// Normalized package name.
    pub package: String,
    /// Qualified path within the package.
    pub path: String,
    /// Content-derived type identity.
    pub id: GotId,
    /// Canonical type key `T:<id>`.
    pub key: String,
    /// Structural kind.
    pub kind: NamedKind,
    /// Size and alignment of the contiguous view.
    pub layout: StorageLayout,
    /// Target-specific layout hash.
    pub layout_hash: GotId,
    /// LLVM storage type for the contiguous view.
    pub llvm_storage: IrType,
    /// LLVM value type in SSA and calls.
    pub llvm_value: IrType,
    /// Direct fields in declaration order.
    pub fields: Vec<NamedField>,
    /// `OVERLAP` marker.
    pub overlap: bool,
    /// Base class/FB.
    pub base: Option<DefId>,
    /// Implemented/extended interfaces.
    pub interfaces: Vec<DefId>,
    /// Deterministic vtable shape hash.
    pub vtable_shape: Option<GotId>,
    /// Vtable entries in slot order.
    pub vtable: Vec<MethodSlot>,
    /// Enum values in declaration order.
    pub enum_values: Vec<EnumValueHir>,
    /// Subrange bounds.
    pub subrange: Option<(i128, i128)>,
}

impl NamedType {
    /// Returns `true` for class/FB object types.
    #[must_use]
    pub const fn is_object(&self) -> bool {
        matches!(self.kind, NamedKind::FunctionBlock | NamedKind::Class)
    }

    /// Returns `true` for class types, whose values are pointer-sized handles.
    #[must_use]
    pub const fn is_class(&self) -> bool {
        matches!(self.kind, NamedKind::Class)
    }
}

/// Named type kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NamedKind {
    /// `STRUCT`.
    Struct,
    /// `FUNCTION_BLOCK`.
    FunctionBlock,
    /// `CLASS`.
    Class,
    /// `INTERFACE`.
    Interface,
    /// Enumeration.
    Enum,
    /// Named-value type.
    NamedValue,
    /// Subrange.
    Subrange,
}

impl NamedKind {
    /// Returns `true` for class/FB object types.
    #[must_use]
    pub const fn is_object(self) -> bool {
        matches!(self, Self::FunctionBlock | Self::Class)
    }

    /// Returns the GOT type kind.
    #[must_use]
    pub const fn got_kind(self) -> stc_got::TypeKind {
        match self {
            Self::Struct => stc_got::TypeKind::Struct,
            Self::FunctionBlock => stc_got::TypeKind::FunctionBlock,
            Self::Class => stc_got::TypeKind::Class,
            Self::Interface => stc_got::TypeKind::Interface,
            Self::Enum | Self::NamedValue => stc_got::TypeKind::Enum,
            Self::Subrange => stc_got::TypeKind::Subrange,
        }
    }
}

/// One segment of a persistent root path below an object or global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootSegment {
    /// A field of a struct/FB/class.
    Field(FieldId),
    /// One element of a static array dimension.
    Element {
        /// Zero-based element index within the dimension.
        index: u64,
        /// Zero-based dimension index (outermost is zero).
        dim: u32,
    },
}

/// One persistent leaf root.
#[derive(Clone, Debug)]
pub struct RootLeaf {
    /// Path from the object type or global name.
    pub segments: Vec<RootSegment>,
    /// Normalized dotted path (`FIELD.E0.SUB`).
    pub path: String,
    /// Leaf type.
    pub ty: TyId,
    /// Canonical type key.
    pub key: String,
    /// Leaf size/alignment.
    pub layout: StorageLayout,
    /// Leaf layout hash.
    pub layout_hash: GotId,
    /// Effective retention policy.
    pub retain: RetainPolicy,
    /// Set when the leaf is a class value handle: one class value or one
    /// array of class values. The class definition identifies the owned
    /// instance the handle refers to; hidden `HANDLE` roots own this storage.
    pub class_handle: Option<DefId>,
}

impl RootLeaf {
    /// Returns the field segment chain as HIR field IDs (elements excluded).
    pub fn fields(&self) -> impl Iterator<Item = FieldId> + '_ {
        self.segments.iter().filter_map(|segment| match segment {
            RootSegment::Field(field) => Some(*field),
            RootSegment::Element { .. } => None,
        })
    }

    /// Returns `true` when the leaf is a class handle value.
    #[must_use]
    pub const fn is_class_handle(&self) -> bool {
        self.class_handle.is_some()
    }
}

/// One persistent class/FB instance below a declaration.
#[derive(Clone, Debug)]
pub struct ObjectInstance {
    /// Declaration-relative path of the instance.
    pub segments: Vec<RootSegment>,
    /// Class/FB definition owning the instance state.
    pub def: DefId,
    /// `true` for a class (its handle is a separate root), `false` for an
    /// inline function block.
    pub class: bool,
    /// For a class value that is one element of an array of classes, the
    /// zero-based flat element index into the owning handle array.
    pub handle_index: Option<u64>,
    /// The instance's record leaves in base-first ordinal order.
    pub leaves: Vec<RootLeaf>,
}

/// The persistent storage tree below one declaration.
#[derive(Clone, Debug, Default)]
pub struct StoragePlan {
    /// Every value leaf, including class handle leaves.
    pub leaves: Vec<RootLeaf>,
    /// Every class/FB instance in declaration order.
    pub objects: Vec<ObjectInstance>,
}

/// The class-handle projection of a class value or an array of class values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassHandle {
    /// Class definition owning the handle target.
    pub def: DefId,
    /// `PTR_TO(T:<class>)`, or the array key with that element key.
    pub key: String,
    /// Pointer-size handle, or the pointer array layout.
    pub layout: StorageLayout,
    /// Handle (or handle-array) layout hash.
    pub layout_hash: GotId,
    /// `true` when the value is an array of handles.
    pub is_array: bool,
}

/// The runtime record shape of one class/FB type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRecord {
    /// Class/FB definition.
    pub def: DefId,
    /// Canonical `ARRAY[0..N]OF(PTR_TO(T:...))` record key.
    pub key: String,
    /// `ptr typeinfo` plus one pointer per instance leaf.
    pub layout: StorageLayout,
    /// Target-specific record layout hash.
    pub layout_hash: GotId,
    /// Number of instance leaves (`N`); the record has `N + 1` pointers.
    pub leaf_count: u32,
}

/// One entry of a type's base-first interface map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceEntry {
    /// Interface definition.
    pub def: DefId,
    /// Interface type identity.
    pub id: GotId,
}

struct Builder<'a> {
    hir: &'a HirCrate,
    target: Target,
    packages: Vec<String>,
    ids: BTreeMap<DefId, GotId>,
    named: BTreeMap<DefId, NamedType>,
    computing: BTreeSet<DefId>,
}

/// The canonical type system for one build.
pub struct TypeSystem<'a> {
    named: BTreeMap<DefId, NamedType>,
    facts: BTreeMap<TyId, TypeFacts>,
    ids: BTreeMap<DefId, GotId>,
    hir: &'a HirCrate,
    target: Target,
}

impl<'a> TypeSystem<'a> {
    /// Builds the type system for a checked HIR crate and target.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-CODEGEN-001` for malformed or unsupported type
    /// declarations and recursive value layouts.
    pub fn new(hir: &'a HirCrate, target: &Target) -> Result<Self, Vec<CodegenError>> {
        let target = target.clone();
        let make = || -> Result<Self, CodegenError> {
            let mut packages = Vec::with_capacity(hir.packages.len());
            for package in &hir.packages {
                let name = normalize_package(&package.name).map_err(|error| {
                    CodegenError::internal(format!(
                        "package name `{}` is not normalized: {}",
                        package.name, error.message
                    ))
                })?;
                packages.push(name);
            }
            let mut ids = BTreeMap::new();
            for (index, def) in hir.defs.iter().enumerate() {
                if named_kind(&def.kind).is_none() {
                    continue;
                }
                let id = DefId::new(u32::try_from(index).unwrap_or(u32::MAX));
                let package = packages
                    .get(def.package.index())
                    .cloned()
                    .ok_or_else(|| CodegenError::internal("type has an unknown package"))?;
                let path = qualified_path(hir, id)?;
                let (got_id, _) = Identity::new(IdentityKind::Type, &package, &path, "").derive();
                ids.insert(id, got_id);
            }

            let mut builder = Builder {
                hir,
                target: target.clone(),
                packages,
                ids,
                named: BTreeMap::new(),
                computing: BTreeSet::new(),
            };
            for def in builder.ids.keys().copied().collect::<Vec<_>>() {
                builder.compute_named(def)?;
            }

            let mut facts = BTreeMap::new();
            for (ty, _) in hir.types.iter() {
                match builder.build_facts(ty) {
                    Ok(value) => {
                        facts.insert(ty, value);
                    }
                    Err(error)
                        if matches!(hir.types.get(ty), Ty::Error | Ty::Generic(_))
                            || matches!(
                                hir.types.get(ty),
                                Ty::Named(def)
                                    if named_kind(&hir.defs[def.index()].kind).is_none()
                                        && !matches!(
                                            hir.defs[def.index()].kind,
                                            DefKind::Alias { .. }
                                        )
                            ) =>
                    {
                        let _ = error;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(Self {
                named: builder.named,
                facts,
                ids: builder.ids,
                hir,
                target,
            })
        };
        make().map_err(|error| vec![error])
    }

    /// Returns the target used for layout hashes.
    #[must_use]
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the facts for a type.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the cascade-suppressor or a
    /// generic standard-function category.
    pub fn facts(&self, ty: TyId) -> Result<&TypeFacts, CodegenError> {
        self.facts
            .get(&ty)
            .ok_or_else(|| CodegenError::Unsupported {
                code: codes::CODEGEN_UNSUPPORTED,
                message: format!(
                    "type `{}` has no storage representation",
                    self.hir.type_name(ty)
                ),
                span: Span::empty(FileId::new(0), 0),
            })
    }

    /// Returns the named type for a definition, resolving aliases.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the definition is not a named type.
    pub fn named(&self, def: DefId) -> Result<&NamedType, CodegenError> {
        let def = self.resolve_alias(def)?;
        self.named.get(&def).ok_or_else(|| {
            CodegenError::internal(format!(
                "`{}` has no named type layout",
                self.hir.def_name(def)
            ))
        })
    }

    /// Returns the named type identity, when the definition is a named type.
    #[must_use]
    pub fn type_id(&self, def: DefId) -> Option<GotId> {
        let mut def = def;
        loop {
            match &self.hir.defs[def.index()].kind {
                DefKind::Alias { target } => match self.hir.types.get(*target) {
                    Ty::Named(next) => def = *next,
                    _ => return None,
                },
                _ => return self.ids.get(&def).copied(),
            }
        }
    }

    /// Returns all projected named types in `DefId` order.
    pub fn named_types(&self) -> impl Iterator<Item = &NamedType> {
        self.named.values()
    }

    /// Returns the interned `Ty::Named` type for a named definition.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the arena has no such type.
    pub fn named_ty(&self, def: DefId) -> Result<TyId, CodegenError> {
        self.hir
            .types
            .iter()
            .find_map(|(ty, kind)| matches!(kind, Ty::Named(found) if *found == def).then_some(ty))
            .ok_or_else(|| CodegenError::internal("named type is not in the type arena"))
    }

    /// Enumerates persistent leaves for a named definition.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed layouts.
    pub fn root_leaves_for_def(
        &self,
        def: DefId,
        prefix: &str,
        retain: RetainPolicy,
    ) -> Result<Vec<RootLeaf>, CodegenError> {
        self.root_leaves(self.named_ty(def)?, prefix, retain)
    }

    fn resolve_alias(&self, def: DefId) -> Result<DefId, CodegenError> {
        let mut def = def;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(def) {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("type alias `{}` is recursive", self.hir.def_name(def)),
                    self.hir.defs[def.index()].span,
                ));
            }
            match &self.hir.defs[def.index()].kind {
                DefKind::Alias { target } => match self.hir.types.get(*target) {
                    Ty::Named(next) => def = *next,
                    _ => {
                        return Err(CodegenError::internal(format!(
                            "alias `{}` does not resolve to a named type",
                            self.hir.def_name(def)
                        )));
                    }
                },
                _ => return Ok(def),
            }
        }
    }

    /// Resolves a type to its structural category.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn category(&self, ty: TyId) -> Result<&TypeCategory, CodegenError> {
        Ok(&self.facts(ty)?.category)
    }

    /// Returns the canonical type key.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn key(&self, ty: TyId) -> Result<&str, CodegenError> {
        Ok(&self.facts(ty)?.key)
    }

    /// Returns the LLVM value type.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn llvm_value(&self, ty: TyId) -> Result<IrType, CodegenError> {
        Ok(self.facts(ty)?.llvm_value.clone())
    }

    /// Returns the LLVM storage type.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn llvm_storage(&self, ty: TyId) -> Result<IrType, CodegenError> {
        Ok(self.facts(ty)?.llvm_storage.clone())
    }

    /// Returns the size and alignment.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn layout(&self, ty: TyId) -> Result<StorageLayout, CodegenError> {
        Ok(self.facts(ty)?.layout)
    }

    /// Returns the target-specific layout hash.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn layout_hash(&self, ty: TyId) -> Result<GotId, CodegenError> {
        Ok(self.facts(ty)?.layout_hash)
    }

    /// Returns the scalar kind when the type is scalar-shaped.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for non-scalar types.
    pub fn scalar_kind(&self, ty: TyId, span: Span) -> Result<ScalarKind, CodegenError> {
        self.facts(ty)?
            .scalar()
            .ok_or_else(|| unsupported(ty, span, self.hir))
    }

    /// Returns `true` when a declaration of this type is one persistent root.
    ///
    /// Scalars, enums, subranges, strings, references, interfaces, `OVERLAP`
    /// structs, class values, and arrays whose element chain ends in one of
    /// those are single roots. Structs, function-block values, and arrays of
    /// aggregates recurse.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn is_single_root(&self, ty: TyId) -> Result<bool, CodegenError> {
        let mut ty = ty;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(ty) {
                return Err(CodegenError::internal("recursive single-root check"));
            }
            match &self.facts(ty)?.category {
                TypeCategory::Scalar(_)
                | TypeCategory::Enum { .. }
                | TypeCategory::Subrange { .. }
                | TypeCategory::NamedValue { .. }
                | TypeCategory::String { .. }
                | TypeCategory::Reference { .. }
                | TypeCategory::Pointer { .. }
                | TypeCategory::Interface { .. }
                | TypeCategory::Null => return Ok(true),
                TypeCategory::Struct { overlap, .. } => return Ok(*overlap),
                TypeCategory::Array { element, .. } => ty = *element,
                TypeCategory::Object { def } => return Ok(self.named(*def)?.is_class()),
            }
        }
    }

    /// Enumerates the persistent value leaves below a declaration of `ty`.
    ///
    /// A class value (including an array of class values) is one handle leaf;
    /// function-block values stay inline and recurse. The owned instance state
    /// of a class is *not* part of the value leaf set; use [`Self::storage_plan`]
    /// to enumerate instance roots.
    ///
    /// `prefix` is the normalized dotted path of the declaration itself (for
    /// example a program field name); returned paths are `prefix.segments`.
    /// `retain` is the effective retention policy of the declaration.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for the error type and generic
    /// categories.
    pub fn root_leaves(
        &self,
        ty: TyId,
        prefix: &str,
        retain: RetainPolicy,
    ) -> Result<Vec<RootLeaf>, CodegenError> {
        let mut leaves = Vec::new();
        let mut segments = Vec::new();
        self.collect_leaves(ty, retain, &mut segments, prefix, &mut leaves)?;
        Ok(leaves)
    }

    /// Enumerates the instance leaves of one class/FB type.
    ///
    /// The result is the record's leaf order: base fields first, then derived
    /// fields in declaration order. Class-typed fields contribute one handle
    /// leaf; nested function-block fields recurse.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed layouts.
    pub fn instance_leaves_for_def(
        &self,
        def: DefId,
        retain: RetainPolicy,
    ) -> Result<Vec<RootLeaf>, CodegenError> {
        let mut leaves = Vec::new();
        let mut segments = Vec::new();
        self.collect_instance_leaves(def, retain, &mut segments, "", &mut leaves)?;
        Ok(leaves)
    }

    fn collect_instance_leaves(
        &self,
        def: DefId,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        out: &mut Vec<RootLeaf>,
    ) -> Result<(), CodegenError> {
        for named in self.field_chain(def)? {
            for field in &named.fields {
                let data = self.hir.field(field.field);
                let retain = effective_retain(retain, data.retain);
                segments.push(RootSegment::Field(field.field));
                self.collect_leaves(data.ty, retain, segments, prefix, out)?;
                segments.pop();
            }
        }
        Ok(())
    }

    /// Returns the class definition named by a class value or by an array
    /// whose elements are class values.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn class_def(&self, ty: TyId) -> Result<Option<DefId>, CodegenError> {
        Ok(self.class_handle_projection(ty)?.map(|handle| handle.def))
    }

    /// Projects a class value (or an array of class values) to its handle
    /// key, layout, and layout hash.
    ///
    /// The handle key is `PTR_TO(T:<class>)`, or the array key with that
    /// element key. It is the type key the catalog uses for `HANDLE` roots and
    /// class-typed fields, while the class `TypeRecord` keeps the instance
    /// layout (GOT Artifacts §8.5.5).
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn class_handle_projection(&self, ty: TyId) -> Result<Option<ClassHandle>, CodegenError> {
        let original = ty;
        let mut current = ty;
        let mut array = false;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current) {
                return Err(CodegenError::internal("recursive class-handle projection"));
            }
            match &self.facts(current)?.category {
                TypeCategory::Array { element, .. } => {
                    array = true;
                    current = *element;
                }
                TypeCategory::Object { def } => {
                    if !self.named(*def)?.is_class() {
                        return Ok(None);
                    }
                    let facts = self.facts(original)?;
                    return Ok(Some(ClassHandle {
                        def: *def,
                        key: facts.key.clone(),
                        layout: facts.layout,
                        layout_hash: facts.layout_hash,
                        is_array: array,
                    }));
                }
                _ => return Ok(None),
            }
        }
    }

    /// Returns the owned instance layout of a class/FB value, or the value
    /// layout for every other type.
    ///
    /// A class value is a pointer-sized handle; the instance layout is the
    /// field struct owned by the declaring storage and recorded in
    /// `TypeRecord` (GOT Artifacts §8.5.5).
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn instance_layout(&self, ty: TyId) -> Result<StorageLayout, CodegenError> {
        match self.category(ty)? {
            TypeCategory::Object { def } => Ok(self.named(*def)?.layout),
            _ => Ok(self.facts(ty)?.layout),
        }
    }

    /// Returns the owned instance LLVM storage type of a class/FB value, or
    /// the value storage type for every other type.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn instance_llvm_storage(&self, ty: TyId) -> Result<IrType, CodegenError> {
        match self.category(ty)? {
            TypeCategory::Object { def } => Ok(self.named(*def)?.llvm_storage.clone()),
            _ => Ok(self.facts(ty)?.llvm_storage.clone()),
        }
    }

    /// Returns the runtime object-record shape of a class/FB type.
    ///
    /// The record is `ptr typeinfo` followed by one pointer per instance leaf;
    /// its canonical key is `ARRAY[0..N]OF(PTR_TO(T:<type>))` with `N + 1`
    /// elements (GOT Artifacts §8.5.1, §8.5.6).
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn object_record(&self, def: DefId) -> Result<ObjectRecord, CodegenError> {
        let named = self.named(def)?;
        if !named.is_object() {
            return Err(CodegenError::internal(
                "object record requested for a non-object type",
            ));
        }
        let leaf_count = self.instance_leaves_for_def(def, RetainPolicy::None)?.len();
        let count = u64::try_from(leaf_count).unwrap_or(u64::MAX);
        let pointer = u64::from(self.target.pointer_width);
        let align = u32::from(self.target.pointer_width);
        let element_key_text = format!("PTR_TO(T:{})", named.id.to_hex_plain());
        let element_key = TypeKey::parse(&element_key_text).map_err(|error| {
            CodegenError::internal(format!(
                "object record element key `{element_key_text}`: {}",
                error.message
            ))
        })?;
        let element_hash = layout_hash(
            &self.target,
            &element_key,
            &reference_layout_bytes(2, &element_key),
        );
        let key = format!("ARRAY[0..{count}]OF({element_key_text})");
        let record_key = TypeKey::parse(&key).map_err(|error| {
            CodegenError::internal(format!("object record key `{key}`: {}", error.message))
        })?;
        let layout = StorageLayout::new(pointer.saturating_mul(count.saturating_add(1)), align);
        let hash = layout_hash(
            &self.target,
            &record_key,
            &array_layout_bytes(count.saturating_add(1), pointer, &element_key, element_hash),
        );
        Ok(ObjectRecord {
            def,
            key,
            layout,
            layout_hash: hash,
            leaf_count: u32::try_from(leaf_count).unwrap_or(u32::MAX),
        })
    }

    /// Returns a class/FB type's base-first interface map.
    ///
    /// Inherited entries keep their indices; interfaces newly satisfied by the
    /// type (including interfaces reached through an interface's `EXTENDS`
    /// closure) are appended sorted by `GotId` (GOT Artifacts §8.5.2).
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed types.
    pub fn interface_map(&self, def: DefId) -> Result<Vec<InterfaceEntry>, CodegenError> {
        let named = self.named(def)?;
        let mut entries = match named.base {
            Some(base) => self.interface_map(base)?,
            None => Vec::new(),
        };
        let mut seen: BTreeSet<DefId> = entries.iter().map(|entry| entry.def).collect();
        let mut added = Vec::new();
        for interface in &named.interfaces {
            self.collect_interface_closure(*interface, &mut seen, &mut added)?;
        }
        added.sort_by_key(|(_, id)| *id);
        entries.extend(
            added
                .into_iter()
                .map(|(def, id)| InterfaceEntry { def, id }),
        );
        Ok(entries)
    }

    fn collect_interface_closure(
        &self,
        def: DefId,
        seen: &mut BTreeSet<DefId>,
        out: &mut Vec<(DefId, GotId)>,
    ) -> Result<(), CodegenError> {
        let def = self.resolve_alias(def)?;
        if !seen.insert(def) {
            return Ok(());
        }
        let named = self.named(def)?;
        if !matches!(named.kind, NamedKind::Interface) {
            return Err(CodegenError::internal(
                "an implemented type is not an interface",
            ));
        }
        out.push((def, named.id));
        let extends = named.interfaces.clone();
        for parent in extends {
            self.collect_interface_closure(parent, seen, out)?;
        }
        Ok(())
    }

    /// Returns an interface type's vtable entries in slot order.
    ///
    /// The entries are the prototypes in the interface's `EXTENDS` closure,
    /// base-first: each interface contributes its own prototypes sorted by
    /// (name, signature), and a duplicate (name, signature) is kept only at its
    /// earliest occurrence (GOT Artifacts §8.5.3). Sema assigns the slots; this
    /// returns them in that order.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for non-interface types.
    pub fn interface_vtable(&self, def: DefId) -> Result<Vec<MethodSlot>, CodegenError> {
        let named = self.named(def)?;
        if !matches!(named.kind, NamedKind::Interface) {
            return Err(CodegenError::internal(
                "interface vtable requested for a non-interface type",
            ));
        }
        Ok(named.vtable.clone())
    }

    /// Returns the signature key of a function or method definition.
    ///
    /// The interface-prototype stub function records use this signature.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for non-callables.
    pub fn callable_signature(&self, def: DefId) -> Result<String, CodegenError> {
        let (params, result) = match self.hir.defs[def.index()].kind.clone() {
            DefKind::Method { params, result, .. } | DefKind::Function { params, result, .. } => {
                (params, result)
            }
            _ => {
                return Err(CodegenError::internal(
                    "signature requested for a non-callable",
                ));
            }
        };
        let mut parameters = Vec::new();
        for param in params {
            let data = self.hir.param(param).clone();
            let mode = match data.direction {
                ArgDirection::Input => "IN",
                ArgDirection::Output => "OUT",
                ArgDirection::InOut => "INOUT",
            };
            let key = self.key(data.ty)?;
            parameters.push(format!("{mode}:{key}"));
        }
        let result = match result {
            Some(ty) => self.key(ty)?,
            None => "VOID",
        };
        Ok(format!("({})->{result}", parameters.join(",")))
    }

    /// Enumerates every persistent root and class/FB instance below a
    /// declaration.
    ///
    /// Unlike [`Self::root_leaves`], a class value contributes its handle leaf
    /// *and* the owned instance's storage: the instance's field roots, nested
    /// handles, and nested object records. This is the catalog root set.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for malformed layouts.
    pub fn storage_plan(
        &self,
        ty: TyId,
        retain: RetainPolicy,
    ) -> Result<StoragePlan, CodegenError> {
        let mut plan = StoragePlan::default();
        let mut segments = Vec::new();
        let mut owning = BTreeSet::new();
        self.collect_storage(ty, retain, &mut segments, "", &mut plan, &mut owning)?;
        Ok(plan)
    }

    /// Marks one owned class/FB instance as being stored, rejecting recursive
    /// ownership that would require infinite persistent storage.
    fn enter_owned_object(
        &self,
        def: DefId,
        owning: &mut BTreeSet<DefId>,
    ) -> Result<(), CodegenError> {
        if !owning.insert(def) {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "`{}` recursively owns an instance of itself; persistent class/FB \
                     instances need finite storage",
                    self.hir.def_name(def)
                ),
                Span::empty(FileId::new(0), 0),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_storage(
        &self,
        ty: TyId,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        plan: &mut StoragePlan,
        owning: &mut BTreeSet<DefId>,
    ) -> Result<(), CodegenError> {
        if matches!(
            self.facts(ty)?.category,
            TypeCategory::Array {
                conformant: true,
                ..
            }
        ) {
            return Err(CodegenError::internal(
                "a conformant array cannot be a persistent root",
            ));
        }
        if let Some(handle) = self.class_handle_projection(ty)? {
            plan.leaves.push(RootLeaf {
                segments: segments.clone(),
                path: segment_path(self.hir, prefix, segments),
                ty,
                key: handle.key,
                layout: handle.layout,
                layout_hash: handle.layout_hash,
                retain,
                class_handle: Some(handle.def),
            });
            match &self.facts(ty)?.category {
                TypeCategory::Object { def } => {
                    self.enter_owned_object(*def, owning)?;
                    self.push_object(*def, true, None, segments, prefix, plan)?;
                    self.collect_storage_fields(*def, retain, segments, prefix, plan, owning)?;
                    owning.remove(def);
                }
                TypeCategory::Array { element, dims, .. } => {
                    if dims.is_empty() {
                        return Err(CodegenError::internal(
                            "a class-handle array has no dimensions",
                        ));
                    }
                    self.collect_class_array(
                        *element, dims, 0, 0, retain, segments, prefix, plan, owning,
                    )?;
                }
                _ => {
                    return Err(CodegenError::internal(
                        "a class handle is not an object or array",
                    ));
                }
            }
            return Ok(());
        }
        if self.is_single_root(ty)? {
            let facts = self.facts(ty)?;
            plan.leaves.push(RootLeaf {
                segments: segments.clone(),
                path: segment_path(self.hir, prefix, segments),
                ty,
                key: facts.key.clone(),
                layout: facts.layout,
                layout_hash: facts.layout_hash,
                retain,
                class_handle: None,
            });
            return Ok(());
        }
        match &self.facts(ty)?.category {
            TypeCategory::Object { def } => {
                self.enter_owned_object(*def, owning)?;
                self.push_object(*def, false, None, segments, prefix, plan)?;
                self.collect_storage_fields(*def, retain, segments, prefix, plan, owning)?;
                owning.remove(def);
            }
            TypeCategory::Struct { def, .. } => {
                for field in &self.named(*def)?.fields {
                    let data = self.hir.field(field.field);
                    let retain = effective_retain(retain, data.retain);
                    segments.push(RootSegment::Field(field.field));
                    self.collect_storage(data.ty, retain, segments, prefix, plan, owning)?;
                    segments.pop();
                }
            }
            TypeCategory::Array {
                element,
                dims,
                conformant,
            } => {
                if *conformant {
                    return Err(CodegenError::internal(
                        "a conformant array cannot be a persistent root",
                    ));
                }
                self.collect_storage_array(
                    *element, dims, 0, retain, segments, prefix, plan, owning,
                )?;
            }
            _ => {
                let facts = self.facts(ty)?;
                plan.leaves.push(RootLeaf {
                    segments: segments.clone(),
                    path: segment_path(self.hir, prefix, segments),
                    ty,
                    key: facts.key.clone(),
                    layout: facts.layout,
                    layout_hash: facts.layout_hash,
                    retain,
                    class_handle: None,
                });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_storage_fields(
        &self,
        def: DefId,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        plan: &mut StoragePlan,
        owning: &mut BTreeSet<DefId>,
    ) -> Result<(), CodegenError> {
        for named in self.field_chain(def)? {
            for field in &named.fields {
                let data = self.hir.field(field.field);
                let retain = effective_retain(retain, data.retain);
                segments.push(RootSegment::Field(field.field));
                self.collect_storage(data.ty, retain, segments, prefix, plan, owning)?;
                segments.pop();
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_storage_array(
        &self,
        element: TyId,
        dims: &[ArrayDim],
        dim: usize,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        plan: &mut StoragePlan,
        owning: &mut BTreeSet<DefId>,
    ) -> Result<(), CodegenError> {
        if dim >= dims.len() {
            return self.collect_storage(element, retain, segments, prefix, plan, owning);
        }
        for index in 0..dims[dim].length {
            segments.push(RootSegment::Element {
                index,
                dim: u32::try_from(dim).unwrap_or(u32::MAX),
            });
            self.collect_storage_array(
                element,
                dims,
                dim + 1,
                retain,
                segments,
                prefix,
                plan,
                owning,
            )?;
            segments.pop();
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_class_array(
        &self,
        element: TyId,
        dims: &[ArrayDim],
        dim: usize,
        flat: u64,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        plan: &mut StoragePlan,
        owning: &mut BTreeSet<DefId>,
    ) -> Result<(), CodegenError> {
        if dim >= dims.len() {
            let Some(def) = self.class_def(element)? else {
                return Err(CodegenError::internal(
                    "a class-handle array has a non-class element",
                ));
            };
            self.enter_owned_object(def, owning)?;
            self.push_object(def, true, Some(flat), segments, prefix, plan)?;
            self.collect_storage_fields(def, retain, segments, prefix, plan, owning)?;
            owning.remove(&def);
            return Ok(());
        }
        for index in 0..dims[dim].length {
            segments.push(RootSegment::Element {
                index,
                dim: u32::try_from(dim).unwrap_or(u32::MAX),
            });
            let next = flat.saturating_mul(dims[dim].length).saturating_add(index);
            self.collect_class_array(
                element,
                dims,
                dim + 1,
                next,
                retain,
                segments,
                prefix,
                plan,
                owning,
            )?;
            segments.pop();
        }
        Ok(())
    }

    fn push_object(
        &self,
        def: DefId,
        class: bool,
        handle_index: Option<u64>,
        segments: &[RootSegment],
        prefix: &str,
        plan: &mut StoragePlan,
    ) -> Result<(), CodegenError> {
        let mut leaves = Vec::new();
        let mut path = segments.to_vec();
        self.collect_instance_leaves(def, RetainPolicy::None, &mut path, prefix, &mut leaves)?;
        plan.objects.push(ObjectInstance {
            segments: segments.to_vec(),
            def,
            class,
            handle_index,
            leaves,
        });
        Ok(())
    }

    fn collect_leaves(
        &self,
        ty: TyId,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        out: &mut Vec<RootLeaf>,
    ) -> Result<(), CodegenError> {
        if matches!(
            self.facts(ty)?.category,
            TypeCategory::Array {
                conformant: true,
                ..
            }
        ) {
            return Err(CodegenError::internal(
                "a conformant array cannot be a persistent root",
            ));
        }
        let handle = self.class_handle_projection(ty)?;
        if self.is_single_root(ty)? {
            let facts = self.facts(ty)?;
            out.push(RootLeaf {
                segments: segments.clone(),
                path: segment_path(self.hir, prefix, segments),
                ty,
                key: facts.key.clone(),
                layout: facts.layout,
                layout_hash: facts.layout_hash,
                retain,
                class_handle: handle.map(|handle| handle.def),
            });
            return Ok(());
        }
        match &self.facts(ty)?.category {
            TypeCategory::Struct { def, .. } | TypeCategory::Object { def } => {
                let chain = self.field_chain(*def)?;
                for named in chain {
                    for field in &named.fields {
                        let data = self.hir.field(field.field);
                        let retain = effective_retain(retain, data.retain);
                        segments.push(RootSegment::Field(field.field));
                        self.collect_leaves(data.ty, retain, segments, prefix, out)?;
                        segments.pop();
                    }
                }
            }
            TypeCategory::Array {
                element,
                dims,
                conformant,
            } => {
                if *conformant {
                    return Err(CodegenError::internal(
                        "a conformant array cannot be a persistent root",
                    ));
                }
                self.collect_array_leaves(*element, dims, 0, retain, segments, prefix, out)?;
            }
            _ => return Err(CodegenError::internal("non-single root has no recursion")),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_array_leaves(
        &self,
        element: TyId,
        dims: &[ArrayDim],
        dim: usize,
        retain: RetainPolicy,
        segments: &mut Vec<RootSegment>,
        prefix: &str,
        out: &mut Vec<RootLeaf>,
    ) -> Result<(), CodegenError> {
        if dim >= dims.len() {
            return self.collect_leaves(element, retain, segments, prefix, out);
        }
        for index in 0..dims[dim].length {
            segments.push(RootSegment::Element {
                index,
                dim: u32::try_from(dim).unwrap_or(u32::MAX),
            });
            self.collect_array_leaves(element, dims, dim + 1, retain, segments, prefix, out)?;
            segments.pop();
        }
        Ok(())
    }

    /// Returns the base-to-derived field chain (base first).
    ///
    /// # Errors
    ///
    /// Returns an internal error for a malformed base cycle.
    pub fn field_chain(&self, def: DefId) -> Result<Vec<&NamedType>, CodegenError> {
        let mut chain = Vec::new();
        let mut current = Some(self.resolve_alias(def)?);
        let mut seen = BTreeSet::new();
        while let Some(def) = current {
            if !seen.insert(def) {
                return Err(CodegenError::internal("base class cycle"));
            }
            let named = self.named(def)?;
            chain.push(named);
            current = named.base;
        }
        chain.reverse();
        Ok(chain)
    }

    /// Returns a field's offset within its declaring type's contiguous view.
    #[must_use]
    pub fn field_offset(&self, field: FieldId) -> Option<u64> {
        let data = self.hir.field(field);
        let named = self.named.get(&data.owner)?;
        named
            .fields
            .iter()
            .find(|entry| entry.field == field)
            .map(|entry| entry.offset)
    }

    /// Returns the LLVM struct indices that reach `field` from `ty`.
    ///
    /// The returned path is zero-based and includes the base-class member
    /// index for inherited fields. `OVERLAP` structs have no struct indices;
    /// callers address those by byte offset.
    ///
    /// # Errors
    ///
    /// Returns an internal error when `field` is not a member of `ty`.
    pub fn field_gep_indices(&self, ty: TyId, field: FieldId) -> Result<Vec<u32>, CodegenError> {
        let def = match self.category(ty)? {
            TypeCategory::Struct { def, .. } | TypeCategory::Object { def } => *def,
            _ => {
                return Err(CodegenError::internal(format!(
                    "field access on non-struct type `{}`",
                    self.key(ty)?
                )));
            }
        };
        if self.named(def)?.overlap {
            return Err(CodegenError::internal(
                "overlap fields are addressed by byte offset",
            ));
        }
        let owner = self.hir.field(field).owner;
        let mut indices = Vec::new();
        let mut current = def;
        loop {
            let named = self.named(current)?;
            if current == owner {
                let position = named
                    .fields
                    .iter()
                    .position(|entry| entry.field == field)
                    .ok_or_else(|| CodegenError::internal("field is not in its owner"))?;
                let index =
                    u32::try_from(position).unwrap_or(u32::MAX) + u32::from(named.base.is_some());
                indices.push(index);
                return Ok(indices);
            }
            let Some(base) = named.base else {
                return Err(CodegenError::internal(
                    "field owner is not in the base chain",
                ));
            };
            indices.push(0);
            current = base;
        }
    }

    /// Returns array element type, dimensions, and the conformant flag.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-construct error for non-array types.
    pub fn array_info(&self, ty: TyId) -> Result<(TyId, &[ArrayDim], bool), CodegenError> {
        match self.category(ty)? {
            TypeCategory::Array {
                element,
                dims,
                conformant,
            } => Ok((*element, dims, *conformant)),
            _ => Err(CodegenError::internal("type is not an array")),
        }
    }

    /// Returns the character type produced by indexing a string.
    ///
    /// # Errors
    ///
    /// Returns an internal error for non-string types.
    pub fn string_char_ty(&self, ty: TyId) -> Result<TyId, CodegenError> {
        match self.category(ty)? {
            TypeCategory::String { kind, .. } => Ok(match kind {
                StringKind::Sbcs => self.hir.types.char_ty(stc_hir::CharTy::Char),
                StringKind::Dbcs => self.hir.types.char_ty(stc_hir::CharTy::WChar),
                StringKind::Unicode => self.hir.types.char_ty(stc_hir::CharTy::UChar),
            }),
            _ => Err(CodegenError::internal("type is not a string")),
        }
    }
}

fn named_kind(kind: &DefKind) -> Option<NamedKind> {
    match kind {
        DefKind::Struct { .. } => Some(NamedKind::Struct),
        DefKind::FunctionBlock { .. } => Some(NamedKind::FunctionBlock),
        DefKind::Class { .. } => Some(NamedKind::Class),
        DefKind::Interface { .. } => Some(NamedKind::Interface),
        DefKind::Enum { .. } => Some(NamedKind::Enum),
        DefKind::NamedValue { .. } => Some(NamedKind::NamedValue),
        DefKind::Subrange { .. } => Some(NamedKind::Subrange),
        _ => None,
    }
}

/// Returns the qualified path of a definition within its package.
///
/// # Errors
///
/// Returns an unsupported-construct error when a name is not a valid
/// normalized identifier.
pub(crate) fn qualified_path(hir: &HirCrate, def: DefId) -> Result<String, CodegenError> {
    let mut segments = Vec::new();
    let mut current = Some(def);
    while let Some(id) = current {
        let data = &hir.defs[id.index()];
        if id == def {
            segments.push(
                normalize_ident(data.name_text(&hir.interner))
                    .map_err(|_| invalid_name(hir, id))?,
            );
        } else if matches!(data.kind, DefKind::Namespace { .. }) {
            if data.parent.is_some() {
                segments.push(
                    normalize_ident(data.name_text(&hir.interner))
                        .map_err(|_| invalid_name(hir, id))?,
                );
            }
        } else {
            break;
        }
        current = data.parent;
    }
    segments.reverse();
    Ok(segments.join("."))
}

fn invalid_name(hir: &HirCrate, def: DefId) -> CodegenError {
    let data = &hir.defs[def.index()];
    CodegenError::unsupported(
        codes::CODEGEN_UNSUPPORTED,
        format!(
            "`{}` is not a valid normalized identifier",
            data.name_text(&hir.interner)
        ),
        data.span,
    )
}

impl Builder<'_> {
    fn compute_named(&mut self, def: DefId) -> Result<(), CodegenError> {
        if self.named.contains_key(&def) {
            return Ok(());
        }
        if !self.computing.insert(def) {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!("type `{}` contains itself by value", self.hir.def_name(def)),
                self.hir.defs[def.index()].span,
            ));
        }
        let data = self.hir.defs[def.index()].clone();
        let kind = named_kind(&data.kind)
            .ok_or_else(|| CodegenError::internal("compute_named on a non-type definition"))?;
        let package = self.packages[data.package.index()].clone();
        let path = qualified_path(self.hir, def)?;
        let id = *self
            .ids
            .get(&def)
            .ok_or_else(|| CodegenError::internal("named type has no content identity"))?;
        let key = format!("T:{}", id.to_hex_plain());

        let named = match &data.kind {
            DefKind::Enum { base, values, .. } | DefKind::NamedValue { base, values, .. } => {
                let base_layout = self.layout_of(*base)?;
                let (llvm_value, llvm_storage) = self.llvm_pair_of(*base)?;
                let hash = self.scalar_named_hash(id, *base, base_layout)?;
                NamedType {
                    def,
                    package,
                    path,
                    id,
                    key,
                    kind,
                    layout: base_layout,
                    layout_hash: hash,
                    llvm_storage,
                    llvm_value,
                    fields: Vec::new(),
                    overlap: false,
                    base: None,
                    interfaces: Vec::new(),
                    vtable_shape: None,
                    vtable: Vec::new(),
                    enum_values: values.clone(),
                    subrange: None,
                }
            }
            DefKind::Subrange {
                base, lower, upper, ..
            } => {
                let base_layout = self.layout_of(*base)?;
                let (llvm_value, llvm_storage) = self.llvm_pair_of(*base)?;
                let hash = self.scalar_named_hash(id, *base, base_layout)?;
                NamedType {
                    def,
                    package,
                    path,
                    id,
                    key,
                    kind,
                    layout: base_layout,
                    layout_hash: hash,
                    llvm_storage,
                    llvm_value,
                    fields: Vec::new(),
                    overlap: false,
                    base: None,
                    interfaces: Vec::new(),
                    vtable_shape: None,
                    vtable: Vec::new(),
                    enum_values: Vec::new(),
                    subrange: Some((*lower, *upper)),
                }
            }
            DefKind::Interface {
                extends,
                methods,
                vtable,
            } => {
                let entries = if vtable.is_empty() {
                    methods.clone()
                } else {
                    vtable.clone()
                };
                let (vtable_shape, vtable) = self.vtable_entries(&entries)?;
                let pointer = u64::from(self.target.pointer_width);
                NamedType {
                    def,
                    package,
                    path,
                    id,
                    key,
                    kind,
                    layout: StorageLayout::new(
                        pointer.saturating_mul(2),
                        u32::from(self.target.pointer_width),
                    ),
                    layout_hash: {
                        let entries = Vec::new();
                        let hash = field_list_hash(&entries);
                        let bytes = struct_layout_bytes(
                            u32::try_from(pointer.saturating_mul(2)).unwrap_or(u32::MAX),
                            u32::from(self.target.pointer_width),
                            hash,
                        );
                        layout_hash(&self.target, &TypeKey::Named(id), &bytes)
                    },
                    llvm_storage: IrType::aggregate("{ ptr, ptr }"),
                    llvm_value: IrType::aggregate("{ ptr, ptr }"),
                    fields: Vec::new(),
                    overlap: false,
                    base: None,
                    interfaces: extends.clone(),
                    vtable_shape,
                    vtable,
                    enum_values: Vec::new(),
                    subrange: None,
                }
            }
            DefKind::Struct {
                fields: field_ids,
                overlap,
            } => {
                let (fields, layout) = self.named_fields(field_ids, *overlap, None)?;
                let llvm = struct_llvm(&fields, None);
                NamedType {
                    def,
                    package,
                    path,
                    id,
                    key,
                    kind,
                    layout,
                    layout_hash: self.record_hash(id, layout, &fields, *overlap)?,
                    llvm_storage: llvm.clone(),
                    llvm_value: llvm,
                    fields,
                    overlap: *overlap,
                    base: None,
                    interfaces: Vec::new(),
                    vtable_shape: None,
                    vtable: Vec::new(),
                    enum_values: Vec::new(),
                    subrange: None,
                }
            }
            DefKind::FunctionBlock {
                base,
                interfaces,
                fields: field_ids,
                methods,
                vtable,
                ..
            }
            | DefKind::Class {
                base,
                interfaces,
                fields: field_ids,
                methods,
                vtable,
                ..
            } => {
                if let Some(base) = base {
                    self.compute_named(*base)?;
                }
                let base_view = match base {
                    Some(base) => {
                        let base = &self.named[base];
                        Some((base.layout, base.llvm_storage.clone()))
                    }
                    None => None,
                };
                let (fields, mut layout) =
                    self.named_fields(field_ids, false, base_view.as_ref().map(|v| v.0))?;
                // An object value is a stable handle; an empty object still
                // occupies a pointer-sized record in manifestations.
                if layout.size == 0 {
                    let pointer = u64::from(self.target.pointer_width);
                    layout = StorageLayout::new(pointer, u32::from(self.target.pointer_width));
                }
                let llvm = struct_llvm(&fields, base_view.as_ref().map(|v| v.1.clone()));
                let entries = if vtable.is_empty() {
                    methods.clone()
                } else {
                    vtable.clone()
                };
                let (vtable_shape, vtable) = self.vtable_entries(&entries)?;
                NamedType {
                    def,
                    package,
                    path,
                    id,
                    key,
                    kind,
                    layout,
                    layout_hash: self.record_hash(id, layout, &fields, false)?,
                    llvm_storage: llvm,
                    llvm_value: IrType::Ptr,
                    fields,
                    overlap: false,
                    base: *base,
                    interfaces: interfaces.clone(),
                    vtable_shape,
                    vtable,
                    enum_values: Vec::new(),
                    subrange: None,
                }
            }
            _ => unreachable!("named_kind only accepts type definitions"),
        };
        self.named.insert(def, named);
        self.computing.remove(&def);
        Ok(())
    }

    fn scalar_named_hash(
        &mut self,
        id: GotId,
        base: TyId,
        layout: StorageLayout,
    ) -> Result<GotId, CodegenError> {
        let key = self.key_of(base)?;
        let type_key = TypeKey::parse(&key).map_err(|error| {
            CodegenError::internal(format!("base key `{key}`: {}", error.message))
        })?;
        let bytes = if let Some(encoding) = elementary_encoding(&type_key) {
            elementary_layout_bytes(
                Layout {
                    size: layout.size,
                    align: layout.align,
                },
                encoding,
            )
        } else {
            struct_layout_bytes(
                u32::try_from(layout.size).unwrap_or(u32::MAX),
                layout.align,
                field_list_hash(&[]),
            )
        };
        Ok(layout_hash(&self.target, &TypeKey::Named(id), &bytes))
    }

    fn record_hash(
        &self,
        id: GotId,
        layout: StorageLayout,
        fields: &[NamedField],
        overlap: bool,
    ) -> Result<GotId, CodegenError> {
        let entries: Vec<FieldLayout> = fields
            .iter()
            .map(|field| {
                let type_key = TypeKey::parse(&field.key).map_err(|error| {
                    CodegenError::internal(format!(
                        "field `{}` key `{}`: {}",
                        field.path, field.key, error.message
                    ))
                })?;
                Ok(FieldLayout {
                    path: field.path.clone(),
                    offset: field.offset,
                    size: field.layout.size,
                    align: field.layout.align,
                    type_key,
                    layout_hash: field.layout_hash,
                })
            })
            .collect::<Result<_, CodegenError>>()?;
        let fields_hash = field_list_hash(&entries);
        let bytes = if overlap {
            overlap_layout_bytes(
                u32::try_from(layout.size).unwrap_or(u32::MAX),
                layout.align,
                fields_hash,
            )
        } else {
            struct_layout_bytes(
                u32::try_from(layout.size).unwrap_or(u32::MAX),
                layout.align,
                fields_hash,
            )
        };
        Ok(layout_hash(&self.target, &TypeKey::Named(id), &bytes))
    }

    fn named_fields(
        &mut self,
        field_ids: &[FieldId],
        overlap: bool,
        base: Option<StorageLayout>,
    ) -> Result<(Vec<NamedField>, StorageLayout), CodegenError> {
        let mut fields = Vec::new();
        let (mut offset, mut align) = base.map_or((0_u64, 1_u32), |base| (base.size, base.align));
        for field in field_ids {
            let data = self.hir.field(*field).clone();
            let layout = self.layout_of(data.ty)?;
            let key = self.key_of(data.ty)?;
            let hash = self.layout_hash_of(data.ty)?;
            let llvm_storage = self.llvm_storage_of(data.ty)?;
            let path = normalize_ident(self.hir.interner.text(data.name)).map_err(|_| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!(
                        "field `{}` is not a valid identifier",
                        self.hir.interner.text(data.name)
                    ),
                    data.span,
                )
            })?;
            let at = if overlap {
                0
            } else {
                round_up(offset, layout.align)
            };
            if overlap {
                offset = offset.max(layout.size);
            } else {
                offset = at.saturating_add(layout.size);
            }
            align = align.max(layout.align);
            fields.push(NamedField {
                field: *field,
                path,
                ty: data.ty,
                key,
                offset: at,
                layout,
                layout_hash: hash,
                llvm_storage,
                retain: data.retain,
                span: data.span,
            });
        }
        let layout = StorageLayout::new(round_up(offset, align), align);
        Ok((fields, layout))
    }

    fn vtable_entries(
        &mut self,
        methods: &[DefId],
    ) -> Result<(Option<GotId>, Vec<MethodSlot>), CodegenError> {
        if methods.is_empty() {
            return Ok((None, Vec::new()));
        }
        let mut entries = Vec::with_capacity(methods.len());
        let mut text = String::new();
        for (slot, def) in methods.iter().enumerate() {
            let signature = self.signature_key(*def)?;
            let slot = u32::try_from(slot).unwrap_or(u32::MAX);
            text.push_str(&format!("{slot}:{signature}\n"));
            entries.push(MethodSlot {
                slot,
                def: *def,
                signature_key: signature,
            });
        }
        let digest = domain_hash("stc:vtable:v1", &[text.as_bytes()]);
        let mut truncated = [0_u8; 16];
        truncated.copy_from_slice(&digest[..16]);
        Ok((Some(GotId::from_bytes(truncated)), entries))
    }

    fn signature_key(&mut self, def: DefId) -> Result<String, CodegenError> {
        let DefKind::Method { params, result, .. } = self.hir.defs[def.index()].kind.clone() else {
            return Err(CodegenError::internal("vtable entry is not a method"));
        };
        let mut parameters = Vec::new();
        for param in params {
            let data = self.hir.param(param).clone();
            let mode = match data.direction {
                ArgDirection::Input => "IN",
                ArgDirection::Output => "OUT",
                ArgDirection::InOut => "INOUT",
            };
            let key = self.key_of(data.ty)?;
            parameters.push(format!("{mode}:{key}"));
        }
        let result = match result {
            Some(ty) => self.key_of(ty)?,
            None => "VOID".into(),
        };
        Ok(format!("({})->{result}", parameters.join(",")))
    }

    /// Renders a canonical structural key without computing target layout.
    ///
    /// Reference and pointer keys may name a struct that is still being laid
    /// out (a self-referential linked-list node), so they must not force the
    /// pointee's facts.
    fn structural_key(&self, ty: TyId) -> Result<String, CodegenError> {
        let span = Span::empty(FileId::new(0), 0);
        match self.hir.types.get(ty) {
            Ty::Error | Ty::Generic(_) => Err(unsupported(ty, span, self.hir)),
            Ty::Bool
            | Ty::Int(_)
            | Ty::Real(_)
            | Ty::BitString(_)
            | Ty::Time(_)
            | Ty::Date(_)
            | Ty::Char(_) => Ok(scalar_kind(self.hir, ty)
                .map_err(|error| error.with_span(span))?
                .canonical_name()),
            Ty::String(string) => Ok(format!(
                "{}[{}]",
                string_kind_name(string.kind),
                string.capacity
            )),
            Ty::Array(array) => {
                let mut key = self.structural_key(array.element)?;
                for bound in array.bounds.iter().rev() {
                    let (lower, upper) = if array.conformant {
                        (ArrayBound::Conformant, ArrayBound::Conformant)
                    } else {
                        (
                            ArrayBound::Value(bound.lower),
                            ArrayBound::Value(bound.upper),
                        )
                    };
                    key = format!("ARRAY[{lower}..{upper}]OF({key})");
                }
                Ok(key)
            }
            Ty::Ref(reference) => Ok(format!(
                "REF_TO({})",
                self.structural_key(reference.target)?
            )),
            Ty::Ptr(pointer) => Ok(format!("PTR_TO({})", self.structural_key(pointer.target)?)),
            Ty::Named(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Alias { target } => self.structural_key(*target),
                kind if named_kind(kind).is_some() => self
                    .ids
                    .get(def)
                    .map(|id| format!("T:{}", id.to_hex_plain()))
                    .ok_or_else(|| {
                        CodegenError::internal(format!(
                            "named type `{}` has no content identity",
                            self.hir.def_name(*def)
                        ))
                    }),
                _ => Err(unsupported(ty, span, self.hir)),
            },
            Ty::Null => Ok("NULL".into()),
        }
    }

    fn layout_of(&mut self, ty: TyId) -> Result<StorageLayout, CodegenError> {
        let span = Span::empty(FileId::new(0), 0);
        match self.hir.types.get(ty).clone() {
            Ty::Bool
            | Ty::Int(_)
            | Ty::Real(_)
            | Ty::BitString(_)
            | Ty::Time(_)
            | Ty::Date(_)
            | Ty::Char(_) => {
                let kind = scalar_kind(self.hir, ty).map_err(|error| error.with_span(span))?;
                Ok(StorageLayout::new(kind.size_bytes(), scalar_align(kind)))
            }
            Ty::String(string) => Ok(string_layout(string)),
            Ty::Array(array) => self.array_layout(&array),
            Ty::Ref(_) => Ok(StorageLayout::new(
                PLACE_DESCRIPTOR_SIZE,
                PLACE_DESCRIPTOR_ALIGN,
            )),
            Ty::Ptr(_) => {
                let pointer = u64::from(self.target.pointer_width);
                Ok(StorageLayout::new(
                    pointer,
                    u32::from(self.target.pointer_width),
                ))
            }
            Ty::Named(def) => {
                if matches!(self.hir.defs[def.index()].kind, DefKind::Class { .. }) {
                    // A class value is a pointer-sized handle. Its instance
                    // layout is separate storage, so a class may refer to
                    // itself by value without a layout recursion.
                    let pointer = u64::from(self.target.pointer_width);
                    return Ok(StorageLayout::new(
                        pointer,
                        u32::from(self.target.pointer_width),
                    ));
                }
                match self.hir.defs[def.index()].kind.clone() {
                    DefKind::Alias { target } => self.layout_of(target),
                    _ => {
                        self.compute_named(def)?;
                        Ok(self.named[&def].layout)
                    }
                }
            }
            Ty::Null => Ok(StorageLayout::new(0, 1)),
            Ty::Error | Ty::Generic(_) => Err(unsupported(ty, span, self.hir)),
        }
    }

    fn array_layout(&mut self, array: &ArrayTy) -> Result<StorageLayout, CodegenError> {
        if array.conformant {
            let rank = u64::try_from(array.bounds.len()).unwrap_or(u64::MAX);
            return Ok(StorageLayout::new(
                16_u64.saturating_add(16_u64.saturating_mul(rank)),
                8,
            ));
        }
        let element = self.layout_of(array.element)?;
        let mut layout = StorageLayout::new(element.size, element.align);
        for bound in array.bounds.iter().rev() {
            let length = dim_length(bound.lower, bound.upper).ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "array bound is not a positive length".into(),
                    Span::empty(FileId::new(0), 0),
                )
            })?;
            layout = StorageLayout::new(
                round_up(layout.size, layout.align).saturating_mul(length),
                layout.align,
            );
        }
        Ok(layout)
    }

    fn llvm_pair_of(&mut self, ty: TyId) -> Result<(IrType, IrType), CodegenError> {
        let facts = self.build_facts(ty)?;
        Ok((facts.llvm_value, facts.llvm_storage))
    }

    fn llvm_storage_of(&mut self, ty: TyId) -> Result<IrType, CodegenError> {
        Ok(self.llvm_pair_of(ty)?.1)
    }

    fn key_of(&mut self, ty: TyId) -> Result<String, CodegenError> {
        Ok(self.build_facts(ty)?.key)
    }

    fn layout_hash_of(&mut self, ty: TyId) -> Result<GotId, CodegenError> {
        Ok(self.build_facts(ty)?.layout_hash)
    }

    fn build_facts(&mut self, ty: TyId) -> Result<TypeFacts, CodegenError> {
        let span = Span::empty(FileId::new(0), 0);
        match self.hir.types.get(ty).clone() {
            Ty::Error | Ty::Generic(_) => Err(unsupported(ty, span, self.hir)),
            Ty::Bool => {
                let kind = ScalarKind::Bool;
                Ok(TypeFacts {
                    key: "BOOL".into(),
                    llvm_value: IrType::Int(1),
                    llvm_storage: kind.storage(),
                    layout: StorageLayout::new(1, 1),
                    layout_hash: elementary_layout_hash(&self.target, &TypeKey::Bool)
                        .ok_or_else(|| CodegenError::internal("BOOL layout hash missing"))?,
                    category: TypeCategory::Scalar(kind),
                    class_value: false,
                })
            }
            Ty::Int(_)
            | Ty::Real(_)
            | Ty::BitString(_)
            | Ty::Time(_)
            | Ty::Date(_)
            | Ty::Char(_) => {
                let kind = scalar_kind(self.hir, ty).map_err(|error| error.with_span(span))?;
                let key = kind.canonical_name();
                let type_key = TypeKey::parse(&key).map_err(|error| {
                    CodegenError::internal(format!(
                        "`{key}` is not a canonical type key: {}",
                        error.message
                    ))
                })?;
                let hash = elementary_layout_hash(&self.target, &type_key).ok_or_else(|| {
                    CodegenError::internal(format!("`{key}` has no elementary layout hash"))
                })?;
                Ok(TypeFacts {
                    key,
                    llvm_value: kind.storage(),
                    llvm_storage: kind.storage(),
                    layout: StorageLayout::new(kind.size_bytes(), scalar_align(kind)),
                    layout_hash: hash,
                    category: TypeCategory::Scalar(kind),
                    class_value: false,
                })
            }
            Ty::String(string) => {
                let key = format!("{}[{}]", string_kind_name(string.kind), string.capacity);
                let type_key = TypeKey::parse(&key).map_err(|error| {
                    CodegenError::internal(format!("string key `{key}`: {}", error.message))
                })?;
                let layout = string_layout(string);
                let unit = string_unit_bytes(string.kind);
                let bytes = stc_got::layout::string_layout_bytes(
                    string.capacity,
                    unit,
                    Layout {
                        size: layout.size,
                        align: layout.align,
                    },
                );
                let hash = layout_hash(&self.target, &type_key, &bytes);
                let llvm = IrType::aggregate(format!(
                    "{{ i32, [{} x i{}] }}",
                    string.capacity,
                    u16::from(unit) * 8
                ));
                Ok(TypeFacts {
                    key,
                    llvm_value: llvm.clone(),
                    llvm_storage: llvm,
                    layout,
                    layout_hash: hash,
                    category: TypeCategory::String {
                        kind: string.kind,
                        capacity: string.capacity,
                        unit_bytes: unit,
                    },
                    class_value: false,
                })
            }
            Ty::Array(array) => self.array_facts(ty, &array),
            Ty::Ref(reference) => {
                let target_key = self.structural_key(reference.target)?;
                let key = format!("REF_TO({target_key})");
                let type_key = TypeKey::parse(&key).map_err(|error| {
                    CodegenError::internal(format!("reference key `{key}`: {}", error.message))
                })?;
                let bytes = reference_layout_bytes(1, &type_key);
                let hash = layout_hash(&self.target, &type_key, &bytes);
                let llvm = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
                Ok(TypeFacts {
                    key,
                    llvm_value: llvm.clone(),
                    llvm_storage: llvm,
                    layout: StorageLayout::new(PLACE_DESCRIPTOR_SIZE, PLACE_DESCRIPTOR_ALIGN),
                    layout_hash: hash,
                    category: TypeCategory::Reference {
                        target: reference.target,
                    },
                    class_value: false,
                })
            }
            Ty::Ptr(pointer) => {
                let target_key = self.structural_key(pointer.target)?;
                let key = format!("PTR_TO({target_key})");
                let type_key = TypeKey::parse(&key).map_err(|error| {
                    CodegenError::internal(format!("pointer key `{key}`: {}", error.message))
                })?;
                let bytes = reference_layout_bytes(2, &type_key);
                let hash = layout_hash(&self.target, &type_key, &bytes);
                let layout = StorageLayout::new(
                    u64::from(self.target.pointer_width),
                    u32::from(self.target.pointer_width),
                );
                Ok(TypeFacts {
                    key,
                    llvm_value: IrType::Ptr,
                    llvm_storage: IrType::Ptr,
                    layout,
                    layout_hash: hash,
                    category: TypeCategory::Pointer {
                        target: pointer.target,
                    },
                    class_value: false,
                })
            }
            Ty::Named(def) => match self.hir.defs[def.index()].kind.clone() {
                DefKind::Alias { target } => self.build_facts(target),
                DefKind::Class { .. } => {
                    let id = *self.ids.get(&def).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "class `{}` has no content identity",
                            self.hir.def_name(def)
                        ))
                    })?;
                    let pointer = u64::from(self.target.pointer_width);
                    let align = u32::from(self.target.pointer_width);
                    let key = format!("PTR_TO(T:{})", id.to_hex_plain());
                    let type_key = TypeKey::parse(&key).map_err(|error| {
                        CodegenError::internal(format!(
                            "class handle key `{key}`: {}",
                            error.message
                        ))
                    })?;
                    let hash = layout_hash(
                        &self.target,
                        &type_key,
                        &reference_layout_bytes(2, &type_key),
                    );
                    Ok(TypeFacts {
                        key,
                        llvm_value: IrType::Ptr,
                        llvm_storage: IrType::Ptr,
                        layout: StorageLayout::new(pointer, align),
                        layout_hash: hash,
                        category: TypeCategory::Object { def },
                        class_value: true,
                    })
                }
                kind if named_kind(&kind).is_none() => Err(unsupported(ty, span, self.hir)),
                _ => {
                    self.compute_named(def)?;
                    let named = self.named[&def].clone();
                    let category = match named.kind {
                        NamedKind::Struct => TypeCategory::Struct {
                            def,
                            overlap: named.overlap,
                        },
                        NamedKind::FunctionBlock => TypeCategory::Object { def },
                        NamedKind::Interface => TypeCategory::Interface { def },
                        NamedKind::Enum => TypeCategory::Enum {
                            def,
                            base: self.named_scalar_base(def)?,
                        },
                        NamedKind::NamedValue => TypeCategory::NamedValue {
                            def,
                            base: self.named_scalar_base(def)?,
                        },
                        NamedKind::Subrange => TypeCategory::Subrange {
                            def,
                            base: self.named_scalar_base(def)?,
                            lower: named.subrange.map(|value| value.0).unwrap_or(0),
                            upper: named.subrange.map(|value| value.1).unwrap_or(0),
                        },
                        NamedKind::Class => unreachable!("classes are handled above"),
                    };
                    Ok(TypeFacts {
                        key: named.key.clone(),
                        llvm_value: named.llvm_value.clone(),
                        llvm_storage: named.llvm_storage.clone(),
                        layout: named.layout,
                        layout_hash: named.layout_hash,
                        category,
                        class_value: false,
                    })
                }
            },
            Ty::Null => Ok(TypeFacts {
                key: "NULL".into(),
                llvm_value: IrType::Ptr,
                llvm_storage: IrType::Ptr,
                layout: StorageLayout::new(0, 1),
                layout_hash: GotId::from_bytes([0xFF; 16]),
                category: TypeCategory::Null,
                class_value: false,
            }),
        }
    }

    fn named_scalar_base(&self, def: DefId) -> Result<ScalarKind, CodegenError> {
        let data = self.hir.defs[def.index()].clone();
        let base_ty = match data.kind {
            DefKind::Enum { base, .. }
            | DefKind::NamedValue { base, .. }
            | DefKind::Subrange { base, .. } => base,
            _ => return Err(CodegenError::internal("named scalar base is not scalar")),
        };
        scalar_kind(self.hir, base_ty).map_err(|error| error.with_span(data.span))
    }

    fn array_facts(&mut self, _ty: TyId, array: &ArrayTy) -> Result<TypeFacts, CodegenError> {
        let span = Span::empty(FileId::new(0), 0);
        if array.conformant {
            let rank = array.bounds.len();
            let mut key = self.key_of(array.element)?;
            for _ in array.bounds.iter().rev() {
                key = format!(
                    "ARRAY[{}..{}]OF({key})",
                    ArrayBound::Conformant,
                    ArrayBound::Conformant
                );
            }
            let type_key = TypeKey::parse(&key).map_err(|error| {
                CodegenError::internal(format!("conformant key `{key}`: {}", error.message))
            })?;
            let layout =
                StorageLayout::new(16_u64.saturating_add(16_u64.saturating_mul(rank as u64)), 8);
            let bytes = struct_layout_bytes(
                u32::try_from(layout.size).unwrap_or(u32::MAX),
                8,
                field_list_hash(&[]),
            );
            let hash = layout_hash(&self.target, &type_key, &bytes);
            let llvm = IrType::aggregate(format!("{{ ptr, i64, [{} x {{ i64, i64 }}] }}", rank));
            return Ok(TypeFacts {
                key,
                llvm_value: llvm.clone(),
                llvm_storage: llvm,
                layout,
                layout_hash: hash,
                category: TypeCategory::Array {
                    element: array.element,
                    dims: array
                        .bounds
                        .iter()
                        .map(|bound| ArrayDim {
                            lower: bound.lower,
                            upper: bound.upper,
                            length: 0,
                        })
                        .collect(),
                    conformant: true,
                },
                class_value: false,
            });
        }
        let element_layout = self.layout_of(array.element)?;
        let element_key = self.key_of(array.element)?;
        let element_hash = self.layout_hash_of(array.element)?;
        let element_llvm = self.llvm_storage_of(array.element)?;
        let mut dims = Vec::new();
        let mut key = element_key.clone();
        let mut total: u64 = 1;
        let mut llvm = element_llvm.clone();
        let mut shape = StorageLayout::new(element_layout.size, element_layout.align);
        for bound in array.bounds.iter().rev() {
            let length = dim_length(bound.lower, bound.upper).ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "array bound is not a positive length".into(),
                    span,
                )
            })?;
            dims.push(ArrayDim {
                lower: bound.lower,
                upper: bound.upper,
                length,
            });
            key = format!("ARRAY[{}..{}]OF({key})", bound.lower, bound.upper);
            total = total.saturating_mul(length);
            llvm = IrType::aggregate(format!("[{} x {}]", length, llvm.render()));
            shape = StorageLayout::new(
                round_up(shape.size, shape.align).saturating_mul(length),
                shape.align,
            );
        }
        dims.reverse();
        let stride = round_up(element_layout.size, element_layout.align);
        let type_key = TypeKey::parse(&key).map_err(|error| {
            CodegenError::internal(format!("array key `{key}`: {}", error.message))
        })?;
        let element_type_key = TypeKey::parse(&element_key).map_err(|error| {
            CodegenError::internal(format!("element key `{element_key}`: {}", error.message))
        })?;
        let bytes = array_layout_bytes(total, stride, &element_type_key, element_hash);
        let hash = layout_hash(&self.target, &type_key, &bytes);
        Ok(TypeFacts {
            key,
            llvm_value: llvm.clone(),
            llvm_storage: llvm,
            layout: shape,
            layout_hash: hash,
            category: TypeCategory::Array {
                element: array.element,
                dims,
                conformant: false,
            },
            class_value: false,
        })
    }
}

fn string_layout(string: StringTy) -> StorageLayout {
    let unit = u64::from(string_unit_bytes(string.kind));
    let data = unit.saturating_mul(u64::from(string.capacity));
    StorageLayout::new(round_up(4_u64.saturating_add(data), 4), 4)
}

fn string_unit_bytes(kind: StringKind) -> u8 {
    match kind {
        StringKind::Sbcs => 1,
        StringKind::Dbcs => 2,
        StringKind::Unicode => 4,
    }
}

fn string_kind_name(kind: StringKind) -> &'static str {
    match kind {
        StringKind::Sbcs => "STRING",
        StringKind::Dbcs => "WSTRING",
        StringKind::Unicode => "USTRING",
    }
}

fn scalar_align(kind: ScalarKind) -> u32 {
    u32::try_from(kind.size_bytes()).unwrap_or(u32::MAX)
}

pub(crate) fn round_up(value: u64, align: u32) -> u64 {
    let align = u64::from(align.max(1));
    value.div_ceil(align).saturating_mul(align)
}

fn dim_length(lower: i128, upper: i128) -> Option<u64> {
    let length = upper.checked_sub(lower)?.checked_add(1)?;
    u64::try_from(length).ok().filter(|length| *length > 0)
}

fn struct_llvm(fields: &[NamedField], base: Option<IrType>) -> IrType {
    let mut parts = Vec::new();
    if let Some(base) = base {
        parts.push(base.render());
    }
    for field in fields {
        parts.push(field.llvm_storage.render());
    }
    if parts.is_empty() {
        return IrType::aggregate("{}");
    }
    IrType::aggregate(format!("{{ {} }}", parts.join(", ")))
}

fn segment_path(hir: &HirCrate, prefix: &str, segments: &[RootSegment]) -> String {
    let mut out = String::from(prefix);
    for segment in segments {
        out.push('.');
        match segment {
            RootSegment::Field(field) => {
                out.push_str(hir.interner.text(hir.field(*field).name));
            }
            RootSegment::Element { index, .. } => {
                out.push('E');
                out.push_str(&index.to_string());
            }
        }
    }
    out
}

fn effective_retain(parent: RetainPolicy, child: RetainPolicy) -> RetainPolicy {
    match child {
        RetainPolicy::None => parent,
        other => other,
    }
}

fn unsupported(ty: TyId, span: Span, hir: &HirCrate) -> CodegenError {
    CodegenError::unsupported(
        codes::CODEGEN_UNSUPPORTED,
        format!(
            "type `{}` has no M5 storage representation",
            hir.type_name(ty)
        ),
        span,
    )
}

/// Names of the standard function blocks delivered by the prelude.
pub const STANDARD_FB_NAMES: [&str; 10] = [
    "TON", "TOF", "TP", "CTU", "CTD", "CTUD", "R_TRIG", "F_TRIG", "SR", "RS",
];

/// Returns the standard FB name for a synthetic prelude definition.
#[must_use]
pub fn standard_fb_name(hir: &HirCrate, def: DefId) -> Option<&'static str> {
    let data = &hir.defs[def.index()];
    if data.span.start != 0 || data.span.end != 0 {
        return None;
    }
    if !matches!(data.kind, DefKind::FunctionBlock { .. }) {
        return None;
    }
    let name = data.name_text(&hir.interner).to_ascii_uppercase();
    STANDARD_FB_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
}

/// Collects the standard FB types instantiated anywhere in the crate.
#[must_use]
pub fn used_standard_fbs(hir: &HirCrate) -> BTreeSet<DefId> {
    fn scan(hir: &HirCrate, ty: TyId, out: &mut BTreeSet<DefId>, visited: &mut BTreeSet<DefId>) {
        match hir.types.get(ty) {
            Ty::Array(array) => scan(hir, array.element, out, visited),
            Ty::Ref(reference) => scan(hir, reference.target, out, visited),
            Ty::Ptr(pointer) => scan(hir, pointer.target, out, visited),
            Ty::Named(def) => {
                if standard_fb_name(hir, *def).is_some() {
                    out.insert(*def);
                    return;
                }
                if !visited.insert(*def) {
                    return;
                }
                match &hir.defs[def.index()].kind {
                    DefKind::Alias { target } => scan(hir, *target, out, visited),
                    kind => {
                        for field in kind_fields(kind) {
                            scan(hir, hir.field(*field).ty, out, visited);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn kind_fields(kind: &DefKind) -> &[FieldId] {
        match kind {
            DefKind::Struct { fields, .. }
            | DefKind::FunctionBlock { fields, .. }
            | DefKind::Class { fields, .. }
            | DefKind::Program { fields, .. } => fields,
            _ => &[],
        }
    }

    let mut out = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for field in &hir.fields {
        scan(hir, field.ty, &mut out, &mut visited);
    }
    for local in &hir.locals {
        scan(hir, local.ty, &mut out, &mut visited);
    }
    for param in &hir.params {
        scan(hir, param.ty, &mut out, &mut visited);
    }
    for def in &hir.defs {
        match &def.kind {
            DefKind::Global { ty, .. } => scan(hir, *ty, &mut out, &mut visited),
            DefKind::Function { result, .. } | DefKind::Method { result, .. } => {
                if let Some(result) = result {
                    scan(hir, *result, &mut out, &mut visited);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_diag::DiagnosticSink;
    use stc_lexer::tokenize_bytes;
    use stc_parser::{ParseOptions, parse};
    use stc_sema::analyze_files;

    fn analyze_source(source: &str) -> stc_sema::Analysis {
        let mut sink = DiagnosticSink::unlimited();
        let file = FileId::new(0);
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        let analysis = analyze_files(&[parsed], &mut sink);
        assert!(
            !sink.iter().any(|diagnostic| diagnostic.is_error()),
            "{:#?}",
            sink.diagnostics()
        );
        analysis
    }

    fn type_system(hir: &HirCrate) -> TypeSystem<'_> {
        let target = Target::new("x86_64-unknown-linux-gnu", stc_got::Endianness::Little, 8);
        TypeSystem::new(hir, &target).expect("type system")
    }

    fn named_def(hir: &HirCrate, name: &str) -> DefId {
        hir.defs
            .iter()
            .enumerate()
            .find_map(|(index, def)| {
                (def.name_text(&hir.interner).eq_ignore_ascii_case(name))
                    .then_some(DefId::new(u32::try_from(index).unwrap()))
            })
            .expect("named definition")
    }

    fn field_id(hir: &HirCrate, owner: DefId, name: &str) -> FieldId {
        hir.defs[owner.index()]
            .field_ids()
            .iter()
            .copied()
            .find(|field| {
                hir.interner
                    .text(hir.field(*field).name)
                    .eq_ignore_ascii_case(name)
            })
            .expect("field")
    }

    const AGGREGATES: &str = r#"
TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : BOOL;
  END_STRUCT;
END_TYPE
PROGRAM P
VAR
  c : Color;
  s : Small;
  p : Point;
  arr : ARRAY[1..3] OF DINT;
  ps : ARRAY[1..2] OF Point;
  str : STRING[10];
  r : REF_TO DINT;
END_VAR
END_PROGRAM
"#;

    #[test]
    fn aggregate_layouts_follow_natural_alignment() {
        let analysis = analyze_source(AGGREGATES);
        let hir = &analysis.hir;
        let system = type_system(hir);
        let point = system.named(named_def(hir, "Point")).unwrap();
        assert_eq!(point.layout, StorageLayout::new(8, 4));
        assert_eq!(point.fields.len(), 2);
        assert_eq!(point.fields[0].offset, 0);
        assert_eq!(point.fields[1].offset, 4);
        assert_eq!(point.llvm_storage.render(), "{ i32, i8 }");
        assert!(point.key.starts_with("T:"));
        TypeKey::parse(&point.key).expect("named key");

        let color = system.named(named_def(hir, "Color")).unwrap();
        assert_eq!(color.layout, StorageLayout::new(4, 4));
        assert_eq!(color.enum_values.len(), 3);
        assert_eq!(color.llvm_storage.render(), "i32");

        let small = system.named(named_def(hir, "Small")).unwrap();
        assert_eq!(small.layout, StorageLayout::new(2, 2));
        assert_eq!(small.subrange, Some((0, 10)));
    }

    #[test]
    fn persistent_leaves_recurse_per_field_and_element() {
        let analysis = analyze_source(AGGREGATES);
        let hir = &analysis.hir;
        let system = type_system(hir);
        let program = named_def(hir, "P");
        let ty = |name: &str| hir.field(field_id(hir, program, name)).ty;

        let point_leaves = system
            .root_leaves(ty("p"), "P", RetainPolicy::None)
            .unwrap();
        assert_eq!(point_leaves.len(), 2);
        assert_eq!(point_leaves[0].path, "P.x");
        assert_eq!(point_leaves[1].path, "P.y");

        let array_leaves = system
            .root_leaves(ty("arr"), "ARR", RetainPolicy::None)
            .unwrap();
        assert_eq!(array_leaves.len(), 1);
        assert_eq!(array_leaves[0].layout, StorageLayout::new(12, 4));

        let struct_array = system
            .root_leaves(ty("ps"), "PS", RetainPolicy::None)
            .unwrap();
        assert_eq!(struct_array.len(), 4);
        let paths: Vec<&str> = struct_array.iter().map(|leaf| leaf.path.as_str()).collect();
        assert_eq!(paths, ["PS.E0.x", "PS.E0.y", "PS.E1.x", "PS.E1.y"]);
        assert_eq!(struct_array[0].layout, StorageLayout::new(4, 4));
        assert_eq!(struct_array[1].layout, StorageLayout::new(1, 1));

        let string_leaves = system
            .root_leaves(ty("str"), "STR", RetainPolicy::None)
            .unwrap();
        assert_eq!(string_leaves.len(), 1);
        assert_eq!(string_leaves[0].layout, StorageLayout::new(16, 4));
        assert_eq!(string_leaves[0].key, "STRING[10]");

        let reference_leaves = system
            .root_leaves(ty("r"), "R", RetainPolicy::None)
            .unwrap();
        assert_eq!(reference_leaves.len(), 1);
        assert_eq!(reference_leaves[0].layout.size, PLACE_DESCRIPTOR_SIZE);
        assert_eq!(reference_leaves[0].key, "REF_TO(DINT)");
    }

    #[test]
    fn inherited_fields_precede_derived_fields() {
        let analysis = analyze_source(
            r#"
CLASS Base
VAR PUBLIC
  a : DINT;
END_VAR
END_CLASS
CLASS Derived EXTENDS Base
VAR PUBLIC
  b : BOOL;
END_VAR
END_CLASS
PROGRAM P
VAR
  d : Derived;
END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let derived = system.named(named_def(hir, "Derived")).unwrap();
        // The type record keeps the owned instance layout.
        assert_eq!(derived.layout, StorageLayout::new(8, 4));
        assert_eq!(derived.fields.len(), 1);
        assert_eq!(derived.fields[0].offset, 4);
        assert_eq!(derived.llvm_value, IrType::Ptr);
        // A class value is a pointer-sized handle.
        let facts = system
            .facts(system.named_ty(named_def(hir, "Derived")).unwrap())
            .unwrap();
        assert_eq!(facts.layout, StorageLayout::new(8, 8));
        assert_eq!(facts.llvm_storage, IrType::Ptr);
        assert!(facts.is_class());
        assert!(facts.key.starts_with("PTR_TO(T:"));
        assert_eq!(
            system
                .instance_layout(system.named_ty(named_def(hir, "Derived")).unwrap())
                .unwrap(),
            StorageLayout::new(8, 4)
        );
        let program = named_def(hir, "P");
        let ty = hir.field(field_id(hir, program, "d")).ty;
        let leaves = system.root_leaves(ty, "P", RetainPolicy::None).unwrap();
        assert_eq!(leaves.len(), 1, "a class value is one handle root");
        assert_eq!(leaves[0].path, "P");
        assert!(leaves[0].is_class_handle());
        let instance = system
            .instance_leaves_for_def(named_def(hir, "Derived"), RetainPolicy::None)
            .unwrap();
        assert_eq!(instance.len(), 2);
        assert_eq!(instance[0].path, ".a");
        assert_eq!(instance[1].path, ".b");
        let plan = system.storage_plan(ty, RetainPolicy::None).unwrap();
        assert_eq!(plan.leaves.len(), 3, "handle plus two instance fields");
        assert!(plan.leaves[0].is_class_handle());
        assert_eq!(plan.objects.len(), 1);
        assert_eq!(plan.objects[0].leaves.len(), 2);
        assert!(plan.objects[0].class);
    }

    #[test]
    fn class_values_and_instance_layouts_are_distinct() {
        let analysis = analyze_source(
            r#"
CLASS Inner
VAR PUBLIC
  x : DINT;
  y : BOOL;
END_VAR
END_CLASS
CLASS Outer
VAR PUBLIC
  inner : Inner;
  n : DINT;
END_VAR
END_CLASS
PROGRAM P
VAR
  o : Outer;
END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let inner = system.named(named_def(hir, "Inner")).unwrap();
        let outer = system.named(named_def(hir, "Outer")).unwrap();
        assert_eq!(outer.layout, StorageLayout::new(16, 8));
        assert_eq!(outer.llvm_storage.render(), "{ ptr, i32 }");
        let field = outer
            .fields
            .iter()
            .find(|field| field.path == "INNER")
            .expect("inner field");
        assert_eq!(field.layout, StorageLayout::new(8, 8));
        assert!(field.key.starts_with("PTR_TO(T:"));
        assert_eq!(field.llvm_storage, IrType::Ptr);
        assert!(inner.key.starts_with("T:"));

        let outer_ty = system.named_ty(named_def(hir, "Outer")).unwrap();
        let values = system
            .root_leaves(outer_ty, "O", RetainPolicy::None)
            .unwrap();
        assert_eq!(values.len(), 1, "a class value is one handle");
        assert!(values[0].is_class_handle());
        let instance = system
            .instance_leaves_for_def(named_def(hir, "Outer"), RetainPolicy::None)
            .unwrap();
        assert_eq!(instance.len(), 2);
        assert!(instance[0].is_class_handle(), "nested class field handle");
        assert!(!instance[1].is_class_handle());

        let record = system
            .object_record(named_def(hir, "Outer"))
            .expect("outer record");
        assert_eq!(record.leaf_count, 2);
        assert_eq!(record.layout, StorageLayout::new(24, 8));
        assert_eq!(
            record.key,
            format!("ARRAY[0..2]OF(PTR_TO(T:{}))", outer.id.to_hex_plain())
        );
        TypeKey::parse(&record.key).expect("record key");

        let plan = system.storage_plan(outer_ty, RetainPolicy::None).unwrap();
        // Outer handle, inner handle, inner.x, inner.y, n.
        assert_eq!(plan.leaves.len(), 5, "{:#?}", plan.leaves);
        assert_eq!(plan.objects.len(), 2, "{:#?}", plan.objects);
        assert_eq!(plan.objects[0].segments.len(), 0);
        assert_eq!(plan.objects[1].segments[0], RootSegment::Field(field.field));
        assert!(plan.objects[0].class && plan.objects[1].class);
        assert_eq!(plan.objects[0].leaves.len(), 2);
        assert_eq!(plan.objects[1].leaves.len(), 2);
    }

    #[test]
    fn arrays_of_classes_own_one_instance_and_record_per_element() {
        let analysis = analyze_source(
            r#"
CLASS Item
VAR PUBLIC
  x : DINT;
END_VAR
END_CLASS
PROGRAM P
VAR
  items : ARRAY[1..2] OF Item;
END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let program = named_def(hir, "P");
        let ty = hir.field(field_id(hir, program, "items")).ty;
        let facts = system.facts(ty).unwrap();
        assert_eq!(facts.layout, StorageLayout::new(16, 8));
        assert!(facts.key.contains("OF(PTR_TO(T:"));
        assert!(system.is_single_root(ty).unwrap());

        let plan = system.storage_plan(ty, RetainPolicy::None).unwrap();
        assert_eq!(plan.leaves.len(), 3, "one handle array plus two fields");
        assert!(plan.leaves[0].is_class_handle());
        assert_eq!(plan.leaves[0].layout, StorageLayout::new(16, 8));
        assert_eq!(plan.objects.len(), 2);
        assert_eq!(plan.objects[0].segments.len(), 1);
        assert_eq!(plan.objects[1].segments.len(), 1);
        assert_eq!(plan.objects[0].handle_index, Some(0));
        assert_eq!(plan.objects[1].handle_index, Some(1));
        assert!(plan.objects.iter().all(|object| object.leaves.len() == 1));
    }

    #[test]
    fn interface_maps_are_base_first_and_append_new_interfaces_by_id() {
        let analysis = analyze_source(
            r#"
INTERFACE IZ
METHOD Zed : DINT
END_METHOD
END_INTERFACE
INTERFACE IA
METHOD Aye : DINT
END_METHOD
END_INTERFACE
CLASS Base IMPLEMENTS IZ
METHOD PUBLIC Zed : DINT
  Zed := 1;
END_METHOD
END_CLASS
CLASS Derived EXTENDS Base IMPLEMENTS IA
METHOD PUBLIC Aye : DINT
  Aye := 2;
END_METHOD
END_CLASS
PROGRAM P
VAR d : Derived; END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let base = system.interface_map(named_def(hir, "Base")).unwrap();
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].def, named_def(hir, "IZ"));
        let derived = system.interface_map(named_def(hir, "Derived")).unwrap();
        assert_eq!(derived.len(), 2);
        assert_eq!(
            derived[0].def,
            named_def(hir, "IZ"),
            "inherited entries keep their index"
        );
        let mut new = [derived[1]];
        new.sort_by_key(|entry| entry.id);
        assert_eq!(new[0].def, named_def(hir, "IA"));
    }

    #[test]
    fn interface_vtables_are_base_first_prefix_extensions() {
        let analysis = analyze_source(
            r#"
INTERFACE IBase
METHOD Zeta : DINT
END_METHOD
METHOD Alpha : DINT
END_METHOD
END_INTERFACE
INTERFACE IDerived EXTENDS IBase
METHOD Middle : DINT
END_METHOD
END_INTERFACE
PROGRAM P
VAR i : IDerived; END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let base = system.interface_vtable(named_def(hir, "IBase")).unwrap();
        assert_eq!(base.len(), 2);
        assert_eq!(base[0].slot, 0);
        assert_eq!(base[0].def, named_def(hir, "Alpha"));
        assert_eq!(base[1].slot, 1);
        assert_eq!(base[1].def, named_def(hir, "Zeta"));

        let derived = system.interface_vtable(named_def(hir, "IDerived")).unwrap();
        assert_eq!(derived.len(), 3);
        assert_eq!(&derived[..2], &base[..], "base table is a prefix");
        assert_eq!(derived[2].slot, 2);
        assert_eq!(derived[2].def, named_def(hir, "Middle"));
    }

    #[test]
    fn class_vtables_keep_base_slots_and_append_new_methods_by_name() {
        let analysis = analyze_source(
            r#"
CLASS Base
METHOD PUBLIC Zed : DINT
  Zed := 1;
END_METHOD
METHOD PUBLIC Alpha : DINT
  Alpha := 2;
END_METHOD
END_CLASS
CLASS Derived EXTENDS Base
METHOD PUBLIC OVERRIDE Alpha : DINT
  Alpha := SUPER.Alpha() + 1;
END_METHOD
METHOD PUBLIC Middle : DINT
  Middle := 3;
END_METHOD
END_CLASS
PROGRAM P
VAR d : Derived; END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let base = system.named(named_def(hir, "Base")).unwrap();
        let base_names: Vec<&str> = base
            .vtable
            .iter()
            .map(|entry| hir.def_name(entry.def))
            .collect();
        assert_eq!(base_names, ["Alpha", "Zed"], "{:?}", base.vtable);
        let derived = system.named(named_def(hir, "Derived")).unwrap();
        assert_eq!(derived.vtable.len(), 3);
        let derived_names: Vec<&str> = derived
            .vtable
            .iter()
            .map(|entry| hir.def_name(entry.def))
            .collect();
        assert_eq!(derived_names, ["Alpha", "Zed", "Middle"]);
        assert_eq!(derived.vtable[0].slot, 0);
        assert_eq!(derived.vtable[1].slot, 1);
        assert_eq!(derived.vtable[2].slot, 2);
        assert_ne!(
            derived.vtable[0].def, base.vtable[0].def,
            "the override replaces the base entry at the same slot"
        );
    }

    #[test]
    fn interface_prototype_signatures_use_projected_class_keys() {
        let analysis = analyze_source(
            r#"
CLASS Value
VAR PUBLIC x : DINT; END_VAR
END_CLASS
INTERFACE IUse
METHOD Take : DINT
VAR_INPUT v : Value; END_VAR
END_METHOD
END_INTERFACE
PROGRAM P
VAR i : IUse; END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let prototype = system.interface_vtable(named_def(hir, "IUse")).unwrap()[0].def;
        let signature = system.callable_signature(prototype).unwrap();
        assert!(
            signature.starts_with("(IN:PTR_TO(T:") && signature.ends_with(")->DINT"),
            "{signature}"
        );
    }

    #[test]
    fn interface_values_are_two_pointers() {
        let analysis = analyze_source(
            r#"
INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Rect IMPLEMENTS IShape
VAR PUBLIC w : DINT; END_VAR
METHOD PUBLIC Area : DINT
  Area := w * w;
END_METHOD
END_CLASS
PROGRAM P
VAR s : IShape; END_VAR
END_PROGRAM
"#,
        );
        let hir = &analysis.hir;
        let system = type_system(hir);
        let iface = system.named(named_def(hir, "IShape")).unwrap();
        assert_eq!(iface.layout, StorageLayout::new(16, 8));
        assert!(iface.vtable_shape.is_some());
        assert_eq!(iface.vtable.len(), 1);
        assert_eq!(iface.vtable[0].signature_key, "()->DINT");
    }

    #[test]
    fn natural_layout_rounds_and_aligns() {
        assert_eq!(round_up(0, 4), 0);
        assert_eq!(round_up(1, 4), 4);
        assert_eq!(round_up(5, 8), 8);
        assert_eq!(round_up(9, 1), 9);
    }

    #[test]
    fn string_layout_includes_the_length_prefix() {
        let sbcs = string_layout(StringTy {
            kind: StringKind::Sbcs,
            capacity: 10,
        });
        assert_eq!(sbcs.size, 16);
        assert_eq!(sbcs.align, 4);
        let unicode = string_layout(StringTy {
            kind: StringKind::Unicode,
            capacity: 10,
        });
        assert_eq!(unicode.size, 44);
        assert_eq!(unicode.align, 4);
    }

    #[test]
    fn array_dim_lengths_are_inclusive() {
        assert_eq!(dim_length(1, 3), Some(3));
        assert_eq!(dim_length(-2, 0), Some(3));
        assert_eq!(dim_length(3, 1), None);
    }

    #[test]
    fn pointers_use_pointer_width_layout_and_the_ptr_key() {
        let analysis = analyze_source(
            "TYPE
  P : POINTER TO DINT;
  Node : STRUCT value : DINT; next : POINTER TO Node; END_STRUCT;
END_TYPE
",
        );
        let types = type_system(&analysis.hir);
        let pointer = analysis
            .hir
            .types
            .iter()
            .find_map(|(id, ty)| matches!(ty, Ty::Ptr(_)).then_some(id))
            .expect("pointer type");
        let facts = types.facts(pointer).expect("pointer facts");
        assert_eq!(facts.key, "PTR_TO(DINT)");
        assert_eq!(facts.layout, StorageLayout::new(8, 8));
        assert_eq!(facts.llvm_value, IrType::Ptr);
        assert_eq!(facts.llvm_storage, IrType::Ptr);
        assert!(facts.is_pointer());
        assert!(matches!(facts.category, TypeCategory::Pointer { .. }));

        let node = named_def(&analysis.hir, "Node");
        let named = types.named(node).expect("node layout");
        let next = named
            .fields
            .iter()
            .find(|field| field.path.eq_ignore_ascii_case("next"))
            .expect("next field");
        assert!(next.key.starts_with("PTR_TO(T:"));
        assert_eq!(next.layout, StorageLayout::new(8, 8));
    }

    #[test]
    fn type_keys_are_canonical() {
        for key in ["BOOL", "DINT", "LREAL", "STRING[10]"] {
            TypeKey::parse(key).expect(key);
        }
    }
}
