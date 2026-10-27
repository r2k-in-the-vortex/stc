//! Statement and control-flow lowering (Architecture §4.7).

use stc_hir::{BinOp, ConstValue, ExprId, StmtId, StmtKindHir};
use stc_span::Span;

use crate::CodegenError;
use crate::codes;
use crate::expr::{BodyInfo, Res};
use crate::ir::{Function, IrType, Value};
use crate::layout::ScalarKind;
use crate::unit::Lowerer;

#[derive(Clone, Debug)]
pub(crate) struct LoopTarget {
    break_label: String,
    continue_label: String,
}

impl Lowerer<'_> {
    pub(crate) fn lower_stmts(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        stmts: &[StmtId],
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        self.lower_stmts_inner(f, info, stmts, loops, false)
    }

    /// Lowers a control-flow block body. Its first executable statement is a
    /// checkpoint-eligible block entry (Debug and Monitoring §16.3).
    pub(crate) fn lower_block_stmts(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        stmts: &[StmtId],
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        self.lower_stmts_inner(f, info, stmts, loops, true)
    }

    fn lower_stmts_inner(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        stmts: &[StmtId],
        loops: &mut Vec<LoopTarget>,
        block_entry: bool,
    ) -> Res<()> {
        let first_executable = stmts
            .iter()
            .position(|stmt| !matches!(self.hir.stmt(*stmt).kind, StmtKindHir::Empty));
        for (index, stmt) in stmts.iter().enumerate() {
            f.ensure_open();
            self.lower_stmt(
                f,
                info,
                *stmt,
                loops,
                block_entry && Some(index) == first_executable,
            )?;
        }
        Ok(())
    }

    fn lower_stmt(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        id: StmtId,
        loops: &mut Vec<LoopTarget>,
        block_entry: bool,
    ) -> Res<()> {
        let stmt = self.hir.stmt(id).clone();
        self.set_debug_location(f, stmt.span);
        if !matches!(stmt.kind, StmtKindHir::Empty) {
            let mut flags = crate::unit::checkpoint_flags::STATEMENT;
            if block_entry {
                flags |= crate::unit::checkpoint_flags::BLOCK;
            }
            self.record_checkpoint(f, stmt.span, flags);
        }
        match stmt.kind {
            StmtKindHir::Assign { place, value } => {
                let value = self.lower_expr(f, info, value)?;
                self.write_place(f, info, place, value)
            }
            StmtKindHir::Call { call } => self.lower_call_statement(f, info, call),
            StmtKindHir::Return => {
                f.branch(&info.return_label);
                Ok(())
            }
            StmtKindHir::If { arms, else_body } => self.lower_if(f, info, &arms, &else_body, loops),
            StmtKindHir::Case {
                selector,
                arms,
                else_body,
            } => self.lower_case(f, info, selector, &arms, &else_body, stmt.span, loops),
            StmtKindHir::For {
                var,
                start,
                end,
                step,
                body,
            } => self.lower_for(f, info, var, start, end, step, &body, stmt.span, loops),
            StmtKindHir::While { cond, body } => {
                self.lower_while(f, info, cond, &body, stmt.span, loops)
            }
            StmtKindHir::Repeat { body, cond } => {
                self.lower_repeat(f, info, &body, cond, stmt.span, loops)
            }
            StmtKindHir::Exit => {
                let Some(target) = loops.last() else {
                    return Err(CodegenError::internal("`EXIT` outside a loop"));
                };
                f.branch(&target.break_label.clone());
                Ok(())
            }
            StmtKindHir::Continue => {
                let Some(target) = loops.last() else {
                    return Err(CodegenError::internal("`CONTINUE` outside a loop"));
                };
                f.branch(&target.continue_label.clone());
                Ok(())
            }
            StmtKindHir::Empty => Ok(()),
            StmtKindHir::RefAssign { place, value } => {
                let value = self.lower_expr(f, info, value)?;
                self.write_place(f, info, place, value)
            }
            StmtKindHir::Attempt { place, value } => {
                self.lower_attempt_stmt(f, info, place, value, stmt.span)
            }
        }
    }

    fn lower_if(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        arms: &[(ExprId, Vec<StmtId>)],
        else_body: &[StmtId],
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        let end = f.next_block_label();
        if arms.is_empty() {
            self.lower_block_stmts(f, info, else_body, loops)?;
        } else {
            for (condition, body) in arms {
                let body_label = f.next_block_label();
                let next_label = f.next_block_label();
                let condition = self.lower_condition(f, info, *condition)?;
                f.cond_branch(&condition, &body_label, &next_label);
                f.start_block(&body_label);
                self.lower_block_stmts(f, info, body, loops)?;
                if !f.is_terminated() {
                    f.branch(&end);
                }
                f.start_block(&next_label);
            }
            self.lower_block_stmts(f, info, else_body, loops)?;
        }
        if !f.is_terminated() {
            f.branch(&end);
        }
        f.start_block(&end);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_case(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        selector: ExprId,
        arms: &[stc_hir::CaseArmHir],
        else_body: &[StmtId],
        span: Span,
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        let kind = self.scalar_at(self.hir.expr(selector).ty, span)?;
        if matches!(kind, ScalarKind::Real { .. }) {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "real-valued CASE selectors are M5".into(),
                span,
            ));
        }
        let selector = self.lower_expr(f, info, selector)?;
        let end = f.next_block_label();
        for arm in arms {
            if arm.labels.is_empty() {
                continue;
            }
            let body_label = f.next_block_label();
            let next_label = f.next_block_label();
            let mut condition: Option<Value> = None;
            for label in &arm.labels {
                let lower = self.case_value(kind, &label.lower, label.span)?;
                let mut check = self.compare_eq(f, kind, &selector, &lower);
                if let Some(upper) = &label.upper {
                    let upper = self.case_value(kind, upper, label.span)?;
                    let lower_ok = self.compare_ordered(f, BinOp::Ge, kind, &selector, &lower);
                    let upper_ok = self.compare_ordered(f, BinOp::Le, kind, &selector, &upper);
                    let range = f.binop("and", &lower_ok, &upper_ok);
                    check = f.binop("or", &check, &range);
                }
                condition = Some(match condition {
                    Some(existing) => f.binop("or", &existing, &check),
                    None => check,
                });
            }
            let condition = condition
                .ok_or_else(|| CodegenError::internal("CASE arm without a usable label"))?;
            f.cond_branch(&condition, &body_label, &next_label);
            f.start_block(&body_label);
            self.lower_block_stmts(f, info, &arm.body, loops)?;
            if !f.is_terminated() {
                f.branch(&end);
            }
            f.start_block(&next_label);
        }
        self.lower_block_stmts(f, info, else_body, loops)?;
        if !f.is_terminated() {
            f.branch(&end);
        }
        f.start_block(&end);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_for(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        var: ExprId,
        start: ExprId,
        end: ExprId,
        step: ExprId,
        body: &[StmtId],
        span: Span,
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        let kind = self.scalar_at(self.hir.expr(var).ty, span)?;
        let ScalarKind::Int { signed, width } = kind else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`FOR` control variables must be integers in M4".into(),
                span,
            ));
        };
        let start_value = self.lower_expr(f, info, start)?;
        self.write_place(f, info, var, start_value)?;
        let end_value = self.lower_expr(f, info, end)?;
        let step_value = self.lower_expr(f, info, step)?;

        let header = f.next_block_label();
        let body_label = f.next_block_label();
        let increment = f.next_block_label();
        let exit = f.next_block_label();
        f.branch(&header);
        f.start_block(&header);
        let current = self.read_place(f, info, var)?;
        let positive = if signed {
            f.icmp("sgt", &step_value, &Value::int(width, 0))
        } else {
            Value::bool(true)
        };
        let le = if signed {
            f.icmp("sle", &current, &end_value)
        } else {
            f.icmp("ule", &current, &end_value)
        };
        let ge = if signed {
            f.icmp("sge", &current, &end_value)
        } else {
            f.icmp("uge", &current, &end_value)
        };
        let condition = f.select(&positive, &le, &ge);
        f.cond_branch(&condition, &body_label, &exit);

        f.start_block(&body_label);
        loops.push(LoopTarget {
            break_label: exit.clone(),
            continue_label: increment.clone(),
        });
        self.lower_block_stmts(f, info, body, loops)?;
        loops.pop();
        if !f.is_terminated() {
            f.branch(&increment);
        }

        f.start_block(&increment);
        let current = self.read_place(f, info, var)?;
        let next = self.checked_increment(f, kind, &current, &step_value, span)?;
        self.write_place(f, info, var, next)?;
        self.record_checkpoint(f, span, crate::unit::checkpoint_flags::BACK_EDGE);
        f.branch(&header);
        f.start_block(&exit);
        Ok(())
    }

    fn checked_increment(
        &mut self,
        f: &mut Function,
        kind: ScalarKind,
        current: &Value,
        step: &Value,
        span: Span,
    ) -> Res<Value> {
        let ScalarKind::Int { signed, width } = kind else {
            return Err(CodegenError::internal("FOR control is not an integer"));
        };
        let ty = IrType::Int(width);
        let name = format!(
            "llvm.{}{}.with.overflow.i{width}",
            if signed { 's' } else { 'u' },
            "add"
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
        let aggregate = f
            .call(
                IrType::Overflow(width),
                &name,
                &[current.clone(), step.clone()],
            )
            .ok_or_else(|| CodegenError::internal("checked add returned void"))?;
        let value = f.extract_value(&aggregate, 0, ty);
        let overflow = f.extract_value(&aggregate, 1, IrType::Int(1));
        self.trap_if(f, &overflow, crate::abi::trap::OVERFLOW, span)?;
        Ok(value)
    }

    fn lower_while(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        cond: ExprId,
        body: &[StmtId],
        span: Span,
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        let condition_label = f.next_block_label();
        let back_edge = self
            .checkpoint_collection_enabled()
            .then(|| f.next_block_label());
        let body_label = f.next_block_label();
        let exit = f.next_block_label();
        f.branch(&condition_label);
        f.start_block(&condition_label);
        let condition = self.lower_condition(f, info, cond)?;
        f.cond_branch(&condition, &body_label, &exit);
        f.start_block(&body_label);
        loops.push(LoopTarget {
            break_label: exit.clone(),
            continue_label: back_edge.clone().unwrap_or_else(|| condition_label.clone()),
        });
        self.lower_block_stmts(f, info, body, loops)?;
        loops.pop();
        if !f.is_terminated() {
            f.branch(back_edge.as_ref().unwrap_or(&condition_label));
        }
        if let Some(back_edge) = back_edge {
            f.start_block(&back_edge);
            self.record_checkpoint(f, span, crate::unit::checkpoint_flags::BACK_EDGE);
            f.branch(&condition_label);
        }
        f.start_block(&exit);
        Ok(())
    }

    fn lower_repeat(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        body: &[StmtId],
        cond: ExprId,
        span: Span,
        loops: &mut Vec<LoopTarget>,
    ) -> Res<()> {
        let body_label = f.next_block_label();
        let condition_label = f.next_block_label();
        let back_edge = self
            .checkpoint_collection_enabled()
            .then(|| f.next_block_label());
        let exit = f.next_block_label();
        f.branch(&body_label);
        f.start_block(&body_label);
        loops.push(LoopTarget {
            break_label: exit.clone(),
            continue_label: condition_label.clone(),
        });
        self.lower_block_stmts(f, info, body, loops)?;
        loops.pop();
        if !f.is_terminated() {
            f.branch(&condition_label);
        }
        f.start_block(&condition_label);
        let condition = self.lower_condition(f, info, cond)?;
        f.cond_branch(&condition, &exit, back_edge.as_ref().unwrap_or(&body_label));
        if let Some(back_edge) = back_edge {
            f.start_block(&back_edge);
            self.record_checkpoint(f, span, crate::unit::checkpoint_flags::BACK_EDGE);
            f.branch(&body_label);
        }
        f.start_block(&exit);
        Ok(())
    }

    fn lower_condition(&mut self, f: &mut Function, info: &BodyInfo, id: ExprId) -> Res<Value> {
        let value = self.lower_expr(f, info, id)?;
        if value.ty != IrType::Int(1) {
            return Err(CodegenError::internal(format!(
                "condition has type {} instead of i1",
                value.ty.render()
            )));
        }
        Ok(value)
    }

    fn compare_eq(
        &mut self,
        f: &mut Function,
        kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
    ) -> Value {
        if matches!(kind, ScalarKind::Real { .. }) {
            f.fcmp("oeq", lhs, rhs)
        } else {
            f.icmp("eq", lhs, rhs)
        }
    }

    fn compare_ordered(
        &mut self,
        f: &mut Function,
        op: BinOp,
        kind: ScalarKind,
        lhs: &Value,
        rhs: &Value,
    ) -> Value {
        match kind {
            ScalarKind::Real { .. } => {
                let predicate = match op {
                    BinOp::Lt => "olt",
                    BinOp::Le => "ole",
                    BinOp::Gt => "ogt",
                    BinOp::Ge => "oge",
                    _ => "oeq",
                };
                f.fcmp(predicate, lhs, rhs)
            }
            _ => {
                let predicate = match (op, kind.is_signed()) {
                    (BinOp::Lt, true) => "slt",
                    (BinOp::Le, true) => "sle",
                    (BinOp::Gt, true) => "sgt",
                    (BinOp::Ge, true) => "sge",
                    (BinOp::Lt, false) => "ult",
                    (BinOp::Le, false) => "ule",
                    (BinOp::Gt, false) => "ugt",
                    (BinOp::Ge, false) => "uge",
                    _ => "eq",
                };
                f.icmp(predicate, lhs, rhs)
            }
        }
    }

    fn case_value(&self, kind: ScalarKind, value: &ConstValue, span: Span) -> Res<Value> {
        let integer = match value {
            ConstValue::Int(value) | ConstValue::Time(value) | ConstValue::Date { value, .. } => {
                *value
            }
            ConstValue::Char(value) => i128::from(*value),
            ConstValue::Bool(value) => i128::from(u8::from(*value)),
            ConstValue::Enum { value, .. } => *value,
            ConstValue::String(_) => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "string CASE labels are M5".into(),
                    span,
                ));
            }
            ConstValue::Real(_) => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "real CASE labels are M5".into(),
                    span,
                ));
            }
            ConstValue::Null => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "NULL CASE labels are M5".into(),
                    span,
                ));
            }
        };
        match kind {
            ScalarKind::Bool => Ok(Value::bool(integer != 0)),
            ScalarKind::Int { width, .. } | ScalarKind::Char { width } => {
                Ok(Value::int(width, integer))
            }
            ScalarKind::Time | ScalarKind::Date(_) => Ok(Value::int(64, integer)),
            ScalarKind::Real { .. } => {
                Err(CodegenError::internal("real CASE label reached case_value"))
            }
        }
    }
}
