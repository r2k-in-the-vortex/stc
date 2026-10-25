//! M5 phase 4/4b.3: object records, FB invocation, and method dispatch.
//!
//! A class value is a handle to a host-owned (or activation-owned) object
//! record; an FB value is an inline instance with a record used by the body
//! ABI. Records are `{ typeinfo, leaf[N] }` where the tail is the base-first
//! leaf storage table (GOT Artifacts §8.5.1, §8.5.6). FB bodies and methods
//! receive `(handle, table, ...)`; class methods dispatch through the
//! TYPEINFO's class VTABLE (D-147–D-152).

use stc_hir::{ArgDirection, ArgHir, DefId, ExprId, FieldId, TyId};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;
use crate::expr::{BodyInfo, Res};
use crate::ir::{Function, IrType, Value};
use crate::module::FunctionTarget;
use crate::place::{PlacePath, PlaceValue};
use crate::unit::Lowerer;

impl Lowerer<'_> {
    /// Resolves an object receiver to its record pointer.
    ///
    /// Class receivers produce the loaded handle; FB receivers produce the
    /// instance record.
    pub(crate) fn object_receiver(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        receiver: ExprId,
        span: Span,
    ) -> Res<Value> {
        let receiver_ty = self.hir.expr(receiver).ty;
        let receiver_facts = self.types.facts(receiver_ty)?.clone();
        let Some(path) = self.collect_place(receiver) else {
            if receiver_facts.is_class() {
                // A class-typed value (for example a function result) already
                // is the handle; no place storage is needed for dispatch.
                let handle = self.lower_expr(f, info, receiver)?;
                self.check_pointer(f, &handle, span)?;
                return Ok(handle);
            }
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "an FB receiver must be a place".into(),
                span,
            ));
        };
        let facts = self.types.facts(path.ty)?.clone();
        let Some(value) = self.resolve_place_value(f, info, &path)? else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "the object receiver has no stable storage".into(),
                span,
            ));
        };
        let handle = match value {
            PlaceValue::ClassValue(handle) => handle,
            PlaceValue::ClassSlot(slot) => f.load(IrType::Ptr, &slot, None),
            PlaceValue::FbRecord(record) => record,
            PlaceValue::Leaf(_) => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!(
                        "a `{}` receiver is not an object",
                        self.hir.type_name(path.ty)
                    ),
                    span,
                ));
            }
        };
        if facts.is_class() {
            self.check_pointer(f, &handle, span)?;
        }
        Ok(handle)
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
        self.read_place_path(f, info, path)
    }

    /// Writes a class/FB field through its place path.
    pub(crate) fn write_object_field(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        path: &PlacePath,
        value: Value,
    ) -> Res<()> {
        self.write_place_path(f, info, path, value)
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
        let record = self.object_receiver(f, info, instance, span)?;
        let table = Self::object_table_pointer(f, &record);
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
        f.call_ptr(IrType::Void, &callee, &[record, table, info.slots.clone()]);
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

    /// Returns the class-vtable slot assigned to a method.
    pub(crate) fn method_slot(&self, owner: DefId, method: DefId) -> Res<u32> {
        let named = self.types.named(owner)?;
        named
            .vtable
            .iter()
            .find(|entry| entry.def == method)
            .map(|entry| entry.slot)
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "method `{}` has no vtable slot in `{}`",
                    self.hir.def_name(method),
                    self.hir.def_name(owner)
                ))
            })
    }

    /// Emits one virtual class/FB method dispatch.
    ///
    /// The record's TYPEINFO supplies the class VTABLE; the slot names a host
    /// function cell, and the cell's value is the code address. Every
    /// dereference traps `STC-TRAP_NULL_REF` (GOT Artifacts §8.5.3, §8.5.8).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_virtual_method(
        &mut self,
        f: &mut Function,
        handle: &Value,
        slot: u32,
        ret: IrType,
        call_args: &[Value],
        span: Span,
    ) -> Res<Option<Value>> {
        self.check_pointer(f, handle, span)?;
        let typeinfo = f.load(IrType::Ptr, handle, Some(8));
        self.check_pointer(f, &typeinfo, span)?;
        let vtable_ptr = f.gep("i64", &typeinfo, &["i64 8".into()], false);
        let vtable = f.load(IrType::Ptr, &vtable_ptr, Some(8));
        self.check_pointer(f, &vtable, span)?;
        let entry_index = 6_u64.saturating_add(u64::from(slot));
        let entry = f.gep("i64", &vtable, &[format!("i64 {entry_index}")], false);
        let cell = f.vtable_cell(&entry);
        self.check_pointer(f, &cell, span)?;
        let code = f.vtable_code(&cell);
        self.check_pointer(f, &code, span)?;
        Ok(f.call_ptr(ret, &code, call_args))
    }

    /// Placeholder for interface dispatch (phase 4b.4).
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
