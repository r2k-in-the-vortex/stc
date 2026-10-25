//! M5 phase 4: object tables, FB invocation, and method calls.
//!
//! Class/FB instances are statically known and their persistent fields are
//! host roots. A call into an FB body or method receives a `[N x ptr]` table
//! of the instance's leaf roots, ordered by the object type's base-first leaf
//! layout (D-131, D-133). Object assignment copies state per field.

use stc_hir::{ArgDirection, ArgHir, DefId, ExprId, FieldId, TyId};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;
use crate::expr::{BodyInfo, Res};
use crate::ir::{Function, IrType, Value};
use crate::module::FunctionTarget;
use crate::place::{PlacePath, field_table_pointer};
use crate::unit::Lowerer;

impl Lowerer<'_> {
    /// Builds the root table for one class/FB instance.
    pub(crate) fn object_table(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        instance: ExprId,
        target: DefId,
        span: Span,
    ) -> Res<Value> {
        let Some(path) = self.collect_place(instance) else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "an object receiver must be a place".into(),
                span,
            ));
        };
        if matches!(path.base, crate::place::PlaceBase::This(_))
            && path.steps.is_empty()
            && let Some(fields) = &info.fields
        {
            return Ok(fields.pointer.clone());
        }
        let transient = matches!(
            path.base,
            crate::place::PlaceBase::Local(_)
                | crate::place::PlaceBase::Param(_)
                | crate::place::PlaceBase::Result(_)
        );
        let prefix = if transient {
            Vec::new()
        } else {
            self.static_segments(&path).ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "a runtime-indexed object receiver is not supported in M5".into(),
                    span,
                )
            })?
        };
        let transient_pointer = if transient {
            Some(
                self.transient_place_pointer(f, info, &path)?
                    .ok_or_else(|| CodegenError::internal("local object has no storage"))?
                    .pointer,
            )
        } else {
            None
        };
        let layout = self
            .objects
            .get(&target)
            .cloned()
            .ok_or_else(|| CodegenError::internal("object type has no leaf layout"))?;
        let table_type = format!("[{} x ptr]", layout.leaves.len());
        let table = f.alloca_typed(&table_type);
        for (index, leaf) in layout.leaves.iter().enumerate() {
            let mut full = prefix.clone();
            full.extend(leaf.segments.iter().copied());
            let pointer = match &path.base {
                crate::place::PlaceBase::This(_) | crate::place::PlaceBase::Instance { .. } => {
                    let Some(fields) = &info.fields else {
                        return Err(CodegenError::internal(
                            "an object receiver outside an instance body",
                        ));
                    };
                    let Some(ordinal) = fields.ordinals.get(&full).copied() else {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "a call through a nested class instance is M5 phase 4b.3".into(),
                            span,
                        ));
                    };
                    field_table_pointer(f, fields, ordinal)
                }
                crate::place::PlaceBase::Global(def) => {
                    let target = Self::global_leaf_target(*def, &full, leaf.class_handle.is_some());
                    let Some(slot) = self.got.root_slot(&target) else {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "a call through a nested class instance is M5 phase 4b.3".into(),
                            span,
                        ));
                    };
                    Self::slot_storage(f, info, slot)
                }
                crate::place::PlaceBase::Local(_)
                | crate::place::PlaceBase::Param(_)
                | crate::place::PlaceBase::Result(_) => {
                    let base_pointer = transient_pointer.as_ref().ok_or_else(|| {
                        CodegenError::internal("transient object pointer missing")
                    })?;
                    self.aggregate_pointer(f, path.ty, base_pointer, &leaf.segments)?
                }
            };
            let entry = f.gep(
                &table_type,
                &table,
                &["i64 0".into(), format!("i64 {index}")],
                true,
            );
            f.store(&pointer, &entry, Some(8));
        }
        Ok(table)
    }

    /// Extends a place path with one field.
    pub(crate) fn field_path(&self, path: &PlacePath, field: FieldId, span: Span) -> PlacePath {
        let mut steps = path.steps.clone();
        steps.push(crate::place::PlaceStep::Field(field));
        PlacePath {
            base: path.base.clone(),
            steps,
            ty: self.hir.field(field).ty,
            span,
        }
    }

    /// Reads a class/FB field through its place path.
    pub(crate) fn read_object_field(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
    ) -> Res<Value> {
        let facts = self.types.facts(path.ty)?.clone();
        if let Some(kind) = facts.scalar()
            && let Some(resolved) = self.resolve_place_pointer(f, info, path)?
        {
            if let Some(address) = &resolved.direct {
                let bit = crate::place::direct_bit(address, kind)?;
                return self.read_direct_pointer(f, &resolved.pointer, kind, bit);
            }
            return Self::read_scalar(f, &resolved.pointer, kind);
        }
        self.read_aggregate_place(f, info, path)
    }

    /// Writes a class/FB field through its place path.
    pub(crate) fn write_object_field(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
        value: Value,
    ) -> Res<()> {
        let facts = self.types.facts(path.ty)?.clone();
        if let Some(kind) = facts.scalar() {
            let Some(resolved) = self.resolve_place_pointer(f, info, path)? else {
                return Err(CodegenError::internal("object field has no storage"));
            };
            if let Some(address) = &resolved.direct {
                let bit = crate::place::direct_bit(address, kind)?;
                self.check_subrange(f, path.ty, &value, path.span)?;
                return self.write_direct_pointer(f, &resolved.pointer, kind, value, bit);
            }
            self.check_subrange(f, path.ty, &value, path.span)?;
            Self::write_scalar(f, &resolved.pointer, kind, &value);
            return Ok(());
        }
        if let Some(resolved) = self.resolve_place_pointer(f, info, path)? {
            f.store(&value, &resolved.pointer, None);
            return Ok(());
        }
        let storage = self.types.llvm_storage(path.ty)?;
        let temp = f.alloca(storage.clone());
        f.store(&value, &temp, None);
        self.split_place_from_temp(f, info, path, &temp)
    }

    /// Lowers an FB invocation (`instance(...)` or `SUPER(...)`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_fb_call(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        instance: ExprId,
        def: DefId,
        base: Option<DefId>,
        args: &[ArgHir],
        span: Span,
    ) -> Res<Option<Value>> {
        let Some(path) = self.collect_place(instance) else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "an FB instance must be a place".into(),
                span,
            ));
        };
        let table = self.object_table(f, info, instance, def, span)?;
        for arg in args {
            if !matches!(arg.direction, ArgDirection::Input | ArgDirection::InOut) {
                continue;
            }
            let field = arg
                .field
                .ok_or_else(|| CodegenError::internal("FB input argument has no field"))?;
            let value_expr = arg
                .value
                .ok_or_else(|| CodegenError::internal("FB input argument has no value"))?;
            let value = self.lower_expr(f, info, value_expr)?;
            let field_path = self.field_path(&path, field, span);
            self.write_object_field(f, info, &field_path, value)?;
        }
        let body = base.unwrap_or(def);
        let slot = self.function_slot(FunctionTarget::FbBody(body))?;
        let callee = Self::slot_storage(f, info, slot);
        f.call_ptr(IrType::Void, &callee, &[table, info.slots.clone()]);
        for arg in args {
            if arg.direction != ArgDirection::Output {
                continue;
            }
            let field = arg
                .field
                .ok_or_else(|| CodegenError::internal("FB output argument has no field"))?;
            let field_path = self.field_path(&path, field, span);
            let value = self.read_object_field(f, info, &field_path)?;
            let Some(target) = arg.target else {
                continue;
            };
            self.write_place(f, info, target, value)?;
            if arg.negated {
                self.apply_output_negation(f, info, target, span)?;
            }
        }
        Ok(None)
    }

    /// Lowers a `SUPER()` body call on the current instance.
    pub(crate) fn lower_super_body(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        instance: ExprId,
        base: DefId,
        args: &[ArgHir],
        span: Span,
    ) -> Res<Option<Value>> {
        self.lower_fb_call(f, info, instance, base, Some(base), args, span)
    }

    /// Placeholder for interface dispatch (phase 4b).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_interface_method(
        &mut self,
        _f: &mut Function,
        _info: &BodyInfo,
        _receiver: ExprId,
        _interface: DefId,
        _method: DefId,
        _args: &[ArgHir],
        _supplied: &[Value],
        _result_ty: TyId,
        span: Span,
        _discard: bool,
    ) -> Res<Option<Value>> {
        Err(CodegenError::unsupported(
            codes::CODEGEN_UNSUPPORTED,
            "interface dispatch is M5 phase 4b".into(),
            span,
        ))
    }
}
