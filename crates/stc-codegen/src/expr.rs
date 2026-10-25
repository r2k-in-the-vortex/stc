//! Expression lowering, scalar operators, checked arithmetic, and casts.
//!
//! Expressions are evaluated with an explicit work stack so a 20k-term
//! left-associative binary chain never recurses (D-073, handover §6.18).

use std::collections::BTreeMap;

use stc_hir::{
    BinOp, CalleeHir, ConstValue, DefId, DefKind, DirectAddress, DirectArea, ExprId, ExprKindHir,
    HirCrate, LocalId, ParamId, PlaceHir, TyId, UnOp,
};
use stc_span::Span;

use crate::CodegenError;
use crate::abi::trap;
use crate::codes;
use crate::ir::{Function, IrType, Module, SymbolAttrs, Value};
use crate::layout::{self, ScalarKind};
use crate::module::{FunctionTarget, RootTarget};
use crate::unit::Lowerer;

/// Result alias for lowering.
pub(crate) type Res<T> = Result<T, CodegenError>;

/// Per-instance field storage resolved to host-cell pointers.
///
/// Program bodies do not know which configured instance they run for, so the
/// cycle resolver passes an array of storage pointers indexed by declaration
/// ordinal. Direct (`AT`) field pointers address the process image directly.
#[derive(Clone, Debug)]
pub(crate) struct FieldAccess {
    /// Pointer to an `[N x ptr]` array of leaf storage pointers.
    pub pointer: Value,
    /// Root segment path to leaf ordinal.
    pub ordinals: BTreeMap<Vec<crate::types::RootSegment>, u32>,
    /// Direct (`AT`) leaves and their process-image addresses.
    pub direct: BTreeMap<Vec<crate::types::RootSegment>, stc_hir::DirectAddress>,
}

/// Per-function lowering context.
#[derive(Clone, Debug)]
pub(crate) struct BodyInfo {
    /// Parameter allocas.
    pub params: BTreeMap<ParamId, Value>,
    /// Local allocas.
    pub locals: BTreeMap<LocalId, Value>,
    /// Result alloca.
    pub result_ptr: Option<Value>,
    /// Result storage type.
    pub result_ty: Option<IrType>,
    /// Slot-table pointer for GOT access.
    pub slots: Value,
    /// Program-instance field access, when the body runs for an instance.
    pub fields: Option<FieldAccess>,
    /// Return/exit block label.
    pub return_label: String,
}

#[derive(Clone, Debug)]
enum Task {
    Eval(ExprId),
    Apply(ExprId),
    ShortAfterLhs(ExprId),
    FinishShort {
        merge: String,
        short_block: String,
        short_value: Value,
    },
}

impl Lowerer<'_> {
    pub(crate) fn scalar_at(&self, ty: TyId, span: Span) -> Res<ScalarKind> {
        self.types.scalar_kind(ty, span)
    }

    /// Lowers one expression to an SSA value.
    pub(crate) fn lower_expr(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
    ) -> Res<Value> {
        let mut tasks = vec![Task::Eval(id)];
        let mut values: Vec<Value> = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Eval(id) => self.eval_task(f, info, id, &mut tasks, &mut values)?,
                Task::Apply(id) => self.apply_task(f, info, id, &mut values)?,
                Task::ShortAfterLhs(id) => {
                    self.short_after_lhs(f, info, id, &mut tasks, &mut values)?;
                }
                Task::FinishShort {
                    merge,
                    short_block,
                    short_value,
                } => {
                    let rhs = values.pop().ok_or_else(|| {
                        CodegenError::internal("missing right-hand value for && / ||")
                    })?;
                    let rhs_pred = f.current_label().to_string();
                    f.branch(&merge);
                    f.start_block(&merge);
                    let result = f.phi(
                        IrType::Int(1),
                        &[(short_value, short_block), (rhs, rhs_pred)],
                    );
                    values.push(result);
                }
            }
        }
        if values.len() != 1 {
            return Err(CodegenError::internal(
                "expression lowering did not produce one value",
            ));
        }
        values
            .pop()
            .ok_or_else(|| CodegenError::internal("missing expression value"))
    }

    fn eval_task(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Value>,
    ) -> Res<()> {
        let expr = self.hir.expr(id).clone();
        match expr.kind {
            ExprKindHir::Literal(literal) => {
                let value = self.lower_literal(f, expr.ty, &literal.value, expr.span)?;
                values.push(value);
            }
            ExprKindHir::Place(_) => {
                let value = self.read_place(f, info, id)?;
                values.push(value);
            }
            ExprKindHir::Binary { op, lhs, rhs } => {
                if matches!(op, BinOp::And | BinOp::Or)
                    && self.scalar_at(self.hir.expr(lhs).ty, expr.span)? == ScalarKind::Bool
                {
                    tasks.push(Task::ShortAfterLhs(id));
                    tasks.push(Task::Eval(lhs));
                } else {
                    tasks.push(Task::Apply(id));
                    tasks.push(Task::Eval(rhs));
                    tasks.push(Task::Eval(lhs));
                }
            }
            ExprKindHir::Unary { operand, .. } => {
                tasks.push(Task::Apply(id));
                tasks.push(Task::Eval(operand));
            }
            ExprKindHir::Cast { operand, .. } => {
                tasks.push(Task::Apply(id));
                tasks.push(Task::Eval(operand));
            }
            ExprKindHir::Call { callee, args } => {
                match &callee {
                    CalleeHir::Function(_)
                    | CalleeHir::Conversion { .. }
                    | CalleeHir::StdFunction(_) => {}
                    other => {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            format!(
                                "callee `{}` is not supported in M4",
                                callee_name(self.hir, other)
                            ),
                            expr.span,
                        ));
                    }
                }
                tasks.push(Task::Apply(id));
                for arg in args.iter().rev() {
                    if arg.direction == stc_hir::ArgDirection::Input
                        && let Some(value) = arg.value
                    {
                        tasks.push(Task::Eval(value));
                    }
                }
            }
            ExprKindHir::Ref { target } => {
                let value = self.lower_ref(f, info, target, expr.span)?;
                values.push(value);
            }
            ExprKindHir::Null => {
                let ty = self.types.llvm_value(expr.ty)?;
                values.push(Value::zero(ty));
            }
            ExprKindHir::ArrayInit { elements } => {
                tasks.push(Task::Apply(id));
                for element in elements.iter().rev() {
                    tasks.push(Task::Eval(*element));
                }
            }
            ExprKindHir::StructInit { fields } => {
                tasks.push(Task::Apply(id));
                for (_, field) in fields.iter().rev() {
                    tasks.push(Task::Eval(*field));
                }
            }
            ExprKindHir::Error => {
                return Err(CodegenError::internal(
                    "an error expression reached code generation",
                ));
            }
        }
        Ok(())
    }

    fn short_after_lhs(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Value>,
    ) -> Res<()> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Binary { op, lhs: _, rhs } = expr.kind else {
            return Err(CodegenError::internal("short-circuit task on a non-binary"));
        };
        let lhs = values
            .pop()
            .ok_or_else(|| CodegenError::internal("missing left-hand value for && / ||"))?;
        let short_block = f.next_block_label();
        let rhs_block = f.next_block_label();
        let merge = f.next_block_label();
        if op == BinOp::And {
            f.cond_branch(&lhs, &rhs_block, &short_block);
        } else {
            f.cond_branch(&lhs, &short_block, &rhs_block);
        }
        f.start_block(&short_block);
        let short_value = Value::bool(op == BinOp::Or);
        f.branch(&merge);
        f.start_block(&rhs_block);
        tasks.push(Task::FinishShort {
            merge,
            short_block,
            short_value,
        });
        tasks.push(Task::Eval(rhs));
        let _ = info;
        Ok(())
    }

    fn apply_task(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
        values: &mut Vec<Value>,
    ) -> Res<()> {
        let expr = self.hir.expr(id).clone();
        match expr.kind {
            ExprKindHir::Binary { op, lhs, rhs } => {
                let rhs_value = values
                    .pop()
                    .ok_or_else(|| CodegenError::internal("missing binary right operand"))?;
                let lhs_value = values
                    .pop()
                    .ok_or_else(|| CodegenError::internal("missing binary left operand"))?;
                let value =
                    self.lower_binary(f, info, op, lhs, rhs, lhs_value, rhs_value, expr.span)?;
                values.push(value);
            }
            ExprKindHir::Unary { op, operand } => {
                let operand_value = values
                    .pop()
                    .ok_or_else(|| CodegenError::internal("missing unary operand"))?;
                let value = self.lower_unary(f, op, operand, operand_value, expr.span)?;
                values.push(value);
            }
            ExprKindHir::Cast { operand, to, .. } => {
                let operand_value = values
                    .pop()
                    .ok_or_else(|| CodegenError::internal("missing cast operand"))?;
                let from_ty = self.hir.expr(operand).ty;
                let from = self.types.facts(from_ty)?.scalar();
                let target = self.types.facts(to)?.scalar();
                let value = match (from, target) {
                    (Some(from), Some(target)) => {
                        self.cast_value(f, operand_value, from, target, expr.span)?
                    }
                    _ => self.lower_aggregate_cast(f, operand_value, from_ty, to, expr.span)?,
                };
                self.check_subrange(f, to, &value, expr.span)?;
                values.push(value);
            }
            ExprKindHir::ArrayInit { elements } => {
                let supplied = values.split_off(values.len() - elements.len());
                let value = self.build_array_init(f, expr.ty, &elements, &supplied, expr.span)?;
                values.push(value);
            }
            ExprKindHir::StructInit { fields } => {
                let supplied = values.split_off(values.len() - fields.len());
                let value = self.build_struct_init(f, expr.ty, &fields, &supplied, expr.span)?;
                values.push(value);
            }
            ExprKindHir::Call { callee, args } => {
                let provided = args
                    .iter()
                    .filter(|arg| {
                        arg.direction == stc_hir::ArgDirection::Input && arg.value.is_some()
                    })
                    .count();
                if values.len() < provided {
                    return Err(CodegenError::internal("missing call argument value"));
                }
                let supplied = values.split_off(values.len() - provided);
                let value =
                    self.lower_call(f, info, &callee, &args, &supplied, expr.ty, expr.span)?;
                values.push(value);
            }
            other => {
                return Err(CodegenError::internal(format!(
                    "unexpected expression task {other:?}"
                )));
            }
        }
        Ok(())
    }

    fn lower_literal(
        &mut self,
        f: &mut Function,
        ty: TyId,
        value: &ConstValue,
        span: Span,
    ) -> Res<Value> {
        if let ConstValue::String(units) = value {
            return self.lower_string_literal(f, ty, units, span);
        }
        if let ConstValue::Null = value {
            let llvm = self.types.llvm_value(ty)?;
            return Ok(Value::zero(llvm));
        }
        let kind = self.scalar_at(ty, span)?;
        let integer = match value {
            ConstValue::Int(value) | ConstValue::Time(value) | ConstValue::Date { value, .. } => {
                Some(*value)
            }
            ConstValue::Char(value) => Some(i128::from(*value)),
            ConstValue::Bool(value) => Some(i128::from(u8::from(*value))),
            ConstValue::Enum { value, .. } => Some(*value),
            ConstValue::String(_) | ConstValue::Null => None,
            ConstValue::Real(_) => None,
        };
        match kind {
            ScalarKind::Bool => match value {
                ConstValue::Bool(value) => Ok(Value::bool(*value)),
                _ => Ok(Value::bool(integer.unwrap_or(0) != 0)),
            },
            ScalarKind::Real { width: 32 } => {
                let number = match value {
                    ConstValue::Real(value) => *value as f32,
                    _ => integer.unwrap_or(0) as f32,
                };
                Ok(Value::float32(number))
            }
            ScalarKind::Real { .. } => {
                let number = match value {
                    ConstValue::Real(value) => *value,
                    _ => integer.unwrap_or(0) as f64,
                };
                Ok(Value::double(number))
            }
            ScalarKind::Time | ScalarKind::Date(_) => Ok(Value::int(64, integer.unwrap_or(0))),
            ScalarKind::Int { width, .. } | ScalarKind::Char { width } => {
                Ok(Value::int(width, integer.unwrap_or(0)))
            }
        }
    }

    // ---- places -----------------------------------------------------------

    pub(crate) fn slot_storage(f: &mut Function, info: &BodyInfo, slot: u32) -> Value {
        f.slot_value(&info.slots, slot)
    }

    pub(crate) fn root_slot(&self, target: &RootTarget) -> Res<u32> {
        self.got
            .root_slot(target)
            .ok_or_else(|| CodegenError::internal(format!("no GOT slot for root {target:?}")))
    }

    pub(crate) fn function_slot(&self, target: FunctionTarget) -> Res<u32> {
        self.got
            .function_slot(target)
            .ok_or_else(|| CodegenError::internal(format!("no GOT slot for function {target:?}")))
    }

    pub(crate) fn import_slot(&self, service: stc_got::catalog::ImportService) -> Res<u32> {
        self.got.import_slot(service).ok_or_else(|| {
            CodegenError::internal(format!("no GOT import slot for service {service:?}"))
        })
    }

    fn direct_bit(address: &DirectAddress, kind: ScalarKind) -> Res<Option<u8>> {
        let span = layout::image_span(address, kind)?;
        Ok((kind == ScalarKind::Bool).then(|| u8::try_from(span.start_bit % 8).unwrap_or(0)))
    }

    pub(crate) fn read_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
    ) -> Res<Value> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Place(place) = expr.kind else {
            return Err(CodegenError::internal(
                "read_place on a non-place expression",
            ));
        };
        if let PlaceHir::BitField { base, bit } = place {
            return self.read_bitfield(f, info, base, bit, expr.span);
        }
        if let PlaceHir::Direct(address) = place {
            let kind = self.types.scalar_kind(expr.ty, expr.span)?;
            let pointer = self.image_service_pointer(f, info, &address, kind)?;
            let bit = Self::direct_bit(&address, kind)?;
            return self.read_direct_pointer(f, &pointer, kind, bit);
        }
        if let PlaceHir::Deref(reference) = place {
            return self.read_deref(f, info, reference, expr.ty, expr.span);
        }
        if let Some(path) = self.collect_place(id) {
            let facts = self.types.facts(path.ty)?.clone();
            if let Some(kind) = facts.scalar() {
                if let Some(resolved) = self.resolve_place_pointer(f, info, &path)? {
                    if let Some(address) = &resolved.direct {
                        let bit = Self::direct_bit(address, kind)?;
                        return self.read_direct_pointer(f, &resolved.pointer, kind, bit);
                    }
                    return Self::read_scalar(f, &resolved.pointer, kind);
                }
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("place of type `{}` has no storage", facts.key),
                    expr.span,
                ));
            }
            return self.read_aggregate_place(f, info, &path);
        }
        let kind = self.types.scalar_kind(expr.ty, expr.span)?;
        let pointer = self.place_ptr(f, info, id)?;
        Self::read_scalar(f, &pointer, kind)
    }

    pub(crate) fn write_place(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
        value: Value,
    ) -> Res<()> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Place(place) = expr.kind else {
            return Err(CodegenError::internal(
                "write_place on a non-place expression",
            ));
        };
        if let PlaceHir::BitField { base, bit } = place {
            return self.write_bitfield(f, info, base, bit, value, expr.span);
        }
        if let PlaceHir::Direct(address) = place {
            Self::reject_input_write(&address, expr.span)?;
            let kind = self.types.scalar_kind(expr.ty, expr.span)?;
            let pointer = self.image_service_pointer(f, info, &address, kind)?;
            let bit = Self::direct_bit(&address, kind)?;
            return self.write_direct_pointer(f, &pointer, kind, value, bit);
        }
        if let PlaceHir::Deref(reference) = place {
            return self.write_deref(f, info, reference, expr.ty, value, expr.span);
        }
        if let Some(path) = self.collect_place(id) {
            let facts = self.types.facts(path.ty)?.clone();
            if let Some(kind) = facts.scalar() {
                if let Some(resolved) = self.resolve_place_pointer(f, info, &path)? {
                    if let Some(address) = &resolved.direct {
                        Self::reject_input_write(address, expr.span)?;
                        let bit = Self::direct_bit(address, kind)?;
                        return self.write_direct_pointer(f, &resolved.pointer, kind, value, bit);
                    }
                    self.check_subrange(f, path.ty, &value, expr.span)?;
                    Self::write_scalar(f, &resolved.pointer, kind, &value);
                    return Ok(());
                }
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("place of type `{}` has no storage", facts.key),
                    expr.span,
                ));
            }
            return self.write_aggregate_place(f, info, &path, value);
        }
        let kind = self.types.scalar_kind(expr.ty, expr.span)?;
        let pointer = self.place_ptr(f, info, id)?;
        Self::write_scalar(f, &pointer, kind, &value);
        Ok(())
    }

    fn place_ptr(&mut self, f: &mut Function, info: &BodyInfo, id: ExprId) -> Res<Value> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Place(place) = expr.kind else {
            return Err(CodegenError::internal(
                "place_ptr on a non-place expression",
            ));
        };
        if let PlaceHir::Deref(reference) = place {
            return self.deref_pointer(f, info, reference, expr.span);
        }
        if let Some(path) = self.collect_place(id)
            && let Some(resolved) = self.resolve_place_pointer(f, info, &path)?
        {
            return Ok(resolved.pointer);
        }
        match place {
            PlaceHir::Result(_) => info.result_ptr.clone().ok_or_else(|| {
                CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "a program has no result place".into(),
                    expr.span,
                )
            }),
            PlaceHir::This(_) => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`THIS` has no whole-object storage pointer".into(),
                expr.span,
            )),
            PlaceHir::Index { .. } => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "array indexing has no contiguous storage here".into(),
                expr.span,
            )),
            PlaceHir::Deref(_) => Err(CodegenError::internal("dereference was not resolved")),
            PlaceHir::Direct(_) | PlaceHir::InstanceField { .. } | PlaceHir::Field { .. } => {
                Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "a direct-variable place has no ordinary storage".into(),
                    expr.span,
                ))
            }
            PlaceHir::Local(_) | PlaceHir::Param(_) | PlaceHir::Global(_) => {
                Err(CodegenError::internal("place pointer was not resolved"))
            }
            PlaceHir::BitField { .. } => Err(CodegenError::internal(
                "bit fields are handled by read/write_place",
            )),
        }
    }

    pub(crate) fn read_scalar(f: &mut Function, pointer: &Value, kind: ScalarKind) -> Res<Value> {
        let raw = f.load(kind.storage(), pointer, None);
        if kind == ScalarKind::Bool {
            Ok(f.icmp("ne", &raw, &Value::int(8, 0)))
        } else {
            Ok(raw)
        }
    }

    pub(crate) fn write_scalar(f: &mut Function, pointer: &Value, kind: ScalarKind, value: &Value) {
        let stored = if kind == ScalarKind::Bool {
            cast_raw(f, "zext", value, IrType::Int(8))
        } else {
            value.clone()
        };
        f.store(&stored, pointer, None);
    }

    fn read_bitfield(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        base: ExprId,
        bit: u16,
        span: Span,
    ) -> Res<Value> {
        let base_kind = self.scalar_at(self.hir.expr(base).ty, span)?;
        // `read_place` handles ordinary storage as well as direct/`AT` places,
        // so bit access works on `%I`/`%Q`/`%M` words as well as locals.
        let raw = self.read_place(f, info, base)?;
        if base_kind == ScalarKind::Bool {
            return Ok(raw);
        }
        let bits = value_bits(base_kind);
        let mask = Value::int(bits, 1 << bit);
        let masked = f.binop("and", &raw, &mask);
        Ok(f.icmp("ne", &masked, &Value::int(bits, 0)))
    }

    fn write_bitfield(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        base: ExprId,
        bit: u16,
        value: Value,
        span: Span,
    ) -> Res<()> {
        let base_kind = self.scalar_at(self.hir.expr(base).ty, span)?;
        if base_kind == ScalarKind::Bool {
            return self.write_place(f, info, base, value);
        }
        let raw = self.read_place(f, info, base)?;
        let bits = value_bits(base_kind);
        let clear_mask = Value::int(bits, !(1_i128 << bit));
        let cleared = f.binop("and", &raw, &clear_mask);
        let extended = cast_raw(f, "zext", &value, IrType::Int(bits));
        let shifted = f.binop("shl", &extended, &Value::int(bits, i128::from(bit)));
        let combined = f.binop("or", &cleared, &shifted);
        self.write_place(f, info, base, combined)
    }

    pub(crate) fn global_address(&self, def: DefId) -> Option<DirectAddress> {
        match &self.hir.defs[def.index()].kind {
            DefKind::Global { at, .. } => at.clone(),
            _ => None,
        }
    }

    fn area_service(area: DirectArea) -> i128 {
        match area {
            DirectArea::Input => 1,
            DirectArea::Output => 2,
            DirectArea::Memory => 3,
        }
    }

    /// Resolves an ad-hoc `%I`/`%Q`/`%M` expression through the `io_image`
    /// host service and returns the byte pointer.
    fn image_service_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        address: &DirectAddress,
        kind: ScalarKind,
    ) -> Res<Value> {
        let span = layout::image_span(address, kind)?;
        let Some(area) = address.area else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "a wildcard `AT` address is not supported in M4".into(),
                address.span,
            ));
        };
        let slot = self.import_slot(stc_got::catalog::ImportService::IoImage)?;
        let service = Self::slot_storage(f, info, slot);
        let base = f
            .call_ptr(
                IrType::Ptr,
                &service,
                &[Value::int(32, Self::area_service(area))],
            )
            .ok_or_else(|| CodegenError::internal("io_image service returned void"))?;
        let offset = i64::try_from(span.start_bit / 8).unwrap_or(i64::MAX);
        Ok(f.gep_bytes(&base, offset))
    }

    fn read_direct_pointer(
        &mut self,
        f: &mut Function,
        pointer: &Value,
        kind: ScalarKind,
        bit: Option<u8>,
    ) -> Res<Value> {
        if let Some(bit) = bit {
            let raw = f.load(IrType::Int(8), pointer, Some(1));
            let mask = Value::int(8, 1 << bit);
            let masked = f.binop("and", &raw, &mask);
            return Ok(f.icmp("ne", &masked, &Value::int(8, 0)));
        }
        Ok(f.load(kind.storage(), pointer, Some(1)))
    }

    fn write_direct_pointer(
        &mut self,
        f: &mut Function,
        pointer: &Value,
        kind: ScalarKind,
        value: Value,
        bit: Option<u8>,
    ) -> Res<()> {
        if let Some(bit) = bit {
            let raw = f.load(IrType::Int(8), pointer, Some(1));
            let mask = Value::int(8, 1 << bit);
            let clear_mask = Value::int(8, !(1_i128 << bit));
            let cleared = f.binop("and", &raw, &clear_mask);
            let set = f.select(&value, &mask, &Value::int(8, 0));
            let combined = f.binop("or", &cleared, &set);
            f.store(&combined, pointer, Some(1));
        } else {
            let stored = if kind == ScalarKind::Bool {
                cast_raw(f, "zext", &value, IrType::Int(8))
            } else {
                value
            };
            f.store(&stored, pointer, Some(1));
        }
        Ok(())
    }

    fn reject_input_write(address: &DirectAddress, span: Span) -> Res<()> {
        if address.area == Some(DirectArea::Input) {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "writing an `%I` input is not allowed".into(),
                span,
            ));
        }
        Ok(())
    }

    /// Writes a declared `AT` root's initial value during cold preparation.
    #[allow(clippy::too_many_arguments)]
    fn lower_call(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        callee: &CalleeHir,
        args: &[stc_hir::ArgHir],
        supplied: &[Value],
        result_ty: TyId,
        span: Span,
    ) -> Res<Value> {
        match callee {
            CalleeHir::Conversion { from, to } => {
                if supplied.len() != 1 {
                    return Err(CodegenError::internal("conversion arity mismatch"));
                }
                let from_facts = self.types.facts(*from)?.clone();
                let to_facts = self.types.facts(*to)?.clone();
                match (from_facts.scalar(), to_facts.scalar()) {
                    (Some(from_kind), Some(to_kind)) => {
                        self.cast_value(f, supplied[0].clone(), from_kind, to_kind, span)
                    }
                    _ => self.lower_aggregate_cast(f, supplied[0].clone(), *from, *to, span),
                }
            }
            CalleeHir::StdFunction(id) => {
                self.lower_std_function(f, info, *id, args, supplied, result_ty, span)
            }
            CalleeHir::Function(def) => {
                let value = self
                    .lower_function_call(f, info, *def, args, supplied, result_ty, span, false)?;
                value.ok_or_else(|| {
                    CodegenError::unsupported(
                        codes::CODEGEN_UNSUPPORTED,
                        "a function without a result cannot be used as an expression".into(),
                        span,
                    )
                })
            }
            other => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "callee `{}` is not supported in M4",
                    callee_name(self.hir, other)
                ),
                span,
            )),
        }
    }

    /// Lowers a call used as a statement, discarding any result.
    pub(crate) fn lower_call_statement(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: ExprId,
    ) -> Res<()> {
        let expr = self.hir.expr(id).clone();
        let ExprKindHir::Call { callee, args } = expr.kind else {
            return Err(CodegenError::internal("call statement is not a call"));
        };
        match &callee {
            CalleeHir::Function(def) => {
                let mut supplied = Vec::new();
                for arg in &args {
                    if arg.direction == stc_hir::ArgDirection::Input
                        && let Some(value) = arg.value
                    {
                        supplied.push(self.lower_expr(f, info, value)?);
                    }
                }
                let _ = self.lower_function_call(
                    f, info, *def, &args, &supplied, expr.ty, expr.span, true,
                )?;
                Ok(())
            }
            CalleeHir::Conversion { .. } | CalleeHir::StdFunction(_) => {
                let _ = self.lower_expr(f, info, id)?;
                Ok(())
            }
            other => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "callee `{}` is not supported in M4",
                    callee_name(self.hir, other)
                ),
                expr.span,
            )),
        }
    }

    // ---- operators --------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn lower_binary(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        lhs_value: Value,
        rhs_value: Value,
        span: Span,
    ) -> Res<Value> {
        let lhs_ty = self.hir.expr(lhs).ty;
        let rhs_ty = self.hir.expr(rhs).ty;
        let lhs_facts = self.types.facts(lhs_ty)?.clone();
        let rhs_facts = self.types.facts(rhs_ty)?.clone();
        let reference_like = |facts: &crate::types::TypeFacts| {
            facts.is_reference() || matches!(facts.category, crate::types::TypeCategory::Null)
        };
        if reference_like(&lhs_facts) || reference_like(&rhs_facts) {
            return match op {
                BinOp::Eq => {
                    self.lower_reference_comparison(f, lhs_ty, &lhs_value, rhs_ty, &rhs_value, true)
                }
                BinOp::Ne => self
                    .lower_reference_comparison(f, lhs_ty, &lhs_value, rhs_ty, &rhs_value, false),
                other => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!(
                        "operator `{}` is not supported for references",
                        other.as_str()
                    ),
                    span,
                )),
            };
        }
        if lhs_facts.is_string() || rhs_facts.is_string() {
            if lhs_facts.key != rhs_facts.key || !lhs_facts.is_string() || !rhs_facts.is_string() {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "string comparison requires equal string types".into(),
                    span,
                ));
            }
            return match op {
                BinOp::Eq => self.lower_string_comparison(f, &lhs_value, &rhs_value, lhs_ty, true),
                BinOp::Ne => self.lower_string_comparison(f, &lhs_value, &rhs_value, lhs_ty, false),
                other => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("operator `{}` is not supported for strings", other.as_str()),
                    span,
                )),
            };
        }
        if !lhs_facts.is_scalar_shaped() || !rhs_facts.is_scalar_shaped() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "operator `{}` is not supported for aggregate values",
                    op.as_str()
                ),
                span,
            ));
        }
        let lhs_kind = lhs_facts.scalar().expect("scalar-shaped");
        let rhs_kind = rhs_facts.scalar().expect("scalar-shaped");
        if op.is_comparison() {
            return self.lower_comparison(f, op, lhs_kind, &lhs_value, &rhs_value, span);
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Power => {
                if lhs_kind == ScalarKind::Time || rhs_kind == ScalarKind::Time {
                    return self.lower_time_arithmetic(
                        f, op, lhs_kind, rhs_kind, &lhs_value, &rhs_value, span,
                    );
                }
                self.lower_arithmetic(f, info, op, lhs_kind, &lhs_value, &rhs_value, span)
            }
            BinOp::And => self.bit_op(f, "and", lhs_kind, &lhs_value, &rhs_value, span),
            BinOp::Or => self.bit_op(f, "or", lhs_kind, &lhs_value, &rhs_value, span),
            BinOp::Xor => self.bit_op(f, "xor", lhs_kind, &lhs_value, &rhs_value, span),
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!("operator `{}` is not supported in M4", op.as_str()),
                span,
            )),
        }
    }

    pub(crate) fn lower_comparison(
        &mut self,
        f: &mut Function,
        op: BinOp,
        kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        match kind {
            ScalarKind::Bool => match op {
                BinOp::Eq => Ok(f.icmp("eq", lhs, rhs)),
                BinOp::Ne => Ok(f.icmp("ne", lhs, rhs)),
                other => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("comparison `{}` on BOOL is not supported", other.as_str()),
                    span,
                )),
            },
            ScalarKind::Real { .. } => {
                let predicate = match op {
                    BinOp::Eq => "oeq",
                    BinOp::Ne => "une",
                    BinOp::Lt => "olt",
                    BinOp::Le => "ole",
                    BinOp::Gt => "ogt",
                    BinOp::Ge => "oge",
                    _ => unreachable!(),
                };
                Ok(f.fcmp(predicate, lhs, rhs))
            }
            _ => {
                let predicate = match (op, kind.is_signed()) {
                    (BinOp::Eq, _) => "eq",
                    (BinOp::Ne, _) => "ne",
                    (BinOp::Lt, true) => "slt",
                    (BinOp::Le, true) => "sle",
                    (BinOp::Gt, true) => "sgt",
                    (BinOp::Ge, true) => "sge",
                    (BinOp::Lt, false) => "ult",
                    (BinOp::Le, false) => "ule",
                    (BinOp::Gt, false) => "ugt",
                    (BinOp::Ge, false) => "uge",
                    _ => unreachable!(),
                };
                Ok(f.icmp(predicate, lhs, rhs))
            }
        }
    }

    pub(crate) fn bit_op(
        &mut self,
        f: &mut Function,
        op: &str,
        kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        match kind {
            ScalarKind::Bool | ScalarKind::Int { .. } => Ok(f.binop(op, lhs, rhs)),
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!("bitwise `{op}` is not supported for this type"),
                span,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_arithmetic(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        op: BinOp,
        kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        match kind {
            ScalarKind::Real { width } => match op {
                BinOp::Add => Ok(f.binop("fadd", lhs, rhs)),
                BinOp::Sub => Ok(f.binop("fsub", lhs, rhs)),
                BinOp::Mul => Ok(f.binop("fmul", lhs, rhs)),
                BinOp::Div => Ok(f.binop("fdiv", lhs, rhs)),
                BinOp::Mod => Ok(f.binop("frem", lhs, rhs)),
                BinOp::Power => {
                    let name = if width == 32 {
                        "llvm.pow.f32"
                    } else {
                        "llvm.pow.f64"
                    };
                    let ty = if width == 32 {
                        IrType::Float
                    } else {
                        IrType::Double
                    };
                    self.module.declare(
                        name,
                        &format!("{} @{name}({}, {})", ty.render(), ty.render(), ty.render()),
                    );
                    f.call(ty, name, &[lhs.clone(), rhs.clone()])
                        .ok_or_else(|| CodegenError::internal("pow call returned void"))
                }
                _ => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("operator `{}` is not supported for REAL", op.as_str()),
                    span,
                )),
            },
            ScalarKind::Int { signed, width } => match op {
                BinOp::Add => self.checked_int(f, "add", signed, width, lhs, rhs, span),
                BinOp::Sub => self.checked_int(f, "sub", signed, width, lhs, rhs, span),
                BinOp::Mul => self.checked_int(f, "mul", signed, width, lhs, rhs, span),
                BinOp::Div | BinOp::Mod => {
                    self.int_div(f, op == BinOp::Mod, signed, width, lhs, rhs, span)
                }
                BinOp::Power => {
                    self.ensure_pow_helper(width, signed)?;
                    let name = format!("stc_pow_{}{width}", if signed { 's' } else { 'u' });
                    let span_value = Value::int(32, i128::from(self.span_id(span)));
                    f.call(
                        IrType::Int(width),
                        &name,
                        &[lhs.clone(), rhs.clone(), span_value, info.slots.clone()],
                    )
                    .ok_or_else(|| CodegenError::internal("pow helper returned void"))
                }
                _ => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    format!("operator `{}` is not supported for integers", op.as_str()),
                    span,
                )),
            },
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!("operator `{}` is not supported for this type", op.as_str()),
                span,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_time_arithmetic(
        &mut self,
        f: &mut Function,
        op: BinOp,
        lhs_kind: ScalarKind,
        rhs_kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        match op {
            BinOp::Add | BinOp::Sub
                if lhs_kind == ScalarKind::Time && rhs_kind == ScalarKind::Time =>
            {
                self.checked_int(
                    f,
                    if op == BinOp::Add { "add" } else { "sub" },
                    true,
                    64,
                    lhs,
                    rhs,
                    span,
                )
            }
            BinOp::Mul => {
                let (time_value, factor_kind, factor) = if lhs_kind == ScalarKind::Time {
                    (lhs, rhs_kind, rhs)
                } else {
                    (rhs, lhs_kind, lhs)
                };
                let factor = match factor_kind {
                    ScalarKind::Int { signed, width } => to_i64(f, factor, signed, width),
                    _ => {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "TIME multiplication by a non-integer is not supported in M4".into(),
                            span,
                        ));
                    }
                };
                self.checked_int(f, "mul", true, 64, time_value, &factor, span)
            }
            BinOp::Div if lhs_kind == ScalarKind::Time => {
                let factor = match rhs_kind {
                    ScalarKind::Int { signed, width } => to_i64(f, rhs, signed, width),
                    _ => {
                        return Err(CodegenError::unsupported(
                            codes::CODEGEN_UNSUPPORTED,
                            "TIME division by a non-integer is not supported in M4".into(),
                            span,
                        ));
                    }
                };
                self.int_div(f, false, true, 64, lhs, &factor, span)
            }
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!("operator `{}` is not supported for TIME", op.as_str()),
                span,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn checked_int(
        &mut self,
        f: &mut Function,
        op: &str,
        signed: bool,
        bits: u16,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        let ty = IrType::Int(bits);
        let name = format!(
            "llvm.{}{}.with.overflow.i{bits}",
            if signed { 's' } else { 'u' },
            op
        );
        self.module.declare(
            &name,
            &format!(
                "{{ {}, i1 }} @{name}({}, {})",
                ty.render(),
                ty.render(),
                ty.render()
            ),
        );
        let aggregate_ty = IrType::Overflow(bits);
        let aggregate = f
            .call(aggregate_ty, &name, &[lhs.clone(), rhs.clone()])
            .ok_or_else(|| CodegenError::internal("overflow intrinsic returned void"))?;
        let value = f.extract_value(&aggregate, 0, ty);
        let overflow = f.extract_value(&aggregate, 1, IrType::Int(1));
        self.trap_if(f, &overflow, trap::OVERFLOW, span)?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    fn int_div(
        &mut self,
        f: &mut Function,
        remainder: bool,
        signed: bool,
        bits: u16,
        lhs: &Value,
        rhs: &Value,
        span: Span,
    ) -> Res<Value> {
        let ty = IrType::Int(bits);
        let zero = f.icmp("eq", rhs, &Value::int(bits, 0));
        self.trap_if(f, &zero, trap::DIV_ZERO, span)?;
        if signed {
            let minimum = Value::int(bits, -(1_i128 << (bits - 1)));
            let is_min = f.icmp("eq", lhs, &minimum);
            let is_neg_one = f.icmp("eq", rhs, &Value::int(bits, -1));
            let overflow = f.binop("and", &is_min, &is_neg_one);
            self.trap_if(f, &overflow, trap::OVERFLOW, span)?;
        }
        let op = match (remainder, signed) {
            (false, true) => "sdiv",
            (false, false) => "udiv",
            (true, true) => "srem",
            (true, false) => "urem",
        };
        let value = f.binop(op, lhs, rhs);
        debug_assert_eq!(value.ty, ty);
        Ok(value)
    }

    pub(crate) fn trap_if(
        &mut self,
        f: &mut Function,
        cond: &Value,
        code: i32,
        span: Span,
    ) -> Res<()> {
        let slots = self
            .current_slots
            .clone()
            .ok_or_else(|| CodegenError::internal("trap emitted outside a GOT-aware function"))?;
        let slot = self.import_slot(stc_got::catalog::ImportService::Trap)?;
        let cont = f.next_block_label();
        let trap_block = f.next_block_label();
        f.cond_branch(cond, &trap_block, &cont);
        f.start_block(&trap_block);
        let span_id = self.span_id(span);
        let trap_fn = f.slot_value(&slots, slot);
        f.call_ptr(
            IrType::Void,
            &trap_fn,
            &[
                Value::int(32, i128::from(code)),
                Value::int(32, i128::from(span_id)),
            ],
        );
        f.unreachable();
        f.start_block(&cont);
        Ok(())
    }

    fn lower_unary(
        &mut self,
        f: &mut Function,
        op: UnOp,
        operand: ExprId,
        value: Value,
        span: Span,
    ) -> Res<Value> {
        let kind = self.scalar_at(self.hir.expr(operand).ty, span)?;
        match op {
            UnOp::Plus => Ok(value),
            UnOp::Neg => match kind {
                ScalarKind::Real { .. } => Ok(f.unop("fneg", &value)),
                ScalarKind::Int { signed, width } => {
                    let zero = Value::int(width, 0);
                    self.checked_int(f, "sub", signed, width, &zero, &value, span)
                }
                ScalarKind::Time => {
                    let zero = Value::int(64, 0);
                    self.checked_int(f, "sub", true, 64, &zero, &value, span)
                }
                _ => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "unary `-` is not supported for this type".into(),
                    span,
                )),
            },
            UnOp::Not => match kind {
                ScalarKind::Bool => Ok(f.binop("xor", &value, &Value::bool(true))),
                ScalarKind::Int { width, .. } => Ok(f.binop("xor", &value, &Value::int(width, -1))),
                _ => Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "`NOT` is only supported for BOOL and bit strings in M4".into(),
                    span,
                )),
            },
        }
    }

    // ---- casts ------------------------------------------------------------

    fn cast_value(
        &mut self,
        f: &mut Function,
        value: Value,
        from: ScalarKind,
        to: ScalarKind,
        span: Span,
    ) -> Res<Value> {
        if from == to {
            return Ok(value);
        }
        match (from, to) {
            (ScalarKind::Bool, ScalarKind::Bool) => Ok(value),
            (ScalarKind::Bool, ScalarKind::Int { width, .. } | ScalarKind::Char { width }) => {
                Ok(cast_raw(f, "zext", &value, IrType::Int(width)))
            }
            (ScalarKind::Bool, ScalarKind::Real { width }) => {
                Ok(cast_raw(f, "uitofp", &value, real_type(width)))
            }
            (ScalarKind::Int { .. } | ScalarKind::Char { .. }, ScalarKind::Bool) => {
                Ok(f.icmp("ne", &value, &Value::int(value_bits(from), 0)))
            }
            (
                ScalarKind::Int { .. } | ScalarKind::Char { .. } | ScalarKind::Time,
                ScalarKind::Int { signed, width },
            ) => self.int_convert(f, &value, from, width, signed, span),
            (
                ScalarKind::Int { .. } | ScalarKind::Char { .. } | ScalarKind::Time,
                ScalarKind::Char { width },
            ) => self.int_convert(f, &value, from, width, false, span),
            (ScalarKind::Int { signed, width }, ScalarKind::Real { width: real_width }) => {
                let op = if signed { "sitofp" } else { "uitofp" };
                let _ = width;
                Ok(cast_raw(f, op, &value, real_type(real_width)))
            }
            (ScalarKind::Real { width: from_width }, ScalarKind::Real { width: to_width }) => {
                if from_width == to_width {
                    return Ok(value);
                }
                let op = if from_width < to_width {
                    "fpext"
                } else {
                    "fptrunc"
                };
                Ok(cast_raw(f, op, &value, real_type(to_width)))
            }
            (ScalarKind::Real { .. }, ScalarKind::Int { signed, width }) => {
                self.real_to_int(f, &value, width, signed, span)
            }
            (ScalarKind::Real { .. }, ScalarKind::Char { width }) => {
                self.real_to_int(f, &value, width, false, span)
            }
            (ScalarKind::Date(a), ScalarKind::Date(b)) => self.date_convert(f, &value, a, b, span),
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                format!(
                    "conversion from {} to {} is not supported in M4",
                    from.canonical_name(),
                    to.canonical_name()
                ),
                span,
            )),
        }
    }

    fn int_convert(
        &mut self,
        f: &mut Function,
        value: &Value,
        from: ScalarKind,
        to_width: u16,
        to_signed: bool,
        span: Span,
    ) -> Res<Value> {
        let from_width = value_bits(from);
        if from_width < to_width {
            let op = if from.is_signed() { "sext" } else { "zext" };
            return Ok(cast_raw(f, op, value, IrType::Int(to_width)));
        }
        if from_width > to_width {
            let fits = range_check(f, value, from, to_width, to_signed);
            let outside = f.binop("xor", &fits, &Value::bool(true));
            self.trap_if(f, &outside, trap::RANGE, span)?;
            return Ok(cast_raw(f, "trunc", value, IrType::Int(to_width)));
        }
        Ok(value.clone())
    }

    fn real_to_int(
        &mut self,
        f: &mut Function,
        value: &Value,
        to_width: u16,
        to_signed: bool,
        span: Span,
    ) -> Res<Value> {
        let ordered = f.fcmp("ord", value, value);
        let unordered = f.binop("xor", &ordered, &Value::bool(true));
        self.trap_if(f, &unordered, trap::CONVERSION, span)?;
        let fits = float_range_check(f, value, to_width, to_signed);
        let outside = f.binop("xor", &fits, &Value::bool(true));
        self.trap_if(f, &outside, trap::RANGE, span)?;
        let op = if to_signed { "fptosi" } else { "fptoui" };
        Ok(cast_raw(f, op, value, IrType::Int(to_width)))
    }

    fn date_convert(
        &mut self,
        f: &mut Function,
        value: &Value,
        from: stc_hir::DateTy,
        to: stc_hir::DateTy,
        span: Span,
    ) -> Res<Value> {
        let from_days = matches!(from, stc_hir::DateTy::Date);
        let to_days = matches!(to, stc_hir::DateTy::Date);
        match (from_days, to_days) {
            (false, true) => {
                let divisor = Value::int(64, 86_400_000_000_000);
                Ok(f.binop("sdiv", value, &divisor))
            }
            (true, false) => {
                let factor = Value::int(64, 86_400_000_000_000);
                self.checked_int(f, "mul", true, 64, value, &factor, span)
            }
            _ => Ok(value.clone()),
        }
    }

    // ---- integer power ----------------------------------------------------

    fn ensure_pow_helper(&mut self, bits: u16, signed: bool) -> Res<()> {
        let name = format!("stc_pow_{}{bits}", if signed { 's' } else { 'u' });
        if self.module.functions.contains_key(&name) {
            return Ok(());
        }
        let ty = IrType::Int(bits);
        let mut helper = Function::new(
            name.clone(),
            SymbolAttrs::internal(),
            ty.clone(),
            vec![
                (ty.clone(), "%a0".into()),
                (ty.clone(), "%a1".into()),
                (IrType::Int(32), "%a2".into()),
                (IrType::Ptr, "%slots".into()),
            ],
        );
        helper.start_fresh_block();
        let base_ptr = helper.alloca(ty.clone());
        let exponent_ptr = helper.alloca(ty.clone());
        let result_ptr = helper.alloca(ty.clone());
        let base = helper.param_value(0);
        let exponent = helper.param_value(1);
        let span_value = helper.param_value(2);
        let slots_value = helper.param_value(3);
        let trap_slot = self.import_slot(stc_got::catalog::ImportService::Trap)?;
        helper.store(&base, &base_ptr, None);
        helper.store(&exponent, &exponent_ptr, None);
        helper.store(&Value::int(bits, 1), &result_ptr, None);

        if signed {
            let exponent_value = helper.load(ty.clone(), &exponent_ptr, None);
            let negative = helper.icmp("slt", &exponent_value, &Value::int(bits, 0));
            emit_trap_block(
                &mut helper,
                &negative,
                &span_value,
                &slots_value,
                trap_slot,
                trap::RANGE,
            );
        }

        let loop_label = helper.next_block_label();
        let body_label = helper.next_block_label();
        let exit_label = helper.next_block_label();
        helper.branch(&loop_label);
        helper.start_block(&loop_label);
        let exponent_value = helper.load(ty.clone(), &exponent_ptr, None);
        let done = if signed {
            helper.icmp("sle", &exponent_value, &Value::int(bits, 0))
        } else {
            helper.icmp("eq", &exponent_value, &Value::int(bits, 0))
        };
        helper.cond_branch(&done, &exit_label, &body_label);
        helper.start_block(&body_label);

        let exponent_value = helper.load(ty.clone(), &exponent_ptr, None);
        let odd_bits = helper.binop("and", &exponent_value, &Value::int(bits, 1));
        let is_odd = helper.icmp("ne", &odd_bits, &Value::int(bits, 0));
        let multiply_label = helper.next_block_label();
        let after_multiply = helper.next_block_label();
        helper.cond_branch(&is_odd, &multiply_label, &after_multiply);

        helper.start_block(&multiply_label);
        let result = helper.load(ty.clone(), &result_ptr, None);
        let base_value = helper.load(ty.clone(), &base_ptr, None);
        let product = checked_mul_in_helper(
            &mut helper,
            &mut self.module,
            bits,
            signed,
            &result,
            &base_value,
            &span_value,
            &slots_value,
            trap_slot,
        );
        helper.store(&product, &result_ptr, None);
        helper.branch(&after_multiply);

        helper.start_block(&after_multiply);
        let exponent_value = helper.load(ty.clone(), &exponent_ptr, None);
        let shifted = helper.binop(
            if signed { "ashr" } else { "lshr" },
            &exponent_value,
            &Value::int(bits, 1),
        );
        helper.store(&shifted, &exponent_ptr, None);
        let still_going = helper.icmp("ne", &shifted, &Value::int(bits, 0));
        let square_label = helper.next_block_label();
        helper.cond_branch(&still_going, &square_label, &loop_label);

        helper.start_block(&square_label);
        let base_value = helper.load(ty.clone(), &base_ptr, None);
        let squared = checked_mul_in_helper(
            &mut helper,
            &mut self.module,
            bits,
            signed,
            &base_value,
            &base_value,
            &span_value,
            &slots_value,
            trap_slot,
        );
        helper.store(&squared, &base_ptr, None);
        helper.branch(&loop_label);

        helper.start_block(&exit_label);
        let result = helper.load(ty.clone(), &result_ptr, None);
        helper.ret(Some(&result));
        helper.check().map_err(CodegenError::internal)?;
        self.module.add_function(helper);
        Ok(())
    }
}

fn emit_trap_block(
    f: &mut Function,
    condition: &Value,
    span_value: &Value,
    slots: &Value,
    trap_slot: u32,
    code: i32,
) {
    let continuation = f.next_block_label();
    let trap_block = f.next_block_label();
    f.cond_branch(condition, &trap_block, &continuation);
    f.start_block(&trap_block);
    let trap_fn = f.slot_value(slots, trap_slot);
    f.call_ptr(
        IrType::Void,
        &trap_fn,
        &[Value::int(32, i128::from(code)), span_value.clone()],
    );
    f.unreachable();
    f.start_block(&continuation);
}

#[allow(clippy::too_many_arguments)]
fn checked_mul_in_helper(
    f: &mut Function,
    module: &mut Module,
    bits: u16,
    signed: bool,
    lhs: &Value,
    rhs: &Value,
    span_value: &Value,
    slots: &Value,
    trap_slot: u32,
) -> Value {
    let ty = IrType::Int(bits);
    let name = format!(
        "llvm.{}{}.with.overflow.i{bits}",
        if signed { 's' } else { 'u' },
        "mul"
    );
    module.declare(
        &name,
        &format!(
            "{{ {}, i1 }} @{name}({}, {})",
            ty.render(),
            ty.render(),
            ty.render()
        ),
    );
    let aggregate_ty = IrType::Overflow(bits);
    let aggregate = f
        .call(aggregate_ty, &name, &[lhs.clone(), rhs.clone()])
        .expect("overflow intrinsic returns a value");
    let value = f.extract_value(&aggregate, 0, ty);
    let overflow = f.extract_value(&aggregate, 1, IrType::Int(1));
    emit_trap_block(f, &overflow, span_value, slots, trap_slot, trap::OVERFLOW);
    value
}

fn callee_name(hir: &HirCrate, callee: &CalleeHir) -> String {
    match callee {
        CalleeHir::Function(def) => hir.def_name(*def).to_string(),
        CalleeHir::StdFunction(id) => hir.std_fns[id.index()].kind.as_str().into(),
        CalleeHir::Conversion { from, to } => {
            format!("{} -> {}", hir.type_name(*from), hir.type_name(*to))
        }
        CalleeHir::Method { def, .. } | CalleeHir::SuperMethod { def, .. } => {
            hir.def_name(*def).to_string()
        }
        CalleeHir::InterfaceMethod { method, .. } => {
            format!("{} (interface)", hir.def_name(*method))
        }
        CalleeHir::Fb { def, .. } => format!("{} (function block)", hir.def_name(*def)),
        CalleeHir::SuperBody { base, .. } => format!("SUPER({})", hir.def_name(*base)),
    }
}

pub(crate) fn value_bits(kind: ScalarKind) -> u16 {
    match kind {
        ScalarKind::Bool => 1,
        ScalarKind::Int { width, .. } | ScalarKind::Char { width } => width,
        ScalarKind::Real { width } => width,
        ScalarKind::Time | ScalarKind::Date(_) => 64,
    }
}

fn real_type(width: u16) -> IrType {
    if width == 32 {
        IrType::Float
    } else {
        IrType::Double
    }
}

fn real_constant(value: f64, ty: &IrType) -> Value {
    if *ty == IrType::Float {
        Value::float32(value as f32)
    } else {
        Value::double(value)
    }
}

fn cast_raw(f: &mut Function, op: &str, value: &Value, to: IrType) -> Value {
    let rendered = to.render();
    let cast = f.temp(to);
    f.emit_value(&cast, format!("{op} {} to {rendered}", value.render()));
    cast
}

pub(crate) fn to_storage(f: &mut Function, value: &Value, kind: ScalarKind) -> Value {
    if kind == ScalarKind::Bool {
        cast_raw(f, "zext", value, IrType::Int(8))
    } else {
        value.clone()
    }
}

pub(crate) fn storage_to_value(f: &mut Function, value: &Value, kind: ScalarKind) -> Value {
    if kind == ScalarKind::Bool {
        f.icmp("ne", value, &Value::int(8, 0))
    } else {
        value.clone()
    }
}

fn to_i64(f: &mut Function, value: &Value, signed: bool, width: u16) -> Value {
    if width >= 64 {
        return value.clone();
    }
    cast_raw(
        f,
        if signed { "sext" } else { "zext" },
        value,
        IrType::Int(64),
    )
}

/// Returns an `i1` that is true when `value` fits the destination integer.
fn range_check(
    f: &mut Function,
    value: &Value,
    from: ScalarKind,
    to_width: u16,
    to_signed: bool,
) -> Value {
    let from_width = value_bits(from);
    let source_signed = from.is_signed();
    if to_signed {
        let minimum = -(1_i128 << (to_width - 1));
        let maximum = (1_i128 << (to_width - 1)) - 1;
        if source_signed {
            let low = f.icmp("sge", value, &Value::int(from_width, minimum));
            let high = f.icmp("sle", value, &Value::int(from_width, maximum));
            f.binop("and", &low, &high)
        } else {
            f.icmp("ule", value, &Value::int(from_width, maximum))
        }
    } else {
        let maximum = (1_i128 << to_width) - 1;
        if source_signed {
            let low = f.icmp("sge", value, &Value::int(from_width, 0));
            let high = f.icmp("sle", value, &Value::int(from_width, maximum));
            f.binop("and", &low, &high)
        } else {
            f.icmp("ule", value, &Value::int(from_width, maximum))
        }
    }
}

fn float_range_check(f: &mut Function, value: &Value, to_width: u16, to_signed: bool) -> Value {
    let ty = value.ty.clone();
    let exponent = if to_signed {
        to_width.saturating_sub(1)
    } else {
        to_width
    };
    let high = real_constant((2.0_f64).powi(i32::from(exponent)), &ty);
    let high_ok = f.fcmp("olt", value, &high);
    if to_signed {
        let low = real_constant(-(2.0_f64).powi(i32::from(exponent)), &ty);
        let low_ok = f.fcmp("oge", value, &low);
        f.binop("and", &low_ok, &high_ok)
    } else {
        let zero = real_constant(0.0, &ty);
        let low_ok = f.fcmp("oge", value, &zero);
        f.binop("and", &low_ok, &high_ok)
    }
}
