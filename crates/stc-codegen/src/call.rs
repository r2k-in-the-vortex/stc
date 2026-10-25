//! Phase-3 function calls: aggregate, output, and VAR_IN_OUT parameters.

use std::collections::BTreeMap;

use stc_hir::{ArgDirection, ArgHir, DefId, DefKind, ExprId, ParamId, TyId};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;
use crate::expr::{BodyInfo, Res, storage_to_value, to_storage};
use crate::ir::{Function, IrType, Value};
use crate::module::FunctionTarget;
use crate::types::TypeCategory;
use crate::unit::Lowerer;

struct DeferredCopy {
    target: ExprId,
    temp: Value,
    ty: TyId,
}

impl Lowerer<'_> {
    /// Lowers a function call using the aggregate/output ABI (D-130).
    ///
    /// `supplied` contains the evaluated input argument values in source order.
    /// Returns `None` for a call without a result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_function_call(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        def: DefId,
        args: &[ArgHir],
        supplied: &[Value],
        _result_ty: TyId,
        span: Span,
        discard: bool,
    ) -> Res<Option<Value>> {
        let DefKind::Function { params, result, .. } = self.hir.defs[def.index()].kind.clone()
        else {
            return Err(CodegenError::internal("callee is not a function"));
        };
        self.lower_callable(
            f,
            info,
            &params,
            result,
            args,
            supplied,
            Vec::new(),
            FunctionTarget::Function(def),
            span,
            discard,
        )
    }

    /// Lowers a method call with the receiver object table first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_method_call(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        receiver: ExprId,
        def: DefId,
        args: &[ArgHir],
        supplied: &[Value],
        _result_ty: TyId,
        span: Span,
        discard: bool,
    ) -> Res<Option<Value>> {
        let DefKind::Method {
            owner,
            params,
            result,
            ..
        } = self.hir.defs[def.index()].kind.clone()
        else {
            return Err(CodegenError::internal("callee is not a method"));
        };
        let table = self.object_table(f, info, receiver, owner, span)?;
        self.lower_callable(
            f,
            info,
            &params,
            result,
            args,
            supplied,
            vec![table],
            FunctionTarget::Method(def),
            span,
            discard,
        )
    }

    /// Shared callable lowering for functions and methods.
    #[allow(clippy::too_many_arguments)]
    fn lower_callable(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        params: &[ParamId],
        result: Option<TyId>,
        args: &[ArgHir],
        supplied: &[Value],
        mut call_args: Vec<Value>,
        target: FunctionTarget,
        span: Span,
        discard: bool,
    ) -> Res<Option<Value>> {
        let result_facts = result.map(|ty| self.types.facts(ty).cloned()).transpose()?;
        let aggregate_result = result_facts
            .as_ref()
            .is_some_and(crate::types::TypeFacts::is_contiguous_aggregate);

        let mut input_values: BTreeMap<ParamId, Value> = BTreeMap::new();
        let mut output_args: BTreeMap<ParamId, &ArgHir> = BTreeMap::new();
        let mut supplied_iter = supplied.iter();
        for arg in args {
            let Some(formal) = arg.formal else {
                return Err(CodegenError::internal(
                    "call argument has no resolved formal parameter",
                ));
            };
            match arg.direction {
                ArgDirection::Input => {
                    if arg.value.is_some() {
                        let value = supplied_iter.next().cloned().ok_or_else(|| {
                            CodegenError::internal("missing supplied argument value")
                        })?;
                        input_values.insert(formal, value);
                    }
                }
                ArgDirection::Output | ArgDirection::InOut => {
                    output_args.insert(formal, arg);
                }
            }
        }

        let mut deferred: Vec<DeferredCopy> = Vec::new();
        let mut negated_targets: Vec<ExprId> = Vec::new();
        for param in params {
            let data = self.hir.param(*param).clone();
            let facts = self.types.facts(data.ty)?.clone();
            match data.direction {
                ArgDirection::Input => {
                    let value = if let Some(value) = input_values.get(param) {
                        value.clone()
                    } else if let Some(default) = data.default {
                        self.lower_expr(f, info, default)?
                    } else {
                        return Err(CodegenError::internal(format!(
                            "missing required argument `{}`",
                            self.hir.interner.text(data.name)
                        )));
                    };
                    if matches!(
                        facts.category,
                        TypeCategory::Array {
                            conformant: true,
                            ..
                        }
                    ) {
                        let actual = args
                            .iter()
                            .find(|arg| arg.formal == Some(*param))
                            .and_then(|arg| arg.value)
                            .ok_or_else(|| {
                                CodegenError::internal("conformant argument expression missing")
                            })?;
                        let descriptor = self.conformant_descriptor(f, info, actual, span)?;
                        let temp = f.alloca(facts.llvm_storage.clone());
                        f.store(&descriptor, &temp, None);
                        call_args.push(temp);
                    } else if facts.is_contiguous_aggregate() {
                        let temp = f.alloca(facts.llvm_storage.clone());
                        f.store(&value, &temp, None);
                        call_args.push(temp);
                    } else if facts.is_object() || facts.is_pointer() {
                        call_args.push(value);
                    } else {
                        let kind = facts.scalar().expect("non-aggregate is scalar");
                        call_args.push(to_storage(f, &value, kind));
                    }
                }
                ArgDirection::Output | ArgDirection::InOut => {
                    let Some(arg) = output_args.get(param).copied() else {
                        if data.direction == ArgDirection::InOut {
                            return Err(CodegenError::unsupported(
                                codes::CODEGEN_UNSUPPORTED,
                                format!(
                                    "missing VAR_IN_OUT argument for `{}`",
                                    self.hir.interner.text(data.name)
                                ),
                                span,
                            ));
                        }
                        let temp = f.alloca(facts.llvm_storage.clone());
                        f.store(&Value::zero(facts.llvm_storage.clone()), &temp, None);
                        call_args.push(temp);
                        continue;
                    };
                    if matches!(
                        facts.category,
                        TypeCategory::Array {
                            conformant: true,
                            ..
                        }
                    ) {
                        if data.direction != ArgDirection::InOut {
                            return Err(CodegenError::unsupported(
                                codes::CODEGEN_UNSUPPORTED,
                                "conformant output parameters are not supported in M5".into(),
                                span,
                            ));
                        }
                        let actual = arg.value.ok_or_else(|| {
                            CodegenError::internal("conformant VAR_IN_OUT target missing")
                        })?;
                        let descriptor = self.conformant_descriptor(f, info, actual, span)?;
                        let temp = f.alloca(facts.llvm_storage.clone());
                        f.store(&descriptor, &temp, None);
                        call_args.push(temp);
                        continue;
                    }
                    let place = if data.direction == ArgDirection::InOut {
                        arg.value
                    } else {
                        arg.target
                    };
                    let (pointer, copy) = self.prepare_indirect_argument(
                        f,
                        info,
                        place,
                        data.ty,
                        data.direction == ArgDirection::InOut,
                        span,
                    )?;
                    call_args.push(pointer);
                    if let Some(copy) = copy {
                        deferred.push(copy);
                    }
                    if arg.negated
                        && let Some(target) = place
                    {
                        negated_targets.push(target);
                    }
                }
            }
        }

        let mut result_temp = None;
        if aggregate_result {
            let ty = result.expect("aggregate result has a type");
            let storage = self.types.llvm_storage(ty)?;
            let temp = f.alloca(storage.clone());
            f.store(&Value::zero(storage.clone()), &temp, None);
            call_args.push(temp.clone());
            result_temp = Some((temp, storage));
        }
        call_args.push(info.slots.clone());

        let slot = self.function_slot(target)?;
        let callee = Self::slot_storage(f, info, slot);
        let ret_storage = if aggregate_result {
            IrType::Void
        } else {
            match &result_facts {
                Some(facts) if facts.is_object() || facts.is_pointer() => IrType::Ptr,
                Some(facts) => facts.llvm_storage.clone(),
                None => IrType::Void,
            }
        };
        let returned = f.call_ptr(ret_storage, &callee, &call_args);

        for copy in deferred {
            let facts = self.types.facts(copy.ty)?.clone();
            let storage = facts.llvm_storage.clone();
            let raw = f.load(storage, &copy.temp, None);
            if let Some(kind) = facts.scalar() {
                let value = storage_to_value(f, &raw, kind);
                self.check_subrange(f, copy.ty, &value, span)?;
                self.write_place(f, info, copy.target, value)?;
            } else {
                let path = self.place_path(copy.target, span)?;
                self.write_aggregate_place(f, info, &path, raw)?;
            }
        }
        for target in negated_targets {
            self.apply_output_negation(f, info, target, span)?;
        }

        if discard {
            return Ok(None);
        }
        match &result_facts {
            None => Ok(None),
            Some(_) if aggregate_result => {
                let (temp, storage) = result_temp
                    .ok_or_else(|| CodegenError::internal("aggregate result temp missing"))?;
                Ok(Some(f.load(storage, &temp, None)))
            }
            Some(facts) if facts.is_object() || facts.is_pointer() => Ok(returned),
            Some(facts) => {
                let value = returned
                    .ok_or_else(|| CodegenError::internal("scalar function call returned void"))?;
                let kind = facts.scalar().expect("non-aggregate result is scalar");
                Ok(Some(storage_to_value(f, &value, kind)))
            }
        }
    }

    fn place_path(&self, target: ExprId, span: Span) -> Res<crate::place::PlacePath> {
        self.collect_place(target).ok_or_else(|| {
            CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "an aggregate output target must be a place".into(),
                span,
            )
        })
    }

    /// Prepares the pointer for one output or VAR_IN_OUT argument.
    fn prepare_indirect_argument(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        target: Option<ExprId>,
        ty: TyId,
        initialize: bool,
        span: Span,
    ) -> Res<(Value, Option<DeferredCopy>)> {
        let facts = self.types.facts(ty)?.clone();
        let storage = facts.llvm_storage.clone();
        let Some(target) = target else {
            let temp = f.alloca(storage.clone());
            f.store(&Value::zero(storage), &temp, None);
            return Ok((temp, None));
        };
        let Some(path) = self.collect_place(target) else {
            if let Some(pointer) = self.pointer_place_address(f, info, target)? {
                return Ok((pointer, None));
            }
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "an output or `VAR_IN_OUT` target must be a place".into(),
                span,
            ));
        };
        if let Some(resolved) = self.resolve_place_pointer(f, info, &path)? {
            if resolved.direct.is_none() {
                return Ok((resolved.pointer, None));
            }
            let temp = f.alloca(storage.clone());
            if initialize {
                let value = self.read_place(f, info, target)?;
                self.store_value_as_storage(f, &temp, ty, value)?;
            } else {
                f.store(&Value::zero(storage), &temp, None);
            }
            return Ok((temp.clone(), Some(DeferredCopy { target, temp, ty })));
        }
        if facts.is_object() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "object output arguments are M5 phase 4".into(),
                span,
            ));
        }
        let temp = f.alloca(storage.clone());
        f.store(&Value::zero(storage.clone()), &temp, None);
        if initialize {
            let value = self.read_aggregate_place(f, info, &path)?;
            f.store(&value, &temp, None);
        }
        Ok((temp.clone(), Some(DeferredCopy { target, temp, ty })))
    }

    pub(crate) fn apply_output_negation(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        target: ExprId,
        span: Span,
    ) -> Res<()> {
        let value = self.read_place(f, info, target)?;
        let ty = self.hir.expr(target).ty;
        let facts = self.types.facts(ty)?.clone();
        let Some(kind) = facts.scalar() else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`NOT =>` requires a scalar output".into(),
                span,
            ));
        };
        let negated = if kind == crate::layout::ScalarKind::Bool {
            f.binop("xor", &value, &Value::bool(true))
        } else if matches!(
            kind,
            crate::layout::ScalarKind::Int { .. } | crate::layout::ScalarKind::Char { .. }
        ) {
            let bits = crate::expr::value_bits(kind);
            f.binop("xor", &value, &Value::int(bits, -1))
        } else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`NOT =>` requires an integer or BOOL output".into(),
                span,
            ));
        };
        self.write_place(f, info, target, negated)
    }
}

impl Lowerer<'_> {
    /// Builds a conformant-array descriptor for an actual argument.
    pub(crate) fn conformant_descriptor(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        actual: ExprId,
        span: Span,
    ) -> Res<Value> {
        let actual_ty = self.hir.expr(actual).ty;
        let facts = self.types.facts(actual_ty)?.clone();
        let TypeCategory::Array {
            element: _,
            dims,
            conformant,
        } = facts.category
        else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "a conformant array requires an array argument".into(),
                span,
            ));
        };
        if conformant {
            return self.lower_expr(f, info, actual);
        }
        if dims.is_empty() {
            return Err(CodegenError::internal(
                "a conformant array has no dimensions",
            ));
        }
        if !self.types.is_single_root(actual_ty)? {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "conformant arrays of aggregate elements are not supported in M5".into(),
                span,
            ));
        }
        let Some(path) = self.collect_place(actual) else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "a conformant array argument must be a place".into(),
                span,
            ));
        };
        let Some(resolved) = self.resolve_place_pointer(f, info, &path)? else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "a conformant array argument has no contiguous storage".into(),
                span,
            ));
        };
        if resolved.direct.is_some() {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "conformant arrays cannot bind direct variables".into(),
                span,
            ));
        }
        let rank = dims.len();
        let desc_ty = IrType::aggregate(format!("{{ ptr, i64, [{rank} x {{ i64, i64 }}] }}"));
        let total = dims
            .iter()
            .fold(1_u64, |total, dim| total.saturating_mul(dim.length));
        let mut value = Value::zero(desc_ty.clone());
        value = f.insert_value_at(&value, &resolved.pointer, &[0], desc_ty.clone());
        value = f.insert_value_at(
            &value,
            &Value::int(64, i128::from(total)),
            &[1],
            desc_ty.clone(),
        );
        for (index, dimension) in dims.iter().enumerate() {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            value = f.insert_value_at(
                &value,
                &Value::int(64, dimension.lower),
                &[2, index, 0],
                desc_ty.clone(),
            );
            value = f.insert_value_at(
                &value,
                &Value::int(64, dimension.upper),
                &[2, index, 1],
                desc_ty.clone(),
            );
        }
        Ok(value)
    }
}
