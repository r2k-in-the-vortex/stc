//! Place resolution and aggregate materialization.
//!
//! Persistent aggregates are split across per-field host roots (D-131), so a
//! place inside one does not have a single storage pointer. This module maps a
//! resolved place to either a storage pointer (transient storage or a
//! single-root persistent value) or a materialized contiguous aggregate whose
//! leaves are copied from/to the owning roots.
//!
//! Paths are normalized into [`RootSegment`]s for slot lookup and into LLVM
//! GEP chains for the contiguous view. Dynamic indices into arrays of
//! aggregates are expanded into per-element alternatives selected at runtime.
//!
//! Some helpers are used by later M5 phases (references, conformant arrays,
//! and retained state) and are kept here with the place model.
#![allow(dead_code)]

use std::collections::BTreeMap;

use stc_hir::{
    ConstValue, DefId, DefKind, DirectAddress, ExprId, ExprKindHir, FieldId, LocalId, ParamId,
    PlaceHir, RetainPolicy, Symbol, Ty, TyId,
};
use stc_span::Span;

use crate::CodegenError;
use crate::abi::trap;
use crate::codes;
use crate::expr::{BodyInfo, FieldAccess, Res};
use crate::ir::{Function, IrType, Value};
use crate::module::RootTarget;
use crate::types::{ArrayDim, RootSegment, StorageLayout, TypeCategory};
use crate::unit::Lowerer;

/// The root of a resolved place.
#[derive(Clone, Debug)]
pub(crate) enum PlaceBase {
    /// A function local.
    Local(LocalId),
    /// A function parameter.
    Param(ParamId),
    /// The current function's result.
    Result(DefId),
    /// `THIS` in a program, FB, or method.
    This(DefId),
    /// A global declaration.
    Global(DefId),
    /// A field path inside a configured program instance.
    Instance {
        /// Program type.
        program: DefId,
        /// Configured instance.
        instance: Symbol,
        /// Resolved field chain.
        fields: Vec<FieldId>,
    },
}

/// One step of a resolved place.
#[derive(Clone, Debug)]
pub(crate) enum PlaceStep {
    /// Field access.
    Field(FieldId),
    /// One array or string subscript.
    Index(ExprId),
}

/// A resolved place path with its expression type.
#[derive(Clone, Debug)]
pub(crate) struct PlacePath {
    /// Root of the place.
    pub base: PlaceBase,
    /// Steps from the root, outermost first.
    pub steps: Vec<PlaceStep>,
    /// Type of the place expression.
    pub ty: TyId,
    /// Source span.
    pub span: Span,
}

/// A resolved storage pointer plus the direct address when it is an image
/// location.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedPointer {
    /// Storage pointer.
    pub pointer: Value,
    /// Direct address for `AT` locations.
    pub direct: Option<DirectAddress>,
}

impl Lowerer<'_> {
    /// Resolves an expression that is a place into root/steps.
    #[must_use]
    pub(crate) fn collect_place(&self, id: ExprId) -> Option<PlacePath> {
        let (base, steps) = self.walk_place(id)?;
        Some(PlacePath {
            base,
            steps,
            ty: self.hir.expr(id).ty,
            span: self.hir.expr(id).span,
        })
    }

    fn walk_place(&self, id: ExprId) -> Option<(PlaceBase, Vec<PlaceStep>)> {
        let ExprKindHir::Place(place) = &self.hir.expr(id).kind else {
            return None;
        };
        match place {
            PlaceHir::Local(local) => Some((PlaceBase::Local(*local), Vec::new())),
            PlaceHir::Param(param) => Some((PlaceBase::Param(*param), Vec::new())),
            PlaceHir::Result(def) => Some((PlaceBase::Result(*def), Vec::new())),
            PlaceHir::This(def) => Some((PlaceBase::This(*def), Vec::new())),
            PlaceHir::Global(def) => Some((PlaceBase::Global(*def), Vec::new())),
            PlaceHir::InstanceField {
                program,
                instance,
                fields,
            } => Some((
                PlaceBase::Instance {
                    program: *program,
                    instance: *instance,
                    fields: fields.clone(),
                },
                Vec::new(),
            )),
            PlaceHir::Field { base, field } => {
                let (base, mut steps) = self.walk_place(*base)?;
                steps.push(PlaceStep::Field(*field));
                Some((base, steps))
            }
            PlaceHir::Index { base, indices } => {
                let (base, mut steps) = self.walk_place(*base)?;
                for index in indices {
                    steps.push(PlaceStep::Index(*index));
                }
                Some((base, steps))
            }
            PlaceHir::Direct(_) | PlaceHir::Deref(_) | PlaceHir::BitField { .. } => None,
        }
    }

    /// Returns the concrete root segments for a place, when every index is a
    /// compile-time constant and the path stays inside static storage.
    pub(crate) fn static_segments(&self, path: &PlacePath) -> Option<Vec<RootSegment>> {
        let mut segments = Vec::new();
        let mut ty = match &path.base {
            PlaceBase::Instance { fields, .. } => {
                for field in fields {
                    segments.push(RootSegment::Field(*field));
                }
                fields.last().map(|field| self.hir.field(*field).ty)
            }
            PlaceBase::Global(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Global { ty, .. } => Some(*ty),
                _ => return None,
            },
            PlaceBase::This(_) => None,
            PlaceBase::Local(_) | PlaceBase::Param(_) | PlaceBase::Result(_) => return None,
        };
        let mut dimension = 0_usize;
        for step in &path.steps {
            match step {
                PlaceStep::Field(field) => {
                    segments.push(RootSegment::Field(*field));
                    ty = Some(self.hir.field(*field).ty);
                    dimension = 0;
                }
                PlaceStep::Index(index) => {
                    let value = self.const_int(*index)?;
                    let current = ty?;
                    let (_, dims, conformant) = self.types.array_info(current).ok()?;
                    if conformant {
                        return None;
                    }
                    let bound = *dims.get(dimension)?;
                    let delta = value.checked_sub(bound.lower)?;
                    if delta < 0 || u64::try_from(delta).ok()? >= bound.length {
                        return None;
                    }
                    segments.push(RootSegment::Element {
                        index: u64::try_from(delta).ok()?,
                        dim: u32::try_from(dimension).ok()?,
                    });
                    let (next_ty, next_dim) = self.array_advance(current, dimension).ok()?;
                    ty = Some(next_ty);
                    dimension = next_dim;
                }
            }
        }
        Some(segments)
    }

    pub(crate) fn const_int(&self, id: ExprId) -> Option<i128> {
        match &self.hir.expr(id).kind {
            ExprKindHir::Literal(literal) => match literal.value {
                ConstValue::Int(value)
                | ConstValue::Time(value)
                | ConstValue::Date { value, .. }
                | ConstValue::Enum { value, .. } => Some(value),
                ConstValue::Char(value) => Some(i128::from(value)),
                ConstValue::Bool(value) => Some(i128::from(u8::from(value))),
                _ => None,
            },
            ExprKindHir::Cast { operand, .. } => self.const_int(*operand),
            _ => None,
        }
    }

    /// Resolves a persistent single-root place to its host-root storage.
    pub(crate) fn place_root_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Option<ResolvedPointer>> {
        let Some(segments) = self.static_segments(path) else {
            return Ok(None);
        };
        if !self.types.is_single_root(path.ty)? {
            return Ok(None);
        }
        match &path.base {
            PlaceBase::This(_) | PlaceBase::Instance { .. } => {
                let Some(fields) = &info.fields else {
                    return Ok(None);
                };
                let Some(index) = fields.ordinals.get(&segments).copied() else {
                    return Ok(None);
                };
                let pointer = field_table_pointer(f, fields, index);
                let direct = fields.direct.get(&segments).cloned();
                Ok(Some(ResolvedPointer { pointer, direct }))
            }
            PlaceBase::Global(def) => {
                let target = if segments.is_empty() {
                    RootTarget::Global(*def)
                } else {
                    RootTarget::GlobalLeaf {
                        def: *def,
                        segments,
                    }
                };
                let Some(slot) = self.got.root_slot(&target) else {
                    // The place extends a single-root array; the GEP path
                    // resolves the declaration root and indexes it.
                    return Ok(None);
                };
                Ok(Some(ResolvedPointer {
                    pointer: Self::slot_storage(f, info, slot),
                    direct: self.global_address(*def),
                }))
            }
            PlaceBase::Local(_) | PlaceBase::Param(_) | PlaceBase::Result(_) => Ok(None),
        }
    }

    /// Resolves a transient place (local, parameter, or result) to storage.
    pub(crate) fn transient_place_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Option<ResolvedPointer>> {
        let (mut pointer, mut ty) = match &path.base {
            PlaceBase::Local(local) => {
                let data = self.hir.local(*local).clone();
                let Some(pointer) = info.locals.get(local).cloned() else {
                    return Ok(None);
                };
                (pointer, data.ty)
            }
            PlaceBase::Param(param) => {
                let data = self.hir.param(*param).clone();
                let Some(pointer) = info.params.get(param).cloned() else {
                    return Ok(None);
                };
                (pointer, data.ty)
            }
            PlaceBase::Result(def) => {
                let Some(pointer) = info.result_ptr.clone() else {
                    return Ok(None);
                };
                let result = match &self.hir.defs[def.index()].kind {
                    DefKind::Function { result, .. } | DefKind::Method { result, .. } => *result,
                    _ => None,
                };
                let Some(result) = result else {
                    return Ok(None);
                };
                (pointer, result)
            }
            _ => return Ok(None),
        };
        let mut dimension = 0_usize;
        let mut position = 0_usize;
        while position < path.steps.len() {
            let step = &path.steps[position];
            position += 1;
            match step {
                PlaceStep::Field(field) => {
                    dimension = 0;
                    let data = self.hir.field(*field).clone();
                    let overlap = matches!(self.types.category(ty)?, TypeCategory::Struct { .. })
                        && self.types.named(self.owner_of(ty)?)?.overlap;
                    if overlap {
                        let offset = self
                            .types
                            .field_offset(*field)
                            .ok_or_else(|| CodegenError::internal("overlap field has no offset"))?;
                        pointer = f.gep_bytes(&pointer, i64::try_from(offset).unwrap_or(i64::MAX));
                        ty = data.ty;
                        continue;
                    }
                    let indices = self.types.field_gep_indices(ty, *field)?;
                    let pointee = self.types.llvm_storage(ty)?.render();
                    let mut gep = vec!["i32 0".to_owned()];
                    for index in indices {
                        gep.push(format!("i32 {index}"));
                    }
                    pointer = f.gep(&pointee, &pointer, &gep, true);
                    ty = data.ty;
                }
                PlaceStep::Index(index) => match self.types.category(ty)?.clone() {
                    TypeCategory::Array {
                        element,
                        dims,
                        conformant,
                    } => {
                        if conformant {
                            let rank = dims.len();
                            if rank == 0 {
                                return Err(CodegenError::internal(
                                    "conformant array has no dimensions",
                                ));
                            }
                            let mut indices = Vec::with_capacity(rank);
                            indices.push(*index);
                            for _ in 1..rank {
                                let Some(PlaceStep::Index(next)) = path.steps.get(position) else {
                                    return Err(CodegenError::unsupported(
                                        codes::CODEGEN_UNSUPPORTED,
                                        format!(
                                            "a rank-{rank} conformant array requires {rank} subscripts"
                                        ),
                                        path.span,
                                    ));
                                };
                                indices.push(*next);
                                position += 1;
                            }
                            let desc_ty = self.types.llvm_storage(ty)?.render();
                            let data_ptr =
                                f.gep(&desc_ty, &pointer, &["i64 0".into(), "i32 0".into()], true);
                            let data = f.load(IrType::Ptr, &data_ptr, None);
                            let mut lower_bounds = Vec::with_capacity(rank);
                            let mut lengths = Vec::with_capacity(rank);
                            for bound in 0..rank {
                                let lower_ptr = f.gep(
                                    &desc_ty,
                                    &pointer,
                                    &[
                                        "i64 0".into(),
                                        "i32 2".into(),
                                        format!("i64 {bound}"),
                                        "i32 0".into(),
                                    ],
                                    true,
                                );
                                let lower = f.load(IrType::Int(64), &lower_ptr, None);
                                let upper_ptr = f.gep(
                                    &desc_ty,
                                    &pointer,
                                    &[
                                        "i64 0".into(),
                                        "i32 2".into(),
                                        format!("i64 {bound}"),
                                        "i32 1".into(),
                                    ],
                                    true,
                                );
                                let upper = f.load(IrType::Int(64), &upper_ptr, None);
                                let length = f.binop("sub", &upper, &lower);
                                let length = f.binop("add", &length, &Value::int(64, 1));
                                lower_bounds.push(lower);
                                lengths.push(length);
                            }
                            let mut linear = Value::int(64, 0);
                            for (bound, index_expr) in indices.iter().enumerate() {
                                let index_value = self.lower_expr(f, info, *index_expr)?;
                                let kind = self
                                    .types
                                    .facts(self.hir.expr(*index_expr).ty)?
                                    .scalar()
                                    .ok_or_else(|| {
                                        CodegenError::internal(
                                            "conformant array subscript is not an integer",
                                        )
                                    })?;
                                let wide = if kind.is_signed() {
                                    f.emit_value_cast("sext", &index_value, IrType::Int(64))
                                } else {
                                    f.emit_value_cast("zext", &index_value, IrType::Int(64))
                                };
                                let lower = &lower_bounds[bound];
                                let upper = f.binop("add", lower, &lengths[bound]);
                                let below = f.icmp("slt", &wide, lower);
                                let above = f.icmp("sge", &wide, &upper);
                                let bad = f.binop("or", &below, &above);
                                self.trap_if(f, &bad, trap::BOUNDS, path.span)?;
                                let delta = f.binop("sub", &wide, lower);
                                linear = f.binop("mul", &linear, &lengths[bound]);
                                linear = f.binop("add", &linear, &delta);
                            }
                            let element_ty = self.types.llvm_storage(element)?.render();
                            pointer =
                                f.gep(&element_ty, &data, &[format!("i64 {}", linear.repr)], false);
                            ty = element;
                            dimension = 0;
                            continue;
                        }
                        let _ = (element, dims, conformant);
                        let dim = self.array_dim_at(ty, dimension)?;
                        let delta = self.bounds_checked_index(f, info, *index, &dim, path.span)?;
                        let pointee = self.array_tail_type(ty, dimension)?;
                        pointer = f.gep(
                            &pointee,
                            &pointer,
                            &["i64 0".into(), format!("i64 {delta}")],
                            true,
                        );
                        let (next_ty, next_dim) = self.array_advance(ty, dimension)?;
                        ty = next_ty;
                        dimension = next_dim;
                    }
                    TypeCategory::String {
                        capacity,
                        unit_bytes,
                        ..
                    } => {
                        let dim = ArrayDim {
                            lower: 1,
                            upper: i128::from(capacity),
                            length: u64::from(capacity),
                        };
                        let delta = self.bounds_checked_index(f, info, *index, &dim, path.span)?;
                        let pointee = self.types.llvm_storage(ty)?.render();
                        pointer = f.gep(
                            &pointee,
                            &pointer,
                            &["i64 0".into(), "i32 1".into(), format!("i64 {delta}")],
                            true,
                        );
                        ty = self.types.string_char_ty(ty)?;
                        let _ = unit_bytes;
                    }
                    _ => {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "subscript on a non-array place".into(),
                            path.span,
                        ));
                    }
                },
            }
        }
        Ok(Some(ResolvedPointer {
            pointer,
            direct: None,
        }))
    }

    /// Returns the bound at `dim` for an array type.
    fn array_dim_at(&self, ty: TyId, dim: usize) -> Res<ArrayDim> {
        let (_, dims, conformant) = self.types.array_info(ty)?;
        if conformant {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "conformant array indexing is M5 phase 3".into(),
                Span::empty(stc_span::FileId::new(0), 0),
            ));
        }
        dims.get(dim)
            .copied()
            .ok_or_else(|| CodegenError::internal("too many array indices"))
    }

    /// Renders the LLVM type after indexing `dim` dimensions off `ty`.
    fn array_tail_type(&self, ty: TyId, dim: usize) -> Res<String> {
        let (element, dims, conformant) = self.types.array_info(ty)?;
        if conformant {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "conformant array indexing is M5 phase 3".into(),
                Span::empty(stc_span::FileId::new(0), 0),
            ));
        }
        let mut rendered = self.types.llvm_storage(element)?.render();
        for bound in dims.iter().skip(dim).rev() {
            rendered = format!("[{} x {rendered}]", bound.length);
        }
        Ok(rendered)
    }

    /// Advances past one array dimension.
    fn array_advance(&self, ty: TyId, dim: usize) -> Res<(TyId, usize)> {
        let (element, dims, _) = self.types.array_info(ty)?;
        if dim + 1 < dims.len() {
            Ok((ty, dim + 1))
        } else {
            Ok((element, 0))
        }
    }

    fn owner_of(&self, ty: TyId) -> Res<DefId> {
        match self.types.category(ty)? {
            TypeCategory::Struct { def, .. } | TypeCategory::Object { def } => Ok(*def),
            _ => Err(CodegenError::internal("type has no owning definition")),
        }
    }

    /// Resolves an index to a zero-based `i64` delta, trapping out of range.
    pub(crate) fn bounds_checked_index(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        index: ExprId,
        dim: &ArrayDim,
        span: Span,
    ) -> Res<String> {
        let value = self.lower_expr(f, info, index)?;
        let kind = self
            .types
            .facts(self.hir.expr(index).ty)?
            .scalar()
            .ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "array subscript is not an integer".into(),
                    span,
                )
            })?;
        let wide = if kind.is_signed() {
            f.emit_value_cast("sext", &value, IrType::Int(64))
        } else {
            f.emit_value_cast("zext", &value, IrType::Int(64))
        };
        let lower = Value::int(64, dim.lower);
        let upper = Value::int(64, dim.upper);
        let below = f.icmp("slt", &wide, &lower);
        let above = f.icmp("sgt", &wide, &upper);
        let bad = f.binop("or", &below, &above);
        self.trap_if(f, &bad, trap::BOUNDS, span)?;
        let delta = f.binop("sub", &wide, &lower);
        Ok(delta.repr)
    }

    /// Resolves a place to a contiguous pointer: transient storage first, then
    /// a persistent single root, then a dynamic index into a single-root array
    /// whose prefix is static.
    pub(crate) fn resolve_place_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Option<ResolvedPointer>> {
        if let Some(pointer) = self.transient_place_pointer(f, info, path)? {
            return Ok(Some(pointer));
        }
        if let Some(pointer) = self.place_root_pointer(f, info, path)? {
            return Ok(Some(pointer));
        }
        if let Some(pointer) = self.dynamic_single_root_pointer(f, info, path)? {
            return Ok(Some(pointer));
        }
        self.expanded_leaf_pointer(f, info, path)
    }

    fn dynamic_single_root_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Option<ResolvedPointer>> {
        let Some(first_index) = path
            .steps
            .iter()
            .position(|step| matches!(step, PlaceStep::Index(_)))
        else {
            return Ok(None);
        };
        if path.steps[first_index..]
            .iter()
            .any(|step| matches!(step, PlaceStep::Field(_)))
        {
            return Ok(None);
        }
        let prefix = PlacePath {
            base: path.base.clone(),
            steps: path.steps[..first_index].to_vec(),
            ty: self.type_at_steps(&path.base, &path.steps[..first_index])?,
            span: path.span,
        };
        let resolved = self.place_root_pointer(f, info, &prefix)?;
        let Some(mut resolved) = resolved else {
            return Ok(None);
        };
        let mut ty = prefix.ty;
        let mut dimension = 0_usize;
        for step in &path.steps[first_index..] {
            let PlaceStep::Index(index) = step else {
                unreachable!("field steps were rejected above");
            };
            let dim = self.array_dim_at(ty, dimension)?;
            let delta = self.bounds_checked_index(f, info, *index, &dim, path.span)?;
            let pointee = self.array_tail_type(ty, dimension)?;
            resolved.pointer = f.gep(
                &pointee,
                &resolved.pointer,
                &["i64 0".into(), format!("i64 {delta}")],
                true,
            );
            let (next_ty, next_dim) = self.array_advance(ty, dimension)?;
            ty = next_ty;
            dimension = next_dim;
        }
        Ok(Some(resolved))
    }

    /// Returns the type after applying `steps` to `base`.
    fn type_at_steps(&self, base: &PlaceBase, steps: &[PlaceStep]) -> Res<TyId> {
        let mut ty = match base {
            PlaceBase::Instance { fields, .. } => {
                fields.last().map(|field| self.hir.field(*field).ty)
            }
            PlaceBase::Global(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Global { ty, .. } => Some(*ty),
                _ => return Err(CodegenError::internal("global base is not a global")),
            },
            PlaceBase::This(_) => None,
            PlaceBase::Local(_) | PlaceBase::Param(_) | PlaceBase::Result(_) => {
                return Err(CodegenError::internal(
                    "transient base has no persistent type walk",
                ));
            }
        };
        let mut dimension = 0_usize;
        for step in steps {
            match step {
                PlaceStep::Field(field) => {
                    ty = Some(self.hir.field(*field).ty);
                    dimension = 0;
                }
                PlaceStep::Index(_) => {
                    let current = ty.ok_or_else(|| {
                        CodegenError::internal("array index precedes the first field")
                    })?;
                    let (next, next_dim) = self.array_advance(current, dimension)?;
                    ty = Some(next);
                    dimension = next_dim;
                }
            }
        }
        ty.ok_or_else(|| CodegenError::internal("place has no type"))
    }

    /// Reads an aggregate place: contiguous when possible, otherwise
    /// materializes it from the owning roots.
    pub(crate) fn read_aggregate_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Value> {
        if let Some(resolved) = self.resolve_place_pointer(f, info, path)? {
            if resolved.direct.is_some() {
                return Err(CodegenError::internal(
                    "an aggregate direct variable has no M5 representation",
                ));
            }
            let ty = self.types.llvm_storage(path.ty)?;
            return Ok(f.load(ty, &resolved.pointer, None));
        }
        if self.types.facts(path.ty)?.is_object() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "class and FB values are M5 phase 4".into(),
                path.span,
            ));
        }
        self.materialize_place(f, info, path)
    }

    /// Writes an aggregate place: contiguous when possible, otherwise splits
    /// the value back into the owning roots.
    pub(crate) fn write_aggregate_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
        value: Value,
    ) -> Res<()> {
        if let Some(resolved) = self.resolve_place_pointer(f, info, path)? {
            if resolved.direct.is_some() {
                return Err(CodegenError::internal(
                    "an aggregate direct variable has no M5 representation",
                ));
            }
            f.store(&value, &resolved.pointer, None);
            return Ok(());
        }
        if self.types.facts(path.ty)?.is_object() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "class and FB assignment is M5 phase 4".into(),
                path.span,
            ));
        }
        let storage = self.types.llvm_storage(path.ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&value, &temp, None);
        self.split_place_from_temp(f, info, path, &temp)?;
        Ok(())
    }

    fn materialize_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Value> {
        let storage = self.types.llvm_storage(path.ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        self.gather_place_into_temp(f, info, path, &temp)?;
        Ok(f.load(storage, &temp, None))
    }

    fn gather_place_into_temp(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
        temp: &Value,
    ) -> Res<()> {
        let base_segments = self.static_segments(path).ok_or_else(|| {
            CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "dynamic indices inside an aggregate value are M5 phase 2b".into(),
                path.span,
            )
        })?;
        let leaves = self
            .types
            .root_leaves(path.ty, "TEMP", RetainPolicy::None)?;
        for leaf in leaves {
            let mut segments = base_segments.clone();
            segments.extend(leaf.segments.iter().copied());
            let source =
                self.root_pointer_for_segments(f, info, &path.base, &segments, path.span)?;
            let value = self.read_leaf(f, &source, leaf.ty)?;
            let destination = self.aggregate_pointer(f, path.ty, temp, &leaf.segments)?;
            f.store(&value, &destination, None);
        }
        Ok(())
    }

    pub(crate) fn split_place_from_temp(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
        temp: &Value,
    ) -> Res<()> {
        let base_segments = self.static_segments(path).ok_or_else(|| {
            CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "dynamic indices inside an aggregate value are M5 phase 2b".into(),
                path.span,
            )
        })?;
        let leaves = self
            .types
            .root_leaves(path.ty, "TEMP", RetainPolicy::None)?;
        for leaf in leaves {
            let source = self.aggregate_pointer(f, path.ty, temp, &leaf.segments)?;
            let value = f.load(self.types.llvm_storage(leaf.ty)?, &source, None);
            let mut segments = base_segments.clone();
            segments.extend(leaf.segments.iter().copied());
            let destination =
                self.root_pointer_for_segments(f, info, &path.base, &segments, path.span)?;
            self.write_leaf(f, &destination, leaf.ty, value)?;
        }
        Ok(())
    }

    fn root_pointer_for_segments(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        base: &PlaceBase,
        segments: &[RootSegment],
        span: Span,
    ) -> Res<ResolvedPointer> {
        match base {
            PlaceBase::This(_) | PlaceBase::Instance { .. } => {
                let Some(fields) = &info.fields else {
                    return Err(CodegenError::internal(
                        "persistent place outside an instance body",
                    ));
                };
                let Some(index) = fields.ordinals.get(segments).copied() else {
                    return Err(CodegenError::internal(format!(
                        "no root slot for persistent path {segments:?}"
                    )));
                };
                Ok(ResolvedPointer {
                    pointer: field_table_pointer(f, fields, index),
                    direct: fields.direct.get(segments).cloned(),
                })
            }
            PlaceBase::Global(def) => {
                if segments.is_empty() {
                    let slot = self.root_slot(&RootTarget::Global(*def))?;
                    Ok(ResolvedPointer {
                        pointer: Self::slot_storage(f, info, slot),
                        direct: self.global_address(*def),
                    })
                } else {
                    let slot = self.root_slot(&RootTarget::GlobalLeaf {
                        def: *def,
                        segments: segments.to_vec(),
                    })?;
                    Ok(ResolvedPointer {
                        pointer: Self::slot_storage(f, info, slot),
                        direct: None,
                    })
                }
            }
            PlaceBase::Local(_) | PlaceBase::Param(_) | PlaceBase::Result(_) => {
                Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "a split aggregate needs persistent storage".into(),
                    span,
                ))
            }
        }
    }

    /// Emits a GEP chain into a contiguous aggregate for `segments`.
    pub(crate) fn aggregate_pointer(
        &mut self,
        f: &mut Function,
        mut ty: TyId,
        base: &Value,
        segments: &[RootSegment],
    ) -> Res<Value> {
        let mut pointer = base.clone();
        for segment in segments {
            match segment {
                RootSegment::Field(field) => {
                    let data = self.hir.field(*field).clone();
                    let category = self.types.category(ty)?.clone();
                    if let TypeCategory::Struct { def, .. } = category
                        && self.types.named(def)?.overlap
                    {
                        let offset = self
                            .types
                            .field_offset(*field)
                            .ok_or_else(|| CodegenError::internal("overlap field has no offset"))?;
                        pointer = f.gep_bytes(&pointer, i64::try_from(offset).unwrap_or(i64::MAX));
                        ty = data.ty;
                        continue;
                    }
                    let indices = self.types.field_gep_indices(ty, *field)?;
                    let pointee = self.types.llvm_storage(ty)?.render();
                    let mut gep = vec!["i32 0".to_owned()];
                    for index in indices {
                        gep.push(format!("i32 {index}"));
                    }
                    pointer = f.gep(&pointee, &pointer, &gep, true);
                    ty = data.ty;
                }
                RootSegment::Element { index, dim } => {
                    let dim = usize::try_from(*dim).unwrap_or(usize::MAX);
                    let pointee = self.array_tail_type(ty, dim)?;
                    pointer = f.gep(
                        &pointee,
                        &pointer,
                        &["i64 0".into(), format!("i64 {index}")],
                        true,
                    );
                    let (next_ty, _) = self.array_advance(ty, dim)?;
                    ty = next_ty;
                }
            }
        }
        Ok(pointer)
    }

    pub(crate) fn read_leaf(
        &mut self,
        f: &mut Function,
        source: &ResolvedPointer,
        ty: TyId,
    ) -> Res<Value> {
        if let Some(address) = &source.direct {
            let kind = self
                .types
                .scalar_kind(ty, Span::empty(stc_span::FileId::new(0), 0))?;
            let bit = direct_bit(address, kind)?;
            let storage = self.types.llvm_storage(ty)?;
            return Ok(if let Some(bit) = bit {
                let raw = f.load(IrType::Int(8), &source.pointer, Some(1));
                let mask = Value::int(8, 1 << bit);
                let masked = f.binop("and", &raw, &mask);
                let flag = f.icmp("ne", &masked, &Value::int(8, 0));
                f.emit_value_cast("zext", &flag, storage)
            } else {
                f.load(storage, &source.pointer, Some(1))
            });
        }
        let storage = self.types.llvm_storage(ty)?;
        Ok(f.load(storage, &source.pointer, None))
    }

    pub(crate) fn write_leaf(
        &mut self,
        f: &mut Function,
        destination: &ResolvedPointer,
        ty: TyId,
        value: Value,
    ) -> Res<()> {
        self.check_subrange(f, ty, &value, Span::empty(stc_span::FileId::new(0), 0))?;
        if let Some(address) = &destination.direct {
            let kind = self
                .types
                .scalar_kind(ty, Span::empty(stc_span::FileId::new(0), 0))?;
            let bit = direct_bit(address, kind)?;
            if let Some(bit) = bit {
                let raw = f.load(IrType::Int(8), &destination.pointer, Some(1));
                let clear = Value::int(8, !(1_i128 << bit));
                let cleared = f.binop("and", &raw, &clear);
                let mask = Value::int(8, 1 << bit);
                let condition = if value.ty == IrType::Int(1) {
                    value
                } else {
                    f.icmp("ne", &value, &Value::int(8, 0))
                };
                let set = f.select(&condition, &mask, &Value::int(8, 0));
                let combined = f.binop("or", &cleared, &set);
                f.store(&combined, &destination.pointer, Some(1));
            } else {
                let stored = if value.ty == IrType::Int(1) {
                    f.emit_value_cast("zext", &value, IrType::Int(8))
                } else {
                    value
                };
                f.store(&stored, &destination.pointer, Some(1));
            }
            return Ok(());
        }
        f.store(&value, &destination.pointer, None);
        Ok(())
    }

    /// Returns the LLVM storage type of the contiguous view for `ty`.
    pub(crate) fn contiguous_type(&self, ty: TyId) -> Res<IrType> {
        self.types.llvm_storage(ty)
    }

    /// Returns the static leaf layout of a type for callers that need sizes.
    pub(crate) fn leaf_layout(&self, ty: TyId) -> Res<StorageLayout> {
        self.types.layout(ty)
    }
}

/// Emits the `[N x ptr]` entry load for a leaf ordinal.
pub(crate) fn field_table_pointer(f: &mut Function, fields: &FieldAccess, index: u32) -> Value {
    let array_type = format!("[{} x ptr]", fields.ordinals.len());
    let entry = f.gep(
        &array_type,
        &fields.pointer,
        &["i64 0".into(), format!("i64 {index}")],
        true,
    );
    f.load(IrType::Ptr, &entry, Some(8))
}

/// Returns `true` for a direct address that names an individual bit.
pub(crate) fn is_bit_address(address: &DirectAddress) -> bool {
    matches!(address.size, Some(stc_hir::DirectSize::Bit))
        || (address.size.is_none() && address.indices.len() == 2)
}

pub(crate) fn direct_bit(
    address: &DirectAddress,
    kind: crate::layout::ScalarKind,
) -> Res<Option<u8>> {
    let span = crate::layout::image_span(address, kind)?;
    Ok((kind == crate::layout::ScalarKind::Bool)
        .then(|| u8::try_from(span.start_bit % 8).unwrap_or(0)))
}

impl Lowerer<'_> {
    /// Returns the per-instance direct-address map for a program layout.
    pub(crate) fn program_leaf_target(
        &self,
        instance: Symbol,
        segments: &[RootSegment],
    ) -> RootTarget {
        if let [RootSegment::Field(field)] = segments {
            RootTarget::InstanceField {
                instance,
                field: *field,
            }
        } else {
            RootTarget::InstanceLeaf {
                instance,
                segments: segments.to_vec(),
            }
        }
    }

    /// Returns the global root target for a leaf path.
    pub(crate) fn global_leaf_target(def: DefId, segments: &[RootSegment]) -> RootTarget {
        if segments.is_empty() {
            RootTarget::Global(def)
        } else {
            RootTarget::GlobalLeaf {
                def,
                segments: segments.to_vec(),
            }
        }
    }

    /// Indexes an object leaf layout by root path.
    pub(crate) fn object_ordinals(
        &self,
        layout: &crate::config::ObjectLayout,
    ) -> BTreeMap<Vec<RootSegment>, u32> {
        layout
            .leaves
            .iter()
            .enumerate()
            .map(|(index, leaf)| {
                (
                    leaf.segments.clone(),
                    u32::try_from(index).unwrap_or(u32::MAX),
                )
            })
            .collect()
    }

    /// Indexes the program leaf table by root path.
    pub(crate) fn leaf_ordinals(
        &self,
        layout: &crate::config::ProgramLayout,
    ) -> BTreeMap<Vec<RootSegment>, u32> {
        layout
            .leaves
            .iter()
            .enumerate()
            .map(|(index, leaf)| {
                (
                    leaf.segments.clone(),
                    u32::try_from(index).unwrap_or(u32::MAX),
                )
            })
            .collect()
    }
}

impl Lowerer<'_> {
    /// Lowers a fixed-capacity string literal into temporary storage.
    pub(crate) fn lower_string_literal(
        &mut self,
        f: &mut Function,
        ty: TyId,
        units: &[u32],
        span: Span,
    ) -> Res<Value> {
        let facts = self.types.facts(ty)?.clone();
        let TypeCategory::String {
            capacity,
            unit_bytes,
            ..
        } = facts.category
        else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "a string literal requires a string type".into(),
                span,
            ));
        };
        let storage = self.types.llvm_storage(ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        let pointee = storage.render();
        let length = i128::try_from(units.len()).unwrap_or(i128::MAX);
        if u64::try_from(units.len()).unwrap_or(u64::MAX) > u64::from(capacity) {
            self.trap_constant(f, trap::RANGE, span)?;
        }
        let len_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        f.store(&Value::int(32, length), &len_ptr, None);
        let data = f.gep(&pointee, &temp, &["i64 0".into(), "i32 1".into()], true);
        let unit_bits = u16::from(unit_bytes) * 8;
        let array_ty = format!("[{capacity} x i{unit_bits}]");
        for (index, unit) in units.iter().enumerate() {
            let slot = f.gep(
                &array_ty,
                &data,
                &["i64 0".into(), format!("i64 {index}")],
                true,
            );
            f.store(&Value::int(unit_bits, i128::from(*unit)), &slot, None);
        }
        Ok(f.load(storage, &temp, None))
    }

    /// Emits an unconditional trap through a duplicate comparison.
    fn trap_constant(&mut self, f: &mut Function, code: i32, span: Span) -> Res<()> {
        let condition = Value::bool(true);
        self.trap_if(f, &condition, code, span)
    }

    /// Builds an array initializer value.
    pub(crate) fn build_array_init(
        &mut self,
        f: &mut Function,
        ty: TyId,
        elements: &[ExprId],
        supplied: &[Value],
        span: Span,
    ) -> Res<Value> {
        let facts = self.types.facts(ty)?.clone();
        let TypeCategory::Array {
            element,
            dims,
            conformant,
        } = facts.category
        else {
            return Err(CodegenError::internal("array initializer on a non-array"));
        };
        let Some(dim) = dims.first().copied() else {
            return Err(CodegenError::internal("array has no dimensions"));
        };
        if conformant {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "conformant arrays cannot be initialized".into(),
                span,
            ));
        }
        if elements.len() as u64 > dim.length {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "too many array initializer elements".into(),
                span,
            ));
        }
        let storage = self.types.llvm_storage(ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        let pointee = storage.render();
        for (index, value) in supplied.iter().enumerate() {
            let slot = f.gep(
                &pointee,
                &temp,
                &["i64 0".into(), format!("i64 {index}")],
                true,
            );
            self.store_value_as_storage(f, &slot, element, value.clone())?;
        }
        Ok(f.load(storage, &temp, None))
    }

    /// Builds a struct initializer value.
    pub(crate) fn build_struct_init(
        &mut self,
        f: &mut Function,
        ty: TyId,
        fields: &[(FieldId, ExprId)],
        supplied: &[Value],
        _span: Span,
    ) -> Res<Value> {
        let storage = self.types.llvm_storage(ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        for ((field, _), value) in fields.iter().zip(supplied.iter()) {
            let field_ty = self.hir.field(*field).ty;
            let slot = self.aggregate_pointer(f, ty, &temp, &[RootSegment::Field(*field)])?;
            self.store_value_as_storage(f, &slot, field_ty, value.clone())?;
        }
        Ok(f.load(storage, &temp, None))
    }

    /// Stores an SSA value into storage of the same type, normalizing `BOOL`.
    pub(crate) fn store_value_as_storage(
        &mut self,
        f: &mut Function,
        destination: &Value,
        ty: TyId,
        value: Value,
    ) -> Res<()> {
        let facts = self.types.facts(ty)?.clone();
        if let Some(kind) = facts.scalar() {
            if kind == crate::layout::ScalarKind::Bool {
                let stored = if value.ty == IrType::Int(1) {
                    f.emit_value_cast("zext", &value, IrType::Int(8))
                } else {
                    value
                };
                f.store(&stored, destination, None);
            } else {
                f.store(&value, destination, None);
            }
            return Ok(());
        }
        f.store(&value, destination, None);
        Ok(())
    }

    /// Lowers an aggregate-to-aggregate cast. Only string capacity changes are
    /// supported in M5 phase 2.
    pub(crate) fn lower_aggregate_cast(
        &mut self,
        f: &mut Function,
        value: Value,
        from: TyId,
        to: TyId,
        span: Span,
    ) -> Res<Value> {
        let from_facts = self.types.facts(from)?.clone();
        let to_facts = self.types.facts(to)?.clone();
        if from_facts.key == to_facts.key {
            return Ok(value);
        }
        match (&from_facts.category, &to_facts.category) {
            (
                TypeCategory::Null,
                TypeCategory::Reference { .. }
                | TypeCategory::Interface { .. }
                | TypeCategory::Pointer { .. },
            ) => Ok(Value::zero(to_facts.llvm_storage.clone())),
            (TypeCategory::Reference { .. }, TypeCategory::Reference { .. }) => Ok(value),
            (TypeCategory::Pointer { .. }, TypeCategory::Pointer { .. }) => Ok(value),
            (
                TypeCategory::String {
                    kind: from_kind, ..
                },
                TypeCategory::String { kind: to_kind, .. },
            ) if from_kind == to_kind => {
                self.lower_string_cast(f, value, &from_facts, &to_facts, span)
            }
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "conversion from `{}` to `{}` is not supported in M5 phase 2",
                    from_facts.key, to_facts.key
                ),
                span,
            )),
        }
    }

    fn lower_string_cast(
        &mut self,
        f: &mut Function,
        value: Value,
        from: &crate::types::TypeFacts,
        to: &crate::types::TypeFacts,
        span: Span,
    ) -> Res<Value> {
        let (
            TypeCategory::String {
                capacity: from_capacity,
                unit_bytes,
                ..
            },
            TypeCategory::String {
                capacity: to_capacity,
                ..
            },
        ) = (&from.category, &to.category)
        else {
            unreachable!();
        };
        if from_capacity == to_capacity {
            return Ok(value);
        }
        let from_storage = from.llvm_storage.clone();
        let to_storage = to.llvm_storage.clone();
        let source = f.alloca(from_storage.clone());
        f.store(&value, &source, None);
        let from_pointee = from_storage.render();
        let len_ptr = f.gep(
            &from_pointee,
            &source,
            &["i64 0".into(), "i32 0".into()],
            true,
        );
        let length = f.load(IrType::Int(32), &len_ptr, None);
        let limit = Value::int(32, i128::from(*to_capacity));
        let too_long = f.icmp("sgt", &length, &limit);
        self.trap_if(f, &too_long, trap::RANGE, span)?;
        let destination = f.alloca(to_storage.clone());
        f.store(&Value::zero(to_storage.clone()), &destination, None);
        let to_pointee = to_storage.render();
        let dest_len = f.gep(
            &to_pointee,
            &destination,
            &["i64 0".into(), "i32 0".into()],
            true,
        );
        f.store(&length, &dest_len, None);
        let bytes = f.emit_value_cast("sext", &length, IrType::Int(64));
        let bytes = f.binop("mul", &bytes, &Value::int(64, i128::from(*unit_bytes)));
        let source_data = f.gep(
            &from_pointee,
            &source,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        let dest_data = f.gep(
            &to_pointee,
            &destination,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        self.ensure_memcpy();
        f.memcpy(&dest_data, &source_data, &bytes);
        Ok(f.load(to_storage, &destination, None))
    }

    /// Declares the `llvm.memcpy` intrinsic once per module.
    pub(crate) fn ensure_memcpy(&mut self) {
        self.module.declare(
            "llvm.memcpy.p0.p0.i64",
            "void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)",
        );
    }

    /// Declares the C `memcmp` helper once per module.
    pub(crate) fn ensure_memcmp(&mut self) {
        self.module.declare("memcmp", "i32 @memcmp(ptr, ptr, i64)");
    }

    /// Compares two string values for equality or inequality.
    pub(crate) fn lower_string_comparison(
        &mut self,
        f: &mut Function,
        lhs: &Value,
        rhs: &Value,
        lhs_ty: TyId,
        equal: bool,
    ) -> Res<Value> {
        let facts = self.types.facts(lhs_ty)?.clone();
        let TypeCategory::String { unit_bytes, .. } = facts.category else {
            return Err(CodegenError::internal("string comparison on a non-string"));
        };
        let storage = facts.llvm_storage.clone();
        let pointee = storage.render();
        let lhs_temp = f.alloca(storage.clone());
        f.store(lhs, &lhs_temp, None);
        let rhs_temp = f.alloca(storage.clone());
        f.store(rhs, &rhs_temp, None);
        let lhs_len_ptr = f.gep(&pointee, &lhs_temp, &["i64 0".into(), "i32 0".into()], true);
        let lhs_len = f.load(IrType::Int(32), &lhs_len_ptr, None);
        let rhs_len_ptr = f.gep(&pointee, &rhs_temp, &["i64 0".into(), "i32 0".into()], true);
        let rhs_len = f.load(IrType::Int(32), &rhs_len_ptr, None);
        let same_len = f.icmp("eq", &lhs_len, &rhs_len);
        let bytes = f.emit_value_cast("sext", &lhs_len, IrType::Int(64));
        let bytes = f.binop("mul", &bytes, &Value::int(64, i128::from(unit_bytes)));
        let lhs_data = f.gep(
            &pointee,
            &lhs_temp,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        let rhs_data = f.gep(
            &pointee,
            &rhs_temp,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        self.ensure_memcmp();
        let comparison = f
            .call(IrType::Int(32), "memcmp", &[lhs_data, rhs_data, bytes])
            .ok_or_else(|| CodegenError::internal("memcmp returned void"))?;
        let same_bytes = f.icmp("eq", &comparison, &Value::int(32, 0));
        let same = f.binop("and", &same_len, &same_bytes);
        Ok(if equal {
            same
        } else {
            f.binop("xor", &same, &Value::bool(true))
        })
    }
}

impl Lowerer<'_> {
    /// Lowers one resolved standard-function call (Runtime and Execution §3.1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_std_function(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: stc_hir::StdFnId,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
    ) -> Res<Value> {
        use stc_hir::StdFnKind;
        let std = self.hir.std_fns[id.index()].clone();
        let arg_types: Vec<TyId> = args
            .iter()
            .filter_map(|arg| arg.value)
            .map(|value| self.hir.expr(value).ty)
            .collect();
        let facts_of = |this: &Self, index: usize| -> Res<crate::types::TypeFacts> {
            let ty = arg_types
                .get(index)
                .copied()
                .ok_or_else(|| CodegenError::internal("missing standard-function argument"))?;
            Ok(this.types.facts(ty)?.clone())
        };
        match std.kind {
            StdFnKind::Len => {
                let facts = facts_of(self, 0)?;
                self.string_len_value(f, &supplied[0], &facts)
            }
            StdFnKind::LenMax => {
                let facts = facts_of(self, 0)?;
                let TypeCategory::String { capacity, .. } = facts.category else {
                    return Err(CodegenError::internal("LEN_MAX on a non-string"));
                };
                Ok(Value::int(32, i128::from(capacity)))
            }
            StdFnKind::LenCodeUnit => {
                let facts = facts_of(self, 0)?;
                match facts.category {
                    TypeCategory::String {
                        kind: stc_hir::StringKind::Unicode,
                        ..
                    } => self.utf8_code_unit_length(f, &supplied[0], &facts),
                    TypeCategory::String { .. } => self.string_len_value(f, &supplied[0], &facts),
                    _ => Err(CodegenError::internal("LEN_CODE_UNIT on a non-string")),
                }
            }
            StdFnKind::Concat => self.lower_concat(f, args, supplied, result_ty, span),
            StdFnKind::Move => Ok(supplied[0].clone()),
            StdFnKind::Gt
            | StdFnKind::Ge
            | StdFnKind::Eq
            | StdFnKind::Le
            | StdFnKind::Lt
            | StdFnKind::Ne => {
                let facts = facts_of(self, 0)?;
                let Some(kind) = facts.scalar() else {
                    return Err(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate standard comparisons are M5 phase 3".into(),
                        span,
                    ));
                };
                let op = match std.kind {
                    StdFnKind::Gt => stc_hir::BinOp::Gt,
                    StdFnKind::Ge => stc_hir::BinOp::Ge,
                    StdFnKind::Eq => stc_hir::BinOp::Eq,
                    StdFnKind::Le => stc_hir::BinOp::Le,
                    StdFnKind::Lt => stc_hir::BinOp::Lt,
                    _ => stc_hir::BinOp::Ne,
                };
                self.lower_comparison(f, op, kind, &supplied[0], &supplied[1], span)
            }
            StdFnKind::Add | StdFnKind::Sub | StdFnKind::Mul => {
                let facts = facts_of(self, 0)?;
                let Some(kind) = facts.scalar() else {
                    return Err(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate arithmetic is M5 phase 3".into(),
                        span,
                    ));
                };
                let op = match std.kind {
                    StdFnKind::Add => stc_hir::BinOp::Add,
                    StdFnKind::Sub => stc_hir::BinOp::Sub,
                    _ => stc_hir::BinOp::Mul,
                };
                let mut accumulator = supplied[0].clone();
                for value in &supplied[1..] {
                    accumulator =
                        self.lower_arithmetic(f, info, op, kind, &accumulator, value, span)?;
                }
                Ok(accumulator)
            }
            StdFnKind::Div | StdFnKind::Mod => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate division is M5 phase 3".into(),
                        span,
                    )
                })?;
                let op = if std.kind == StdFnKind::Div {
                    stc_hir::BinOp::Div
                } else {
                    stc_hir::BinOp::Mod
                };
                self.lower_arithmetic(f, info, op, kind, &supplied[0], &supplied[1], span)
            }
            StdFnKind::And | StdFnKind::Or | StdFnKind::Xor => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate bitwise operations are M5 phase 3".into(),
                        span,
                    )
                })?;
                let op = match std.kind {
                    StdFnKind::And => "and",
                    StdFnKind::Or => "or",
                    _ => "xor",
                };
                let mut accumulator = supplied[0].clone();
                for value in &supplied[1..] {
                    accumulator = self.bit_op(f, op, kind, &accumulator, value, span)?;
                }
                Ok(accumulator)
            }
            StdFnKind::Not => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate NOT is M5 phase 3".into(),
                        span,
                    )
                })?;
                let bits = if kind == crate::layout::ScalarKind::Bool {
                    1
                } else {
                    crate::expr::value_bits(kind)
                };
                let mask = Value::int(bits, -1);
                Ok(f.binop("xor", &supplied[0], &mask))
            }
            StdFnKind::Abs => self.lower_abs(f, &facts_of(self, 0)?, &supplied[0], span),
            StdFnKind::Min | StdFnKind::Max => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate MIN/MAX is M5 phase 3".into(),
                        span,
                    )
                })?;
                let op = if std.kind == StdFnKind::Min {
                    stc_hir::BinOp::Lt
                } else {
                    stc_hir::BinOp::Gt
                };
                let mut accumulator = supplied[0].clone();
                for value in &supplied[1..] {
                    let condition =
                        self.lower_comparison(f, op, kind, value, &accumulator, span)?;
                    accumulator = f.select(&condition, value, &accumulator);
                }
                Ok(accumulator)
            }
            StdFnKind::Limit => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate LIMIT is M5 phase 3".into(),
                        span,
                    )
                })?;
                let (mn, value, mx) = (&supplied[0], &supplied[1], &supplied[2]);
                let below = self.lower_comparison(f, stc_hir::BinOp::Lt, kind, value, mn, span)?;
                let above = self.lower_comparison(f, stc_hir::BinOp::Gt, kind, value, mx, span)?;
                let low = f.select(&below, mn, value);
                Ok(f.select(&above, mx, &low))
            }
            StdFnKind::Sel => {
                let facts = facts_of(self, 1)?;
                let condition = if facts.scalar() == Some(crate::layout::ScalarKind::Bool) {
                    supplied[0].clone()
                } else {
                    return Err(CodegenError::internal("SEL selector is not BOOL"));
                };
                Ok(f.select(&condition, &supplied[2], &supplied[1]))
            }
            StdFnKind::Mux => {
                let selector_facts = facts_of(self, 0)?;
                let Some(kind) = selector_facts.scalar() else {
                    return Err(CodegenError::internal("MUX selector is not an integer"));
                };
                let selector = &supplied[0];
                let bits = crate::expr::value_bits(kind);
                let mut result = supplied[supplied.len() - 1].clone();
                for (index, value) in supplied[1..supplied.len() - 1].iter().enumerate().rev() {
                    let matches = f.icmp("eq", selector, &Value::int(bits, index as i128));
                    result = f.select(&matches, value, &result);
                }
                Ok(result)
            }
            StdFnKind::Shl | StdFnKind::Shr | StdFnKind::Rol | StdFnKind::Ror => {
                let facts = facts_of(self, 0)?;
                let kind = facts.scalar().ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "aggregate shifts are M5 phase 3".into(),
                        span,
                    )
                })?;
                let count_kind = facts_of(self, 1)?
                    .scalar()
                    .ok_or_else(|| CodegenError::internal("shift count is not an integer"))?;
                let count = if count_kind.is_signed() {
                    f.emit_value_cast("sext", &supplied[1], IrType::Int(64))
                } else {
                    f.emit_value_cast("zext", &supplied[1], IrType::Int(64))
                };
                self.lower_shift_or_rotate(f, std.kind, kind, &supplied[0], &count, span)
            }
            StdFnKind::Sqrt
            | StdFnKind::Ln
            | StdFnKind::Log
            | StdFnKind::Exp
            | StdFnKind::Sin
            | StdFnKind::Cos
            | StdFnKind::Tan
            | StdFnKind::Asin
            | StdFnKind::Acos
            | StdFnKind::Atan => {
                let facts = facts_of(self, 0)?;
                let value = supplied[0].clone();
                self.lower_float_unary(f, std.kind, &facts, value)
            }
            StdFnKind::Atan2 | StdFnKind::Expt => {
                let facts = facts_of(self, 0)?;
                let lhs = supplied[0].clone();
                let rhs = supplied[1].clone();
                self.lower_float_binary(f, std.kind, &facts, lhs, rhs)
            }
            StdFnKind::Trunc => {
                let facts = facts_of(self, 0)?;
                let value = supplied[0].clone();
                self.lower_trunc(f, &facts, value, span)
            }
            StdFnKind::Left | StdFnKind::Right => self.lower_string_extract(
                f,
                args,
                supplied,
                result_ty,
                span,
                std.kind == StdFnKind::Right,
            ),
            StdFnKind::Mid => self.lower_string_mid(f, args, supplied, result_ty, span),
            StdFnKind::Insert | StdFnKind::Replace => self.lower_string_edit(
                f,
                args,
                supplied,
                result_ty,
                span,
                std.kind == StdFnKind::Replace,
            ),
            StdFnKind::Delete => self.lower_string_delete(f, args, supplied, result_ty, span),
            StdFnKind::Find => self.lower_string_find(f, args, supplied, span),
        }
    }

    fn string_len_value(
        &mut self,
        f: &mut Function,
        value: &Value,
        facts: &crate::types::TypeFacts,
    ) -> Res<Value> {
        if !facts.is_string() {
            return Err(CodegenError::internal("LEN on a non-string"));
        }
        let storage = facts.llvm_storage.clone();
        let temp = f.alloca(storage.clone());
        f.store(value, &temp, None);
        let len = f.gep(
            &storage.render(),
            &temp,
            &["i64 0".into(), "i32 0".into()],
            true,
        );
        Ok(f.load(IrType::Int(32), &len, None))
    }

    fn utf8_code_unit_length(
        &mut self,
        f: &mut Function,
        value: &Value,
        facts: &crate::types::TypeFacts,
    ) -> Res<Value> {
        let TypeCategory::String {
            capacity,
            unit_bytes,
            ..
        } = facts.category
        else {
            return Err(CodegenError::internal("LEN_CODE_UNIT on a non-string"));
        };
        let storage = facts.llvm_storage.clone();
        let pointee = storage.render();
        let temp = f.alloca(storage.clone());
        f.store(value, &temp, None);
        let len_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        let length = f.load(IrType::Int(32), &len_ptr, None);
        let data = f.gep(&pointee, &temp, &["i64 0".into(), "i32 1".into()], true);
        let array_ty = format!("[{capacity} x i{}]", u16::from(unit_bytes) * 8);
        let index_ptr = f.alloca(IrType::Int(64));
        f.store(&Value::int(64, 0), &index_ptr, None);
        let total_ptr = f.alloca(IrType::Int(64));
        f.store(&Value::int(64, 0), &total_ptr, None);
        let header = f.next_block_label();
        let body = f.next_block_label();
        let exit = f.next_block_label();
        f.branch(&header);
        f.start_block(&header);
        let index = f.load(IrType::Int(64), &index_ptr, None);
        let length64 = f.emit_value_cast("sext", &length, IrType::Int(64));
        let done = f.icmp("sge", &index, &length64);
        f.cond_branch(&done, &exit, &body);
        f.start_block(&body);
        let slot = f.gep(
            &array_ty,
            &data,
            &["i64 0".into(), format!("i64 {}", index.repr)],
            true,
        );
        let scalar = f.load(IrType::Int(32), &slot, None);
        let one = Value::int(32, 1);
        let mut units = one.clone();
        for limit in [0x7F_i128, 0x7FF, 0xFFFF] {
            let above = f.icmp("ugt", &scalar, &Value::int(32, limit));
            let addition = f.select(&above, &one, &Value::int(32, 0));
            units = f.binop("add", &units, &addition);
        }
        let units64 = f.emit_value_cast("zext", &units, IrType::Int(64));
        let total = f.load(IrType::Int(64), &total_ptr, None);
        let next_total = f.binop("add", &total, &units64);
        f.store(&next_total, &total_ptr, None);
        let next_index = f.binop("add", &index, &Value::int(64, 1));
        f.store(&next_index, &index_ptr, None);
        f.branch(&header);
        f.start_block(&exit);
        let total = f.load(IrType::Int(64), &total_ptr, None);
        Ok(f.emit_value_cast("trunc", &total, IrType::Int(32)))
    }

    fn lower_concat(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
    ) -> Res<Value> {
        let facts = self.types.facts(result_ty)?.clone();
        let TypeCategory::String {
            capacity,
            unit_bytes,
            ..
        } = facts.category
        else {
            return Err(CodegenError::internal("CONCAT result is not a string"));
        };
        let storage = facts.llvm_storage.clone();
        let pointee = storage.render();
        let destination = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &destination, None);
        let running_ptr = f.alloca(IrType::Int(32));
        f.store(&Value::int(32, 0), &running_ptr, None);
        let dest_data = f.gep(
            &pointee,
            &destination,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        for (index, value) in supplied.iter().enumerate() {
            let Some(arg_ty) = args
                .iter()
                .filter_map(|arg| arg.value)
                .nth(index)
                .map(|expr| self.hir.expr(expr).ty)
            else {
                return Err(CodegenError::internal("CONCAT argument type missing"));
            };
            let arg_facts = self.types.facts(arg_ty)?.clone();
            let TypeCategory::String {
                capacity: _,
                unit_bytes: arg_unit,
                ..
            } = arg_facts.category
            else {
                return Err(CodegenError::internal("CONCAT argument is not a string"));
            };
            if arg_unit != unit_bytes {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "CONCAT of different string encodings is not supported".into(),
                    span,
                ));
            }
            let arg_storage = arg_facts.llvm_storage.clone();
            let arg_pointee = arg_storage.render();
            let temp = f.alloca(arg_storage.clone());
            f.store(value, &temp, None);
            let len_ptr = f.gep(&arg_pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
            let length = f.load(IrType::Int(32), &len_ptr, None);
            let total = f.load(IrType::Int(32), &running_ptr, None);
            let next_total = f.binop("add", &total, &length);
            let too_long = f.icmp("sgt", &next_total, &Value::int(32, i128::from(capacity)));
            self.trap_if(f, &too_long, trap::RANGE, span)?;
            let offset_bytes = f.emit_value_cast("sext", &total, IrType::Int(64));
            let offset_bytes = f.binop(
                "mul",
                &offset_bytes,
                &Value::int(64, i128::from(unit_bytes)),
            );
            let target = f.gep(
                "i8",
                &dest_data,
                &[format!("i64 {}", offset_bytes.repr)],
                false,
            );
            let arg_data = f.gep(
                &arg_pointee,
                &temp,
                &["i64 0".into(), "i32 1".into(), "i64 0".into()],
                true,
            );
            let bytes = f.emit_value_cast("sext", &length, IrType::Int(64));
            let bytes = f.binop("mul", &bytes, &Value::int(64, i128::from(arg_unit)));
            self.ensure_memcpy();
            f.memcpy(&target, &arg_data, &bytes);
            f.store(&next_total, &running_ptr, None);
        }
        let total = f.load(IrType::Int(32), &running_ptr, None);
        let len_ptr = f.gep(
            &pointee,
            &destination,
            &["i64 0".into(), "i32 0".into()],
            true,
        );
        f.store(&total, &len_ptr, None);
        Ok(f.load(storage, &destination, None))
    }
}

struct StringArgument {
    length: Value,
    data: Value,
    unit_bytes: u8,
}

struct StringBuffer {
    storage: IrType,
    pointee: String,
    length: Value,
    data: Value,
    capacity: u32,
    unit_bytes: u8,
    destination: Value,
}

impl Lowerer<'_> {
    fn std_arg_type(&self, args: &[stc_hir::ArgHir], index: usize) -> Res<TyId> {
        args.iter()
            .filter_map(|arg| arg.value)
            .nth(index)
            .map(|expr| self.hir.expr(expr).ty)
            .ok_or_else(|| CodegenError::internal("standard-function argument type missing"))
    }

    fn string_argument(
        &mut self,
        f: &mut Function,
        value: &Value,
        ty: TyId,
    ) -> Res<StringArgument> {
        let facts = self.types.facts(ty)?.clone();
        let TypeCategory::String { unit_bytes, .. } = facts.category else {
            return Err(CodegenError::internal(
                "standard function argument is not a string",
            ));
        };
        let storage = facts.llvm_storage.clone();
        let pointee = storage.render();
        let temp = f.alloca(storage.clone());
        f.store(value, &temp, None);
        let length_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        let length = f.load(IrType::Int(32), &length_ptr, None);
        let data = f.gep(
            &pointee,
            &temp,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        Ok(StringArgument {
            length,
            data,
            unit_bytes,
        })
    }

    fn new_string_buffer(&mut self, f: &mut Function, result_ty: TyId) -> Res<StringBuffer> {
        let facts = self.types.facts(result_ty)?.clone();
        let TypeCategory::String {
            capacity,
            unit_bytes,
            ..
        } = facts.category
        else {
            return Err(CodegenError::internal(
                "standard function result is not a string",
            ));
        };
        let storage = facts.llvm_storage.clone();
        let pointee = storage.render();
        let destination = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &destination, None);
        let length = f.gep(
            &pointee,
            &destination,
            &["i64 0".into(), "i32 0".into()],
            true,
        );
        let data = f.gep(
            &pointee,
            &destination,
            &["i64 0".into(), "i32 1".into(), "i64 0".into()],
            true,
        );
        Ok(StringBuffer {
            storage,
            pointee,
            length,
            data,
            capacity,
            unit_bytes,
            destination,
        })
    }

    fn finish_string_buffer(f: &mut Function, buffer: StringBuffer, length: Value) -> Value {
        f.store(&length, &buffer.length, None);
        f.load(buffer.storage, &buffer.destination, None)
    }

    fn require_string_unit(&self, actual: u8, expected: u8, span: Span) -> Res<()> {
        if actual == expected {
            Ok(())
        } else {
            Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "standard string functions cannot mix STRING, WSTRING, and USTRING".into(),
                span,
            ))
        }
    }

    fn integer_to_i32(f: &mut Function, value: &Value, kind: crate::layout::ScalarKind) -> Value {
        let width = crate::expr::value_bits(kind);
        if width < 32 {
            let op = if kind.is_signed() { "sext" } else { "zext" };
            f.emit_value_cast(op, value, IrType::Int(32))
        } else if width > 32 {
            f.emit_value_cast("trunc", value, IrType::Int(32))
        } else {
            value.clone()
        }
    }

    fn clamp_nonnegative(f: &mut Function, value: Value, upper: &Value) -> Value {
        let positive = f.icmp("sgt", &value, &Value::int(32, 0));
        let bounded = f.select(&positive, &value, &Value::int(32, 0));
        let above = f.icmp("sgt", &bounded, upper);
        f.select(&above, upper, &bounded)
    }

    fn clamp_position(f: &mut Function, position: &Value, length: &Value) -> Value {
        let start = f.binop("sub", position, &Value::int(32, 1));
        let positive = f.icmp("sgt", &start, &Value::int(32, 0));
        let bounded = f.select(&positive, &start, &Value::int(32, 0));
        let beyond = f.icmp("sgt", &bounded, length);
        f.select(&beyond, length, &bounded)
    }

    fn string_byte_pointer(f: &mut Function, data: &Value, index: &Value, unit_bytes: u8) -> Value {
        let offset = f.emit_value_cast("sext", index, IrType::Int(64));
        let offset = f.binop("mul", &offset, &Value::int(64, i128::from(unit_bytes)));
        f.gep("i8", data, &[format!("i64 {}", offset.repr)], false)
    }

    #[allow(clippy::too_many_arguments)]
    fn string_copy(
        &mut self,
        f: &mut Function,
        destination: &Value,
        destination_index: &Value,
        source: &Value,
        source_index: &Value,
        count: &Value,
        unit_bytes: u8,
    ) {
        let source_pointer = Self::string_byte_pointer(f, source, source_index, unit_bytes);
        let destination_pointer =
            Self::string_byte_pointer(f, destination, destination_index, unit_bytes);
        let bytes = f.emit_value_cast("sext", count, IrType::Int(64));
        let bytes = f.binop("mul", &bytes, &Value::int(64, i128::from(unit_bytes)));
        self.ensure_memcpy();
        f.memcpy(&destination_pointer, &source_pointer, &bytes);
    }

    fn lower_string_extract(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
        right: bool,
    ) -> Res<Value> {
        let input_ty = self.std_arg_type(args, 0)?;
        let count_ty = self.std_arg_type(args, 1)?;
        let input = self.string_argument(f, &supplied[0], input_ty)?;
        let buffer = self.new_string_buffer(f, result_ty)?;
        self.require_string_unit(input.unit_bytes, buffer.unit_bytes, span)?;
        let count_kind = self.types.scalar_kind(count_ty, span)?;
        let count = Self::integer_to_i32(f, &supplied[1], count_kind);
        let count = Self::clamp_nonnegative(f, count, &input.length);
        let start = if right {
            f.binop("sub", &input.length, &count)
        } else {
            Value::int(32, 0)
        };
        self.string_copy(
            f,
            &buffer.data,
            &Value::int(32, 0),
            &input.data,
            &start,
            &count,
            buffer.unit_bytes,
        );
        Ok(Self::finish_string_buffer(f, buffer, count))
    }

    fn lower_string_mid(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
    ) -> Res<Value> {
        let input_ty = self.std_arg_type(args, 0)?;
        let count_ty = self.std_arg_type(args, 1)?;
        let position_ty = self.std_arg_type(args, 2)?;
        let input = self.string_argument(f, &supplied[0], input_ty)?;
        let buffer = self.new_string_buffer(f, result_ty)?;
        self.require_string_unit(input.unit_bytes, buffer.unit_bytes, span)?;
        let count_kind = self.types.scalar_kind(count_ty, span)?;
        let position_kind = self.types.scalar_kind(position_ty, span)?;
        let position = Self::integer_to_i32(f, &supplied[2], position_kind);
        let start = Self::clamp_position(f, &position, &input.length);
        let available = f.binop("sub", &input.length, &start);
        let count = Self::integer_to_i32(f, &supplied[1], count_kind);
        let count = Self::clamp_nonnegative(f, count, &available);
        self.string_copy(
            f,
            &buffer.data,
            &Value::int(32, 0),
            &input.data,
            &start,
            &count,
            buffer.unit_bytes,
        );
        Ok(Self::finish_string_buffer(f, buffer, count))
    }

    fn lower_string_delete(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
    ) -> Res<Value> {
        let input_ty = self.std_arg_type(args, 0)?;
        let count_ty = self.std_arg_type(args, 1)?;
        let position_ty = self.std_arg_type(args, 2)?;
        let input = self.string_argument(f, &supplied[0], input_ty)?;
        let buffer = self.new_string_buffer(f, result_ty)?;
        self.require_string_unit(input.unit_bytes, buffer.unit_bytes, span)?;
        let count_kind = self.types.scalar_kind(count_ty, span)?;
        let position_kind = self.types.scalar_kind(position_ty, span)?;
        let position = Self::integer_to_i32(f, &supplied[2], position_kind);
        let start = Self::clamp_position(f, &position, &input.length);
        let available = f.binop("sub", &input.length, &start);
        let count = Self::integer_to_i32(f, &supplied[1], count_kind);
        let removed = Self::clamp_nonnegative(f, count, &available);
        let new_length = f.binop("sub", &input.length, &removed);
        self.string_copy(
            f,
            &buffer.data,
            &Value::int(32, 0),
            &input.data,
            &Value::int(32, 0),
            &start,
            buffer.unit_bytes,
        );
        let suffix_start = f.binop("add", &start, &removed);
        let suffix_count = f.binop("sub", &available, &removed);
        self.string_copy(
            f,
            &buffer.data,
            &start,
            &input.data,
            &suffix_start,
            &suffix_count,
            buffer.unit_bytes,
        );
        Ok(Self::finish_string_buffer(f, buffer, new_length))
    }

    fn lower_string_edit(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
        replace: bool,
    ) -> Res<Value> {
        let first_ty = self.std_arg_type(args, 0)?;
        let second_ty = self.std_arg_type(args, 1)?;
        let first = self.string_argument(f, &supplied[0], first_ty)?;
        let second = self.string_argument(f, &supplied[1], second_ty)?;
        let buffer = self.new_string_buffer(f, result_ty)?;
        self.require_string_unit(first.unit_bytes, buffer.unit_bytes, span)?;
        self.require_string_unit(second.unit_bytes, buffer.unit_bytes, span)?;
        let position_index = if replace { 3 } else { 2 };
        let position_ty = self.std_arg_type(args, position_index)?;
        let position_kind = self.types.scalar_kind(position_ty, span)?;
        let position = Self::integer_to_i32(f, &supplied[position_index], position_kind);
        let start = Self::clamp_position(f, &position, &first.length);
        let available = f.binop("sub", &first.length, &start);
        let removed = if replace {
            let count_ty = self.std_arg_type(args, 2)?;
            let count_kind = self.types.scalar_kind(count_ty, span)?;
            let count = Self::integer_to_i32(f, &supplied[2], count_kind);
            Self::clamp_nonnegative(f, count, &available)
        } else {
            Value::int(32, 0)
        };
        let kept = f.binop("sub", &available, &removed);
        let base_length = f.binop("sub", &first.length, &removed);
        let new_length = f.binop("add", &base_length, &second.length);
        let too_long = f.icmp(
            "sgt",
            &new_length,
            &Value::int(32, i128::from(buffer.capacity)),
        );
        self.trap_if(f, &too_long, trap::RANGE, span)?;
        self.string_copy(
            f,
            &buffer.data,
            &Value::int(32, 0),
            &first.data,
            &Value::int(32, 0),
            &start,
            buffer.unit_bytes,
        );
        self.string_copy(
            f,
            &buffer.data,
            &start,
            &second.data,
            &Value::int(32, 0),
            &second.length,
            buffer.unit_bytes,
        );
        let destination_suffix = f.binop("add", &start, &second.length);
        let source_suffix = f.binop("add", &start, &removed);
        self.string_copy(
            f,
            &buffer.data,
            &destination_suffix,
            &first.data,
            &source_suffix,
            &kept,
            buffer.unit_bytes,
        );
        Ok(Self::finish_string_buffer(f, buffer, new_length))
    }

    fn lower_string_find(
        &mut self,
        f: &mut Function,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        span: Span,
    ) -> Res<Value> {
        let haystack_ty = self.std_arg_type(args, 0)?;
        let needle_ty = self.std_arg_type(args, 1)?;
        let haystack = self.string_argument(f, &supplied[0], haystack_ty)?;
        let needle = self.string_argument(f, &supplied[1], needle_ty)?;
        self.require_string_unit(haystack.unit_bytes, needle.unit_bytes, span)?;
        let result_ptr = f.alloca(IrType::Int(32));
        f.store(&Value::int(32, 0), &result_ptr, None);
        let index_ptr = f.alloca(IrType::Int(32));
        f.store(&Value::int(32, 0), &index_ptr, None);
        let limit = f.binop("sub", &haystack.length, &needle.length);
        let header = f.next_block_label();
        let body = f.next_block_label();
        let done = f.next_block_label();
        f.branch(&header);
        f.start_block(&header);
        let index = f.load(IrType::Int(32), &index_ptr, None);
        let result = f.load(IrType::Int(32), &result_ptr, None);
        let in_range = f.icmp("sle", &index, &limit);
        let searching = f.icmp("eq", &result, &Value::int(32, 0));
        let active = f.binop("and", &in_range, &searching);
        f.cond_branch(&active, &body, &done);
        f.start_block(&body);
        let candidate = Self::string_byte_pointer(f, &haystack.data, &index, haystack.unit_bytes);
        let bytes = f.emit_value_cast("sext", &needle.length, IrType::Int(64));
        let bytes = f.binop(
            "mul",
            &bytes,
            &Value::int(64, i128::from(needle.unit_bytes)),
        );
        self.ensure_memcmp();
        let comparison = f
            .call(IrType::Int(32), "memcmp", &[candidate, needle.data, bytes])
            .ok_or_else(|| CodegenError::internal("memcmp returned void"))?;
        let found = f.icmp("eq", &comparison, &Value::int(32, 0));
        let one_based = f.binop("add", &index, &Value::int(32, 1));
        let next_result = f.select(&found, &one_based, &result);
        f.store(&next_result, &result_ptr, None);
        let next_index = f.binop("add", &index, &Value::int(32, 1));
        f.store(&next_index, &index_ptr, None);
        f.branch(&header);
        f.start_block(&done);
        Ok(f.load(IrType::Int(32), &result_ptr, None))
    }

    fn lower_abs(
        &mut self,
        f: &mut Function,
        facts: &crate::types::TypeFacts,
        value: &Value,
        span: Span,
    ) -> Res<Value> {
        let kind = facts.scalar().ok_or_else(|| {
            CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "aggregate ABS is M5 phase 3".into(),
                span,
            )
        })?;
        match kind {
            crate::layout::ScalarKind::Real { width } => {
                let name = if width == 32 {
                    "llvm.fabs.f32"
                } else {
                    "llvm.fabs.f64"
                };
                let ty = facts.llvm_value.clone();
                self.module
                    .declare(name, &format!("{} @{name}({})", ty.render(), ty.render()));
                f.call(ty, name, std::slice::from_ref(value))
                    .ok_or_else(|| CodegenError::internal("fabs returned void"))
            }
            crate::layout::ScalarKind::Int { width, signed } => {
                if signed {
                    let minimum = -(1_i128 << (width - 1));
                    let is_min = f.icmp("eq", value, &Value::int(width, minimum));
                    self.trap_if(f, &is_min, trap::OVERFLOW, span)?;
                }
                let zero = Value::int(width, 0);
                let negative = if signed {
                    f.icmp("slt", value, &zero)
                } else {
                    Value::bool(false)
                };
                let negated = f.binop("sub", &zero, value);
                Ok(f.select(&negative, &negated, value))
            }
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "ABS on this type is not supported".into(),
                span,
            )),
        }
    }

    fn lower_shift_or_rotate(
        &mut self,
        f: &mut Function,
        kind: stc_hir::StdFnKind,
        scalar: crate::layout::ScalarKind,
        value: &Value,
        count: &Value,
        _span: Span,
    ) -> Res<Value> {
        use stc_hir::StdFnKind;
        let width = crate::expr::value_bits(scalar);
        let count = match kind {
            StdFnKind::Rol | StdFnKind::Ror => count.clone(),
            _ => {
                let mask = Value::int(64, i128::from(width - 1));
                f.binop("and", count, &mask)
            }
        };
        let count = if count.ty == IrType::Int(64) {
            count
        } else {
            f.emit_value_cast("zext", &count, IrType::Int(64))
        };
        match kind {
            StdFnKind::Shl => Ok(f.binop("shl", value, &count)),
            StdFnKind::Shr => Ok(f.binop("lshr", value, &count)),
            StdFnKind::Rol | StdFnKind::Ror => {
                let ty = IrType::Int(width);
                let name = if kind == StdFnKind::Rol {
                    format!("llvm.fshl.i{width}")
                } else {
                    format!("llvm.fshr.i{width}")
                };
                self.module.declare(
                    &name,
                    &format!(
                        "{} @{name}({}, {}, {})",
                        ty.render(),
                        ty.render(),
                        ty.render(),
                        ty.render()
                    ),
                );
                let count = if count.ty == ty {
                    count
                } else {
                    f.emit_value_cast("trunc", &count, ty.clone())
                };
                f.call(ty, &name, &[value.clone(), value.clone(), count])
                    .ok_or_else(|| CodegenError::internal("rotate returned void"))
            }
            _ => Err(CodegenError::internal("not a shift or rotate")),
        }
    }
}

impl Lowerer<'_> {
    /// Resolves a scalar leaf inside an array of aggregates selected by a
    /// runtime index. Each element's leaf root is a separate host cell; the
    /// generated code selects the active cell with a bounded select chain.
    fn expanded_leaf_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Option<ResolvedPointer>> {
        if !self.types.facts(path.ty)?.is_scalar_shaped() {
            return Ok(None);
        }
        #[derive(Clone)]
        struct Candidate {
            segments: Vec<RootSegment>,
            condition: Option<Value>,
        }
        let mut segments = Vec::new();
        let mut ty = match &path.base {
            PlaceBase::Instance { fields, .. } => {
                for field in fields {
                    segments.push(RootSegment::Field(*field));
                }
                fields.last().map(|field| self.hir.field(*field).ty)
            }
            PlaceBase::Global(def) => match &self.hir.defs[def.index()].kind {
                DefKind::Global { ty, .. } => Some(*ty),
                _ => return Ok(None),
            },
            PlaceBase::This(_) => None,
            _ => return Ok(None),
        };
        let mut candidates = vec![Candidate {
            segments,
            condition: None,
        }];
        let mut dimension = 0_usize;
        let mut saw_index = false;
        for step in &path.steps {
            match step {
                PlaceStep::Field(field) => {
                    for candidate in &mut candidates {
                        candidate.segments.push(RootSegment::Field(*field));
                    }
                    ty = Some(self.hir.field(*field).ty);
                    dimension = 0;
                }
                PlaceStep::Index(index) => {
                    let current =
                        ty.ok_or_else(|| CodegenError::internal("index precedes the first field"))?;
                    if self.types.is_single_root(current)? {
                        // A contiguous array root is handled by the GEP path.
                        return Ok(None);
                    }
                    let dim = self.array_dim_at(current, dimension)?;
                    let delta = self.bounds_checked_index(f, info, *index, &dim, path.span)?;
                    if dim.length > 4096 {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "runtime selection in an array of more than 4096 aggregate elements \
                             is not supported in M5"
                                .into(),
                            path.span,
                        ));
                    }
                    let delta = Value {
                        ty: IrType::Int(64),
                        repr: delta,
                    };
                    let mut expanded = Vec::with_capacity(
                        candidates
                            .len()
                            .saturating_mul(usize::try_from(dim.length).unwrap_or(usize::MAX)),
                    );
                    for candidate in &candidates {
                        for element_index in 0..dim.length {
                            let matches =
                                f.icmp("eq", &delta, &Value::int(64, i128::from(element_index)));
                            let condition = match &candidate.condition {
                                Some(previous) => f.binop("and", previous, &matches),
                                None => matches,
                            };
                            let mut segments = candidate.segments.clone();
                            segments.push(RootSegment::Element {
                                index: element_index,
                                dim: u32::try_from(dimension).unwrap_or(u32::MAX),
                            });
                            expanded.push(Candidate {
                                segments,
                                condition: Some(condition),
                            });
                        }
                    }
                    candidates = expanded;
                    let (next_ty, next_dim) = self.array_advance(current, dimension)?;
                    ty = Some(next_ty);
                    dimension = next_dim;
                    saw_index = true;
                }
            }
        }
        if !saw_index || candidates.is_empty() {
            return Ok(None);
        }
        let mut pointer: Option<Value> = None;
        let mut direct: Option<DirectAddress> = None;
        for candidate in candidates {
            let resolved = self.root_pointer_for_segments(
                f,
                info,
                &path.base,
                &candidate.segments,
                path.span,
            )?;
            if resolved.direct.is_some() {
                direct = resolved.direct.clone();
            }
            pointer = Some(match (pointer, candidate.condition) {
                (None, _) => resolved.pointer,
                (Some(previous), Some(condition)) => {
                    f.select(&condition, &resolved.pointer, &previous)
                }
                (Some(previous), None) => previous,
            });
        }
        Ok(pointer.map(|pointer| ResolvedPointer { pointer, direct }))
    }
}

impl Lowerer<'_> {
    /// Returns the storage pointer denoted by a place rooted at a pointer or
    /// reference dereference. Returns `None` when the place has no such root;
    /// ordinary places are resolved through [`Self::collect_place`].
    pub(crate) fn pointer_place_address(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
    ) -> Res<Option<Value>> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Place(place) = expr.kind else {
            return Ok(None);
        };
        match place {
            PlaceHir::Deref(base) => Ok(Some(self.pointer_base_storage(f, info, base, expr.span)?)),
            PlaceHir::Index { base, indices } => {
                let base_ty = self.hir.expr(base).ty;
                if let Ty::Ptr(pointer) = self.hir.types.get(base_ty).clone() {
                    if indices.len() != 1 {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "pointer indexing consumes exactly one subscript".into(),
                            expr.span,
                        ));
                    }
                    let pointer_value = self.lower_expr(f, info, base)?;
                    self.check_pointer(f, &pointer_value, expr.span)?;
                    let index = self.pointer_index_value(f, info, indices[0], expr.span)?;
                    let pointee = self.types.llvm_storage(pointer.target)?.render();
                    let pointer = f.gep(
                        &pointee,
                        &pointer_value,
                        &[format!("i64 {}", index.repr)],
                        false,
                    );
                    return Ok(Some(pointer));
                }
                let Some(pointer) = self.pointer_place_address(f, info, base)? else {
                    return Ok(None);
                };
                let pointer =
                    self.index_pointer_value(f, info, &pointer, base_ty, &indices, expr.span)?;
                Ok(Some(pointer))
            }
            PlaceHir::Field { base, field } => {
                let Some(mut pointer) = self.pointer_place_address(f, info, base)? else {
                    return Ok(None);
                };
                let base_ty = self.hir.expr(base).ty;
                let overlap = matches!(self.types.category(base_ty)?, TypeCategory::Struct { .. })
                    && self.types.named(self.owner_of(base_ty)?)?.overlap;
                if overlap {
                    let offset = self
                        .types
                        .field_offset(field)
                        .ok_or_else(|| CodegenError::internal("overlap field has no offset"))?;
                    pointer = f.gep_bytes(&pointer, i64::try_from(offset).unwrap_or(i64::MAX));
                } else {
                    let indices = self.types.field_gep_indices(base_ty, field)?;
                    let pointee = self.types.llvm_storage(base_ty)?.render();
                    let mut gep = vec!["i32 0".to_owned()];
                    gep.extend(indices.into_iter().map(|index| format!("i32 {index}")));
                    pointer = f.gep(&pointee, &pointer, &gep, true);
                }
                Ok(Some(pointer))
            }
            _ => Ok(None),
        }
    }

    /// Resolves the storage pointer held by a `REF_TO` or `POINTER TO`
    /// expression, trapping on a null pointer.
    fn pointer_base_storage(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        base: ExprId,
        span: Span,
    ) -> Res<Value> {
        match self.hir.types.get(self.hir.expr(base).ty).clone() {
            Ty::Ptr(_) => {
                let value = self.lower_expr(f, info, base)?;
                self.check_pointer(f, &value, span)?;
                Ok(value)
            }
            Ty::Ref(_) => self.deref_pointer(f, info, base, span),
            _ => Err(CodegenError::internal(
                "dereference base is neither a reference nor a pointer",
            )),
        }
    }

    /// Applies array/string subscripts to a pointer that addresses `ty`.
    fn index_pointer_value(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        pointer: &Value,
        ty: TyId,
        indices: &[ExprId],
        span: Span,
    ) -> Res<Value> {
        let mut pointer = pointer.clone();
        let mut ty = ty;
        let mut dimension = 0_usize;
        for index in indices {
            match self.types.category(ty)?.clone() {
                TypeCategory::Array { .. } => {
                    let dim = self.array_dim_at(ty, dimension)?;
                    let delta = self.bounds_checked_index(f, info, *index, &dim, span)?;
                    let pointee = self.array_tail_type(ty, dimension)?;
                    pointer = f.gep(
                        &pointee,
                        &pointer,
                        &["i64 0".into(), format!("i64 {delta}")],
                        true,
                    );
                    let (next_ty, next_dim) = self.array_advance(ty, dimension)?;
                    ty = next_ty;
                    dimension = next_dim;
                }
                TypeCategory::String { capacity, .. } => {
                    let dim = ArrayDim {
                        lower: 1,
                        upper: i128::from(capacity),
                        length: u64::from(capacity),
                    };
                    let delta = self.bounds_checked_index(f, info, *index, &dim, span)?;
                    let pointee = self.types.llvm_storage(ty)?.render();
                    pointer = f.gep(
                        &pointee,
                        &pointer,
                        &["i64 0".into(), "i32 1".into(), format!("i64 {delta}")],
                        true,
                    );
                    ty = self.types.string_char_ty(ty)?;
                }
                _ => {
                    return Err(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "subscript on a value reached through a pointer".into(),
                        span,
                    ));
                }
            }
        }
        Ok(pointer)
    }

    /// Lowers a pointer index to a sign- or zero-extended `i64`.
    fn pointer_index_value(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        index: ExprId,
        span: Span,
    ) -> Res<Value> {
        let value = self.lower_expr(f, info, index)?;
        let kind = self
            .types
            .facts(self.hir.expr(index).ty)?
            .scalar()
            .ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "pointer index is not an integer".into(),
                    span,
                )
            })?;
        Ok(if kind.is_signed() {
            f.emit_value_cast("sext", &value, IrType::Int(64))
        } else {
            f.emit_value_cast("zext", &value, IrType::Int(64))
        })
    }

    /// Traps when a raw pointer is null (`STC-TRAP_NULL_REF`).
    pub(crate) fn check_pointer(&mut self, f: &mut Function, value: &Value, span: Span) -> Res<()> {
        let is_null = f.icmp("eq", value, &Value::null_ptr());
        self.trap_if(f, &is_null, trap::NULL_REF, span)
    }

    /// Lowers `ADR(place)`.
    pub(crate) fn lower_address_of(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        target: ExprId,
        span: Span,
    ) -> Res<Value> {
        let expr = self.hir.expr(target).clone();
        if let ExprKindHir::Place(PlaceHir::Direct(address)) = &expr.kind {
            if is_bit_address(address) {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "an individual `AT` bit has no pointer representation".into(),
                    span,
                ));
            }
            let kind = self.scalar_at(expr.ty, span)?;
            return self.image_service_pointer(f, info, address, kind);
        }
        if let Some(path) = self.collect_place(target)
            && let Some(resolved) = self.resolve_place_pointer(f, info, &path)?
        {
            if let Some(address) = &resolved.direct
                && is_bit_address(address)
            {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "an individual `AT` bit has no pointer representation".into(),
                    span,
                ));
            }
            return Ok(resolved.pointer);
        }
        if let Some(pointer) = self.pointer_place_address(f, info, target)? {
            return Ok(pointer);
        }
        Err(CodegenError::unsupported(
            codes::CODEGEN_UNSUPPORTED,
            "the pointer target has no contiguous storage".into(),
            span,
        ))
    }

    /// Lowers `SIZEOF(type | place)` to its constant `UDINT` value.
    pub(crate) fn lower_sizeof(&mut self, ty: TyId, span: Span) -> Res<Value> {
        let facts = self.types.facts(ty)?.clone();
        if facts.is_object() || facts.is_interface() || facts.is_reference() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`SIZEOF` requires a statically sized type".into(),
                span,
            ));
        }
        if matches!(
            facts.category,
            TypeCategory::Array {
                conformant: true,
                ..
            }
        ) {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`SIZEOF` of a conformant array is not defined".into(),
                span,
            ));
        }
        let size = u32::try_from(facts.layout.size).map_err(|_| {
            CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`SIZEOF` does not fit `UDINT`".into(),
                span,
            )
        })?;
        Ok(Value::int(32, i128::from(size)))
    }

    /// Checks that an integer value is inside a subrange, trapping `RANGE`.
    pub(crate) fn check_subrange(
        &mut self,
        f: &mut Function,
        ty: TyId,
        value: &Value,
        span: Span,
    ) -> Res<()> {
        let facts = self.types.facts(ty)?.clone();
        let TypeCategory::Subrange { lower, upper, .. } = facts.category else {
            return Ok(());
        };
        let bits = match value.ty {
            IrType::Int(bits) => bits,
            _ => return Ok(()),
        };
        let below = f.icmp("slt", value, &Value::int(bits, lower));
        let above = f.icmp("sgt", value, &Value::int(bits, upper));
        let bad = f.binop("or", &below, &above);
        self.trap_if(f, &bad, trap::RANGE, span)
    }
}

impl Lowerer<'_> {
    /// Copies an object's state per field, preserving the destination object.
    pub(crate) fn copy_object_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        destination: &crate::place::PlacePath,
        source: &crate::place::PlacePath,
    ) -> Res<()> {
        let storage = self.types.llvm_storage(destination.ty)?;
        let destination_pointer = self.resolve_place_pointer(f, info, destination)?;
        let source_pointer = self.resolve_place_pointer(f, info, source)?;
        if let (Some(destination_pointer), Some(source_pointer)) =
            (&destination_pointer, &source_pointer)
        {
            let value = f.load(storage, &source_pointer.pointer, None);
            f.store(&value, &destination_pointer.pointer, None);
            return Ok(());
        }
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        if let Some(source_pointer) = &source_pointer {
            let value = f.load(storage.clone(), &source_pointer.pointer, None);
            f.store(&value, &temp, None);
        } else {
            self.gather_place_into_temp(f, info, source, &temp)?;
        }
        if let Some(destination_pointer) = &destination_pointer {
            let value = f.load(storage, &temp, None);
            f.store(&value, &destination_pointer.pointer, None);
        } else {
            self.split_place_from_temp(f, info, destination, &temp)?;
        }
        Ok(())
    }
}

impl Lowerer<'_> {
    fn float_intrinsic_name(&self, kind: stc_hir::StdFnKind, width: u16) -> &'static str {
        use stc_hir::StdFnKind;
        match (kind, width == 32) {
            (StdFnKind::Sqrt, true) => "llvm.sqrt.f32",
            (StdFnKind::Sqrt, false) => "llvm.sqrt.f64",
            (StdFnKind::Ln, true) => "llvm.log.f32",
            (StdFnKind::Ln, false) => "llvm.log.f64",
            (StdFnKind::Log, true) => "llvm.log10.f32",
            (StdFnKind::Log, false) => "llvm.log10.f64",
            (StdFnKind::Exp, true) => "llvm.exp.f32",
            (StdFnKind::Exp, false) => "llvm.exp.f64",
            (StdFnKind::Sin, true) => "llvm.sin.f32",
            (StdFnKind::Sin, false) => "llvm.sin.f64",
            (StdFnKind::Cos, true) => "llvm.cos.f32",
            (StdFnKind::Cos, false) => "llvm.cos.f64",
            (StdFnKind::Tan, true) => "llvm.tan.f32",
            (StdFnKind::Tan, false) => "llvm.tan.f64",
            (StdFnKind::Asin, true) => "llvm.asin.f32",
            (StdFnKind::Asin, false) => "llvm.asin.f64",
            (StdFnKind::Acos, true) => "llvm.acos.f32",
            (StdFnKind::Acos, false) => "llvm.acos.f64",
            (StdFnKind::Atan, true) => "llvm.atan.f32",
            (StdFnKind::Atan, false) => "llvm.atan.f64",
            (StdFnKind::Atan2, true) => "llvm.atan2.f32",
            (StdFnKind::Atan2, false) => "llvm.atan2.f64",
            (StdFnKind::Expt, true) => "llvm.pow.f32",
            (StdFnKind::Expt, false) => "llvm.pow.f64",
            _ => "llvm.sqrt.f64",
        }
    }

    fn lower_float_unary(
        &mut self,
        f: &mut Function,
        kind: stc_hir::StdFnKind,
        facts: &crate::types::TypeFacts,
        value: Value,
    ) -> Res<Value> {
        let crate::layout::ScalarKind::Real { width } = facts
            .scalar()
            .ok_or_else(|| CodegenError::internal("math argument is not REAL"))?
        else {
            return Err(CodegenError::internal("math argument is not REAL"));
        };
        let name = self.float_intrinsic_name(kind, width);
        let ty = facts.llvm_value.clone();
        self.module
            .declare(name, &format!("{} @{name}({})", ty.render(), ty.render()));
        f.call(ty, name, std::slice::from_ref(&value))
            .ok_or_else(|| CodegenError::internal("math intrinsic returned void"))
    }

    fn lower_float_binary(
        &mut self,
        f: &mut Function,
        kind: stc_hir::StdFnKind,
        facts: &crate::types::TypeFacts,
        lhs: Value,
        rhs: Value,
    ) -> Res<Value> {
        let crate::layout::ScalarKind::Real { width } = facts
            .scalar()
            .ok_or_else(|| CodegenError::internal("math argument is not REAL"))?
        else {
            return Err(CodegenError::internal("math argument is not REAL"));
        };
        let name = self.float_intrinsic_name(kind, width);
        let ty = facts.llvm_value.clone();
        self.module.declare(
            name,
            &format!("{} @{name}({}, {})", ty.render(), ty.render(), ty.render()),
        );
        f.call(ty, name, &[lhs, rhs])
            .ok_or_else(|| CodegenError::internal("math intrinsic returned void"))
    }

    fn lower_trunc(
        &mut self,
        f: &mut Function,
        facts: &crate::types::TypeFacts,
        value: Value,
        span: Span,
    ) -> Res<Value> {
        let crate::layout::ScalarKind::Real { .. } = facts
            .scalar()
            .ok_or_else(|| CodegenError::internal("TRUNC argument is not REAL"))?
        else {
            return Err(CodegenError::internal("TRUNC argument is not REAL"));
        };
        let (neg_limit, pos_limit) = if facts.llvm_value == IrType::Float {
            (
                Value::float32(-2_147_483_649.0),
                Value::float32(2_147_483_648.0),
            )
        } else {
            (
                Value::double(-2_147_483_649.0),
                Value::double(2_147_483_648.0),
            )
        };
        let above = f.fcmp("ogt", &value, &neg_limit);
        let below = f.fcmp("olt", &value, &pos_limit);
        let ordered = f.fcmp("ord", &value, &value);
        let in_range = f.binop("and", &above, &below);
        let valid = f.binop("and", &in_range, &ordered);
        let bad = f.binop("xor", &valid, &Value::bool(true));
        self.trap_if(f, &bad, trap::CONVERSION, span)?;
        Ok(f.emit_value_cast("fptosi", &value, IrType::Int(32)))
    }
}
