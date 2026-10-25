//! M5 phase 4/4b.3: object records, FB invocation, and method dispatch.
//!
//! A class value is a handle to a host-owned (or activation-owned) object
//! record; an FB value is an inline instance with a record used by the body
//! ABI. Records are `{ typeinfo, leaf[N] }` where the tail is the base-first
//! leaf storage table (GOT Artifacts §8.5.1, §8.5.6). FB bodies and methods
//! receive `(handle, table, ...)`; class methods dispatch through the
//! TYPEINFO's class VTABLE (D-147–D-152).

use stc_hir::{ArgDirection, ArgHir, DefId, DefKind, ExprId, FieldId, TyId};
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
        self.call_vtable_slot(f, &vtable, slot, ret, call_args, span)
    }

    /// Emits one interface-method dispatch through an interface value's
    /// per-(object type, interface) VTABLE (GOT Artifacts §8.5.4).
    ///
    /// The interface value's record and vtable are both null-checked; the
    /// pair is passed as `(record, record + 8, ...)` like any object body.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_interface_method(
        &mut self,
        f: &mut Function,
        record: &Value,
        vtable: &Value,
        slot: u32,
        ret: IrType,
        call_args: &[Value],
        span: Span,
    ) -> Res<Option<Value>> {
        self.check_pointer(f, record, span)?;
        self.check_pointer(f, vtable, span)?;
        self.call_vtable_slot(f, vtable, slot, ret, call_args, span)
    }

    /// Loads a VTABLE slot's host function cell and calls the cell's code.
    fn call_vtable_slot(
        &mut self,
        f: &mut Function,
        vtable: &Value,
        slot: u32,
        ret: IrType,
        call_args: &[Value],
        span: Span,
    ) -> Res<Option<Value>> {
        let entry_index = 6_u64.saturating_add(u64::from(slot));
        let entry = f.gep("i64", vtable, &[format!("i64 {entry_index}")], false);
        let cell = f.vtable_cell(&entry);
        self.check_pointer(f, &cell, span)?;
        let code = f.vtable_code(&cell);
        self.check_pointer(f, &code, span)?;
        Ok(f.call_ptr(ret, &code, call_args))
    }

    /// Lowers a class/FB-to-interface upcast (D-148, D-149).
    ///
    /// The source type's compile-time base-first interface map index stays
    /// valid for any derived runtime object because inherited entries keep
    /// their indices. The runtime TYPEINFO supplies the map's base offset
    /// (`72 + 16 * base_count`) and the per-(object type, interface) VTABLE
    /// pointer; a null class handle yields the all-zero null interface.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn upcast_object_to_interface(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        value: Value,
        def: DefId,
        target: DefId,
        pair_ty: &IrType,
        class: bool,
        operand: Option<ExprId>,
        span: Span,
    ) -> Res<Value> {
        let index = self
            .types
            .interface_map(def)?
            .iter()
            .position(|entry| entry.def == target)
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "`{}` does not satisfy the target interface",
                    self.hir.def_name(def)
                ))
            })?;
        let index = u64::try_from(index).unwrap_or(u64::MAX);
        let record = if class {
            value
        } else {
            let operand = operand.ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "an FB-to-interface upcast requires an instance place".into(),
                    span,
                )
            })?;
            let path = self.collect_place(operand).ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "an FB-to-interface upcast requires an instance place".into(),
                    span,
                )
            })?;
            match self.resolve_place_value(f, info, &path)? {
                Some(PlaceValue::FbRecord(record)) => record,
                _ => {
                    return Err(CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "an FB-to-interface upcast requires an instance record".into(),
                        span,
                    ));
                }
            }
        };
        let result = f.alloca(pair_ty.clone());
        f.store(&Value::zero(pair_ty.clone()), &result, None);
        let upcast = f.next_block_label();
        let done = f.next_block_label();
        let is_null = f.icmp("eq", &record, &Value::null_ptr());
        f.cond_branch(&is_null, &done, &upcast);
        f.start_block(&upcast);
        let vtable = self.interface_map_vtable(f, &record, index, span)?;
        let pair = f.insert_value_at(
            &Value::zero(pair_ty.clone()),
            &record,
            &[0],
            pair_ty.clone(),
        );
        let pair = f.insert_value_at(&pair, &vtable, &[1], pair_ty.clone());
        f.store(&pair, &result, None);
        f.branch(&done);
        f.start_block(&done);
        Ok(f.load(pair_ty.clone(), &result, None))
    }

    /// Lowers an interface-to-interface upcast by a bounded search of the
    /// object TYPEINFO's base-first interface map (D-148).
    ///
    /// A null source yields the null interface. A missing target entry (which
    /// a conforming descriptor cannot produce) also leaves the null interface
    /// rather than trapping.
    pub(crate) fn upcast_interface_to_interface(
        &mut self,
        f: &mut Function,
        value: Value,
        target: DefId,
        pair_ty: &IrType,
        span: Span,
    ) -> Res<Value> {
        let record = f.extract_value_at(&value, &[0], IrType::Ptr);
        let result = f.alloca(pair_ty.clone());
        f.store(&Value::zero(pair_ty.clone()), &result, None);
        let search = f.next_block_label();
        let done = f.next_block_label();
        let is_null = f.icmp("eq", &record, &Value::null_ptr());
        f.cond_branch(&is_null, &done, &search);
        f.start_block(&search);
        self.store_interface_vtable_by_id(f, &record, target, pair_ty, &result, span)?;
        f.branch(&done);
        f.start_block(&done);
        Ok(f.load(pair_ty.clone(), &result, None))
    }

    /// Loads the per-interface VTABLE pointer at a compile-time map index.
    fn interface_map_vtable(
        &mut self,
        f: &mut Function,
        record: &Value,
        index: u64,
        span: Span,
    ) -> Res<Value> {
        let typeinfo = f.load(IrType::Ptr, record, Some(8));
        self.check_pointer(f, &typeinfo, span)?;
        let base_count_ptr = f.gep_bytes(&typeinfo, 16);
        let base_count = f.load(IrType::Int(32), &base_count_ptr, None);
        let base_bytes = f.emit_value_cast("zext", &base_count, IrType::Int(64));
        let base_offset = f.binop("mul", &base_bytes, &Value::int(64, 16));
        let entry_index = 72_u64.saturating_add(24_u64.saturating_mul(index));
        let offset = f.binop(
            "add",
            &base_offset,
            &Value::int(64, i128::from(entry_index)),
        );
        let entry = f.gep("i8", &typeinfo, &[format!("i64 {}", offset.repr)], false);
        let vtable_ptr = f.gep_bytes(&entry, 16);
        Ok(f.load(IrType::Ptr, &vtable_ptr, Some(8)))
    }

    /// Searches the object TYPEINFO's interface map by interface `GotId` and
    /// stores `{ record, vtable }` into `result` when found.
    #[allow(clippy::too_many_arguments)]
    fn store_interface_vtable_by_id(
        &mut self,
        f: &mut Function,
        record: &Value,
        target: DefId,
        pair_ty: &IrType,
        result: &Value,
        span: Span,
    ) -> Res<()> {
        let id = self.types.named(target)?.id.as_bytes();
        let low = u64::from_le_bytes(id[..8].try_into().expect("eight id bytes"));
        let high = u64::from_le_bytes(id[8..].try_into().expect("eight id bytes"));
        let typeinfo = f.load(IrType::Ptr, record, Some(8));
        self.check_pointer(f, &typeinfo, span)?;
        let base_count_ptr = f.gep_bytes(&typeinfo, 16);
        let base_count = f.load(IrType::Int(32), &base_count_ptr, None);
        let interface_count_ptr = f.gep_bytes(&typeinfo, 20);
        let interface_count = f.load(IrType::Int(32), &interface_count_ptr, None);
        let base_bytes = f.emit_value_cast("zext", &base_count, IrType::Int(64));
        let base_offset = f.binop("mul", &base_bytes, &Value::int(64, 16));
        let map_offset = f.binop("add", &base_offset, &Value::int(64, 72));
        let map = f.gep(
            "i8",
            &typeinfo,
            &[format!("i64 {}", map_offset.repr)],
            false,
        );

        let index_slot = f.alloca(IrType::Int(32));
        f.store(&Value::int(32, 0), &index_slot, None);
        let header = f.next_block_label();
        let body = f.next_block_label();
        let found = f.next_block_label();
        let next = f.next_block_label();
        let search_done = f.next_block_label();
        f.branch(&header);

        f.start_block(&header);
        let index = f.load(IrType::Int(32), &index_slot, None);
        let in_range = f.icmp("ult", &index, &interface_count);
        f.cond_branch(&in_range, &body, &search_done);

        f.start_block(&body);
        let index64 = f.emit_value_cast("zext", &index, IrType::Int(64));
        let stride = f.binop("mul", &index64, &Value::int(64, 24));
        let entry = f.gep("i8", &map, &[format!("i64 {}", stride.repr)], false);
        let low_word = f.load(IrType::Int(64), &entry, Some(8));
        let high_word_ptr = f.gep_bytes(&entry, 8);
        let high_word = f.load(IrType::Int(64), &high_word_ptr, Some(8));
        let low_matches = f.icmp("eq", &low_word, &Value::int(64, i128::from(low)));
        let high_matches = f.icmp("eq", &high_word, &Value::int(64, i128::from(high)));
        let matches = f.binop("and", &low_matches, &high_matches);
        f.cond_branch(&matches, &found, &next);

        f.start_block(&found);
        let vtable_ptr = f.gep_bytes(&entry, 16);
        let vtable = f.load(IrType::Ptr, &vtable_ptr, Some(8));
        let pair = f.insert_value_at(&Value::zero(pair_ty.clone()), record, &[0], pair_ty.clone());
        let pair = f.insert_value_at(&pair, &vtable, &[1], pair_ty.clone());
        f.store(&pair, result, None);
        f.branch(&search_done);

        f.start_block(&next);
        let next_index = f.binop("add", &index, &Value::int(32, 1));
        f.store(&next_index, &index_slot, None);
        f.branch(&header);

        f.start_block(&search_done);
        Ok(())
    }

    /// Lowers an interface-method call.
    ///
    /// The receiver is an interface value `{ record, per-pair vtable }`; the
    /// record and its tail table are passed to the implementation like any
    /// object body (GOT Artifacts §8.5.4, §8.5.8).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_interface_method(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        receiver: ExprId,
        interface: DefId,
        method: DefId,
        args: &[ArgHir],
        supplied: &[Value],
        _result_ty: TyId,
        span: Span,
        discard: bool,
    ) -> Res<Option<Value>> {
        let DefKind::Method { params, result, .. } = self.hir.defs[method.index()].kind.clone()
        else {
            return Err(CodegenError::internal("callee is not a method"));
        };
        let receiver_ty = self.hir.expr(receiver).ty;
        let pair_ty = self.types.facts(receiver_ty)?.llvm_value.clone();
        if !pair_ty.is_aggregate() {
            return Err(CodegenError::internal(
                "an interface receiver is not a two-pointer value",
            ));
        }
        let pair = self.lower_expr(f, info, receiver)?;
        let record = f.extract_value_at(&pair, &[0], IrType::Ptr);
        let vtable = f.extract_value_at(&pair, &[1], IrType::Ptr);
        let slot = self.method_slot(interface, method)?;
        let table = Self::object_table_pointer(f, &record);
        self.lower_callable(
            f,
            info,
            &params,
            result,
            args,
            supplied,
            vec![record, table],
            crate::call::CallTarget::Interface { vtable, slot },
            span,
            discard,
        )
    }
}
