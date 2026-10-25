//! Constant expression evaluation and initializer lowering.

use stc_hir::{
    BinOp, CharTy, ConstValue, DateTy, DefId, DefKind, ExprId, ExprKindHir, FieldId, LiteralHir,
    RealTy, StringKind, TimeTy, Ty, TyId,
};
use stc_lexer::{ElementaryType, Literal, NumericKind, Radix, Sign, TimeUnit};
use stc_span::Span;
use stc_syntax::SyntaxKind;
use stc_syntax::ast::AstNode;
use stc_syntax::ast::{
    Arg, BinaryExpr, CallExpr, Expr, Initializer, LiteralExpr, ParenExpr, PathExpr, StructInit,
    UnaryExpr,
};

use crate::Analyzer;
use crate::resolve::Lookup;
use crate::types::fits_int;

impl Analyzer<'_> {
    /// Evaluates an expression expected to yield an integer constant.
    pub(crate) fn eval_const_int(&mut self, expr: &Expr, ns: DefId) -> Option<i128> {
        let (span, value) = self.eval_const_expr(expr, None, ns)?;
        let _ = span;
        match value {
            ConstValue::Int(value) | ConstValue::Time(value) => Some(value),
            ConstValue::Enum { value, .. } => Some(value),
            ConstValue::Char(value) => Some(i128::from(value)),
            ConstValue::Bool(value) => Some(i128::from(value)),
            _ => {
                self.error(
                    crate::codes::CONST_FAILURE,
                    "constant expression must be an integer",
                    expr.token_span(),
                );
                None
            }
        }
    }

    /// Evaluates a constant expression, optionally adapting it to `expected`.
    pub(crate) fn eval_const_expr(
        &mut self,
        expr: &Expr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<(TyId, ConstValue)> {
        let span = expr.token_span();
        let result = match expr {
            Expr::Literal(literal) => self.eval_literal(literal, expected),
            Expr::Paren(paren) => self.eval_paren(paren, expected, ns),
            Expr::Unary(unary) => self.eval_unary(unary, expected, ns),
            Expr::Binary(binary) => self.eval_binary(binary, expected, ns),
            Expr::Path(path) => self.eval_const_path(path, ns),
            Expr::RefValue(value) if value.is_null() => {
                Some((self.hir.types.null(), ConstValue::Null))
            }
            Expr::Call(call) => self.eval_const_conversion(call, expected, ns),
            _ => {
                self.error(
                    crate::codes::CONST_NOT_CONSTANT,
                    "expression is not a constant",
                    span,
                );
                return None;
            }
        };
        match result {
            Some((ty, value)) => match expected {
                Some(expected) => self.adapt_const(expected, ty, value, span),
                None => Some((ty, value)),
            },
            None => None,
        }
    }

    fn eval_paren(
        &mut self,
        paren: &ParenExpr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<(TyId, ConstValue)> {
        let inner = paren.inner()?;
        let (ty, value) = self.eval_const_expr(&inner, expected, ns)?;
        Some((ty, value))
    }

    fn eval_literal(
        &mut self,
        literal: &LiteralExpr,
        expected: Option<TyId>,
    ) -> Option<(TyId, ConstValue)> {
        let span = literal.token_span();
        let token = literal.token()?;
        match token.kind() {
            SyntaxKind::KwTrue => return Some((self.hir.types.bool(), ConstValue::Bool(true))),
            SyntaxKind::KwFalse => return Some((self.hir.types.bool(), ConstValue::Bool(false))),
            _ => {}
        }
        let literal_value = token.literal()?;
        match literal_value {
            Literal::Numeric(numeric) => match numeric.kind {
                NumericKind::Integer => {
                    let value = parse_int(&numeric.text, numeric.radix)?;
                    self.int_const(value, expected, span)
                }
                NumericKind::Real => {
                    let value: f64 = numeric.text.parse().ok()?;
                    self.real_const(value, expected, span)
                }
            },
            Literal::Typed(typed) => self.typed_const(typed, expected, span),
            Literal::String(string) => {
                let kind = match string.encoding {
                    stc_lexer::StringEncoding::Sbcs => StringKind::Sbcs,
                    stc_lexer::StringEncoding::Dbcs => StringKind::Dbcs,
                    stc_lexer::StringEncoding::Utf8 => StringKind::Unicode,
                };
                let natural = self
                    .hir
                    .types
                    .string(kind, u32::try_from(string.units.len()).unwrap_or(1).max(1));
                self.string_const(natural, string.units.clone(), expected, span)
            }
            Literal::Char(character) => {
                let value = character.value?;
                let ty = match character.kind {
                    stc_lexer::CharKind::Char => self.hir.types.char_ty(CharTy::Char),
                    stc_lexer::CharKind::WChar => self.hir.types.char_ty(CharTy::WChar),
                    stc_lexer::CharKind::UChar => self.hir.types.char_ty(CharTy::UChar),
                };
                Some((ty, ConstValue::Char(value)))
            }
            Literal::Duration(duration) => {
                let value = duration_ns(duration)?;
                let ty = if duration.long {
                    self.hir.types.time(TimeTy::LTime)
                } else {
                    self.hir.types.time(TimeTy::Time)
                };
                Some((ty, ConstValue::Time(value)))
            }
            Literal::Date(date) => {
                let days = days_from_civil(date.year, date.month, date.day)?;
                // D-089: DATE is days; LDATE is nanoseconds since the epoch.
                let (ty, value) = if date.long {
                    (
                        self.hir.types.date(DateTy::LDate),
                        days.checked_mul(86_400_000_000_000)?,
                    )
                } else {
                    (self.hir.types.date(DateTy::Date), days)
                };
                Some((
                    ty,
                    ConstValue::Date {
                        kind: self.date_kind_of(ty),
                        value,
                    },
                ))
            }
            Literal::Tod(tod) => {
                let value = tod_ns(tod.hour, tod.minute, tod.second, tod.fraction.as_deref())?;
                let ty = if tod.long {
                    self.hir.types.date(DateTy::LTod)
                } else {
                    self.hir.types.date(DateTy::Tod)
                };
                Some((
                    ty,
                    ConstValue::Date {
                        kind: self.date_kind_of(ty),
                        value,
                    },
                ))
            }
            Literal::DateTime(datetime) => {
                let days =
                    days_from_civil(datetime.date.year, datetime.date.month, datetime.date.day)?;
                let ns = tod_ns(
                    datetime.tod.hour,
                    datetime.tod.minute,
                    datetime.tod.second,
                    datetime.tod.fraction.as_deref(),
                )?;
                let value = days.checked_mul(86_400_000_000_000)?.checked_add(ns)?;
                let ty = if datetime.long || datetime.date.long || datetime.tod.long {
                    self.hir.types.date(DateTy::LDateTime)
                } else {
                    self.hir.types.date(DateTy::DateTime)
                };
                Some((
                    ty,
                    ConstValue::Date {
                        kind: self.date_kind_of(ty),
                        value,
                    },
                ))
            }
            Literal::Direct(_) => {
                self.error(
                    crate::codes::CONST_NOT_CONSTANT,
                    "a direct variable is not a constant",
                    span,
                );
                None
            }
        }
    }

    fn int_const(
        &mut self,
        value: i128,
        expected: Option<TyId>,
        span: Span,
    ) -> Option<(TyId, ConstValue)> {
        if let Some(expected) = expected
            && !self.is_enum_like(expected)
        {
            let base = self.scalar_base(expected);
            match self.hir.types.get(base) {
                Ty::Bool => {
                    if value == 0 || value == 1 {
                        return Some((expected, ConstValue::Bool(value == 1)));
                    }
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "integer constant used as `BOOL` must be 0 or 1",
                        span,
                    );
                    return None;
                }
                Ty::Real(real) => {
                    let value = value as f64;
                    let ty = self.hir.types.real(*real);
                    return Some((ty, ConstValue::Real(value)));
                }
                Ty::Int(int) => {
                    if !fits_int(value, int.signed, int.width) {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            format!("constant {value} does not fit {}", self.type_name(base)),
                            span,
                        );
                        return None;
                    }
                    if let Ty::Named(def) = self.hir.types.get(expected)
                        && let DefKind::Subrange { lower, upper, .. } =
                            &self.hir.defs[def.index()].kind
                        && !(*lower..=*upper).contains(&value)
                    {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            format!("constant {value} is outside the subrange bounds"),
                            span,
                        );
                        return None;
                    }
                    return Some((expected, ConstValue::Int(value)));
                }
                Ty::BitString(width) => {
                    if !fits_int(value, false, *width) {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            format!("constant {value} does not fit {}", self.type_name(base)),
                            span,
                        );
                        return None;
                    }
                    return Some((expected, ConstValue::Int(value)));
                }
                Ty::Time(_) | Ty::Date(_) => {
                    // Integer literals are not implicitly times/dates.
                }
                _ => {}
            }
        }
        let ty = if i32::try_from(value).is_ok() {
            self.hir.types.int(true, 32)
        } else if i64::try_from(value).is_ok() {
            self.hir.types.int(true, 64)
        } else if u64::try_from(value).is_ok() {
            self.hir.types.int(false, 64)
        } else {
            self.error(
                crate::codes::CONST_FAILURE,
                "integer constant is too large",
                span,
            );
            return None;
        };
        Some((ty, ConstValue::Int(value)))
    }

    fn real_const(
        &mut self,
        value: f64,
        expected: Option<TyId>,
        _span: Span,
    ) -> Option<(TyId, ConstValue)> {
        let ty = if let Some(expected) = expected {
            if let Ty::Real(real) = self.hir.types.get(self.scalar_base(expected)) {
                self.hir.types.real(*real)
            } else {
                self.hir.types.real(RealTy::Real)
            }
        } else {
            self.hir.types.real(RealTy::Real)
        };
        Some((ty, ConstValue::Real(value)))
    }

    fn string_const(
        &mut self,
        natural: TyId,
        units: Vec<u32>,
        expected: Option<TyId>,
        span: Span,
    ) -> Option<(TyId, ConstValue)> {
        let ty = match expected {
            Some(expected) => {
                if let Ty::String(expected_string) = self.hir.types.get(self.scalar_base(expected))
                {
                    let Ty::String(natural_string) = self.hir.types.get(natural) else {
                        return Some((natural, ConstValue::String(units)));
                    };
                    if expected_string.kind != natural_string.kind {
                        self.error(
                            crate::codes::TYPE_MISMATCH,
                            "string literal encoding does not match the target type",
                            span,
                        );
                        return None;
                    }
                    if units.len() as u64 > u64::from(expected_string.capacity) {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            "string literal exceeds the target capacity",
                            span,
                        );
                        return None;
                    }
                    expected
                } else {
                    natural
                }
            }
            None => natural,
        };
        Some((ty, ConstValue::String(units)))
    }

    fn typed_const(
        &mut self,
        typed: &stc_lexer::TypedLiteral,
        expected: Option<TyId>,
        span: Span,
    ) -> Option<(TyId, ConstValue)> {
        let ty = match self.elementary_type_ty(typed.ty) {
            Some(ty) => ty,
            None => {
                self.error(
                    crate::codes::TYPE_INVALID,
                    "unsupported typed literal",
                    span,
                );
                return None;
            }
        };
        match &typed.value {
            stc_lexer::TypedValue::Bool(value) => {
                let value = *value;
                Some((self.hir.types.bool(), ConstValue::Bool(value)))
            }
            stc_lexer::TypedValue::Numeric(numeric) => {
                if numeric.kind == NumericKind::Real {
                    let value: f64 = numeric.text.parse().ok()?;
                    return Some((ty, ConstValue::Real(value)));
                }
                let mut value = parse_int(&numeric.text, numeric.radix)?;
                if typed.sign == Some(Sign::Minus) {
                    value = value.checked_neg()?;
                }
                let base = self.scalar_base(ty);
                match self.hir.types.get(base) {
                    Ty::Int(int) => {
                        if !fits_int(value, int.signed, int.width) {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                format!(
                                    "typed literal value {value} does not fit {}",
                                    self.type_name(ty)
                                ),
                                span,
                            );
                            return None;
                        }
                    }
                    Ty::BitString(width) => {
                        if !fits_int(value, false, *width) {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                format!(
                                    "typed literal value {value} does not fit {}",
                                    self.type_name(ty)
                                ),
                                span,
                            );
                            return None;
                        }
                    }
                    Ty::Char(_) => {
                        return Some((ty, ConstValue::Char(u32::try_from(value).ok()?)));
                    }
                    _ => {}
                }
                let _ = expected;
                Some((ty, ConstValue::Int(value)))
            }
        }
    }

    fn elementary_type_ty(&mut self, ty: ElementaryType) -> Option<TyId> {
        let types = &self.hir.types;
        Some(match ty {
            ElementaryType::Bool => types.bool(),
            ElementaryType::Sint => types.int(true, 8),
            ElementaryType::Int => types.int(true, 16),
            ElementaryType::Dint => types.int(true, 32),
            ElementaryType::Lint => types.int(true, 64),
            ElementaryType::Usint => types.int(false, 8),
            ElementaryType::Uint => types.int(false, 16),
            ElementaryType::Udint => types.int(false, 32),
            ElementaryType::Ulint => types.int(false, 64),
            ElementaryType::Byte => types.bit_string(8),
            ElementaryType::Word => types.bit_string(16),
            ElementaryType::Dword => types.bit_string(32),
            ElementaryType::Lword => types.bit_string(64),
            ElementaryType::Real => types.real(RealTy::Real),
            ElementaryType::Lreal => types.real(RealTy::LReal),
            ElementaryType::Char => types.char_ty(CharTy::Char),
            ElementaryType::Wchar => types.char_ty(CharTy::WChar),
            ElementaryType::Uchar => types.char_ty(CharTy::UChar),
        })
    }

    fn eval_unary(
        &mut self,
        unary: &UnaryExpr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<(TyId, ConstValue)> {
        let operator = unary.operator()?;
        let operand = unary.operand()?;
        let (ty, value) = self.eval_const_expr(&operand, expected, ns)?;
        let span = unary.token_span();
        match operator.kind() {
            SyntaxKind::PunctPlus => Some((ty, value)),
            SyntaxKind::PunctMinus => match value {
                ConstValue::Int(value) => {
                    let value = value.checked_neg()?;
                    if let Ty::Int(int) = self.hir.types.get(self.scalar_base(ty))
                        && !fits_int(value, int.signed, int.width)
                    {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            "constant negation overflows",
                            span,
                        );
                        return None;
                    }
                    Some((ty, ConstValue::Int(value)))
                }
                ConstValue::Real(value) => Some((ty, ConstValue::Real(-value))),
                ConstValue::Time(value) => Some((ty, ConstValue::Time(value.checked_neg()?))),
                _ => {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "unary `-` requires a numeric constant",
                        span,
                    );
                    None
                }
            },
            SyntaxKind::KwNot => match value {
                ConstValue::Bool(value) => Some((ty, ConstValue::Bool(!value))),
                ConstValue::Int(value) => {
                    let width = match self.hir.types.get(self.scalar_base(ty)) {
                        Ty::BitString(width) => *width,
                        _ => {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                "`NOT` requires `BOOL` or a bit string",
                                span,
                            );
                            return None;
                        }
                    };
                    let mask = bit_mask(width);
                    Some((ty, ConstValue::Int((!value) & mask)))
                }
                _ => {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "`NOT` requires `BOOL` or a bit string",
                        span,
                    );
                    None
                }
            },
            _ => None,
        }
    }

    fn eval_binary(
        &mut self,
        binary: &BinaryExpr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<(TyId, ConstValue)> {
        let operator = binary.operator()?;
        let lhs = binary.lhs()?;
        let rhs = binary.rhs()?;
        let (lhs_ty, lhs_value) = self.eval_const_expr(&lhs, None, ns)?;
        let (rhs_ty, rhs_value) = self.eval_const_expr(&rhs, None, ns)?;
        let span = binary.token_span();
        let op = bin_op(operator.kind())?;
        let lhs = int_value(&lhs_value);
        let rhs = int_value(&rhs_value);
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Power => {
                if let (Some(a), Some(b)) = (real_value(&lhs_value), real_value(&rhs_value)) {
                    let ty = if matches!(
                        self.hir.types.get(self.scalar_base(lhs_ty)),
                        Ty::Real(RealTy::LReal)
                    ) || matches!(
                        self.hir.types.get(self.scalar_base(rhs_ty)),
                        Ty::Real(RealTy::LReal)
                    ) {
                        self.hir.types.real(RealTy::LReal)
                    } else {
                        self.hir.types.real(RealTy::Real)
                    };
                    let value = match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => {
                            if b == 0.0 {
                                self.error(
                                    crate::codes::CONST_FAILURE,
                                    "division by zero in constant expression",
                                    span,
                                );
                                return None;
                            }
                            a / b
                        }
                        BinOp::Power => a.powf(b),
                        BinOp::Mod => {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                "`MOD` requires integer operands",
                                span,
                            );
                            return None;
                        }
                        _ => unreachable!(),
                    };
                    let _ = expected;
                    return Some((ty, ConstValue::Real(value)));
                }
                let (Some(a), Some(b)) = (lhs, rhs) else {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "arithmetic requires numeric constants",
                        span,
                    );
                    return None;
                };
                if !self.numeric_types(lhs_ty) || !self.numeric_types(rhs_ty) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "arithmetic requires numeric constants",
                        span,
                    );
                    return None;
                }
                let ty = self.promote_int(lhs_ty, rhs_ty);
                let value = match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div => {
                        if b == 0 {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                "division by zero in constant expression",
                                span,
                            );
                            return None;
                        }
                        a.checked_div(b)
                    }
                    BinOp::Mod => {
                        if b == 0 {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                "division by zero in constant expression",
                                span,
                            );
                            return None;
                        }
                        a.checked_rem(b)
                    }
                    BinOp::Power => {
                        if b < 0 {
                            self.error(
                                crate::codes::CONST_FAILURE,
                                "negative exponent in integer constant expression",
                                span,
                            );
                            return None;
                        }
                        a.checked_pow(u32::try_from(b).ok()?)
                    }
                    _ => None,
                };
                let Some(value) = value else {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "constant arithmetic overflow",
                        span,
                    );
                    return None;
                };
                if let Ty::Int(int) = self.hir.types.get(self.scalar_base(ty))
                    && !fits_int(value, int.signed, int.width)
                {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "constant arithmetic overflow",
                        span,
                    );
                    return None;
                }
                Some((ty, ConstValue::Int(value)))
            }
            BinOp::And | BinOp::Or | BinOp::Xor => {
                if let (ConstValue::Bool(a), ConstValue::Bool(b)) = (&lhs_value, &rhs_value) {
                    let value = match op {
                        BinOp::And => *a && *b,
                        BinOp::Or => *a || *b,
                        _ => *a ^ *b,
                    };
                    let _ = expected;
                    return Some((self.hir.types.bool(), ConstValue::Bool(value)));
                }
                let (Some(a), Some(b)) = (lhs, rhs) else {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "bitwise operation requires bit-string constants",
                        span,
                    );
                    return None;
                };
                let width = match self.hir.types.get(self.scalar_base(lhs_ty)) {
                    Ty::BitString(width) => *width,
                    _ => {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            "bitwise operation requires bit-string constants",
                            span,
                        );
                        return None;
                    }
                };
                let mask = bit_mask(width);
                let value = match op {
                    BinOp::And => a & b,
                    BinOp::Or => a | b,
                    _ => a ^ b,
                } & mask;
                Some((lhs_ty, ConstValue::Int(value)))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let Some(ordering) = compare_const(&lhs_value, &rhs_value) else {
                    self.error(
                        crate::codes::TYPE_INVALID_OPERAND,
                        "comparison operands are not comparable constants",
                        span,
                    );
                    return None;
                };
                let value = match op {
                    BinOp::Eq => ordering == std::cmp::Ordering::Equal,
                    BinOp::Ne => ordering != std::cmp::Ordering::Equal,
                    BinOp::Lt => ordering == std::cmp::Ordering::Less,
                    BinOp::Le => ordering != std::cmp::Ordering::Greater,
                    BinOp::Gt => ordering == std::cmp::Ordering::Greater,
                    BinOp::Ge => ordering != std::cmp::Ordering::Less,
                    _ => unreachable!(),
                };
                let _ = expected;
                Some((self.hir.types.bool(), ConstValue::Bool(value)))
            }
        }
    }

    fn date_kind_of(&self, ty: TyId) -> DateTy {
        match self.hir.types.get(self.scalar_base(ty)) {
            Ty::Date(kind) => *kind,
            _ => DateTy::Date,
        }
    }

    fn numeric_types(&self, ty: TyId) -> bool {
        matches!(
            self.hir.types.get(self.scalar_base(ty)),
            Ty::Int(_) | Ty::Real(_)
        )
    }

    fn promote_int(&self, lhs: TyId, rhs: TyId) -> TyId {
        let lhs = self.scalar_base(lhs);
        let rhs = self.scalar_base(rhs);
        match (self.hir.types.get(lhs), self.hir.types.get(rhs)) {
            (Ty::Int(a), Ty::Int(b)) => {
                let width = a.width.max(b.width);
                self.hir.types.int(a.signed || b.signed, width)
            }
            (Ty::Int(_), _) => lhs,
            (_, Ty::Int(_)) => rhs,
            _ => lhs,
        }
    }

    fn eval_const_conversion(
        &mut self,
        call: &CallExpr,
        expected: Option<TyId>,
        ns: DefId,
    ) -> Option<(TyId, ConstValue)> {
        let Some(callee) = call.callee() else {
            return None;
        };
        let Expr::Path(path) = callee else {
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "function call is not constant",
                call.token_span(),
            );
            return None;
        };
        let Some((from, to)) = self.conversion_name(&path) else {
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "function call is not constant",
                call.token_span(),
            );
            return None;
        };
        let arguments = call.arguments();
        if arguments.len() != 1 {
            self.error(
                crate::codes::CALL_ARITY,
                "conversion functions take one argument",
                call.token_span(),
            );
            return None;
        }
        let value_expr = match &arguments[0] {
            Arg::Positional(expr) => expr.clone(),
            _ => {
                self.error(
                    crate::codes::CALL_ARGUMENT,
                    "conversion arguments are positional",
                    call.token_span(),
                );
                return None;
            }
        };
        let (source_ty, value) = self.eval_const_expr(&value_expr, Some(from), ns)?;
        let _ = expected;
        if !self.can_convert(to, source_ty) {
            self.error(
                crate::codes::CONV_NONE,
                format!(
                    "no conversion from {} to {}",
                    self.type_name(source_ty),
                    self.type_name(to)
                ),
                call.token_span(),
            );
            return None;
        }
        let converted = self.convert_const(to, value, call.token_span())?;
        Some((to, converted))
    }

    fn conversion_name(&mut self, path: &PathExpr) -> Option<(TyId, TyId)> {
        if path.prefix().is_some() {
            return None;
        }
        let segments = path.segments();
        if segments.len() != 1 {
            return None;
        }
        let name = self.hir.intern(segments[0].text());
        let text = self.symbol_text(name).to_string();
        let (from_name, to_name) = text.split_once("_TO_")?;
        let from = self.type_name_to_ty(from_name)?;
        let to = self.type_name_to_ty(to_name)?;
        Some((from, to))
    }

    pub(crate) fn type_name_to_ty(&mut self, name: &str) -> Option<TyId> {
        let kind = match name {
            "BOOL" => SyntaxKind::KwBool,
            "SINT" => SyntaxKind::KwSint,
            "INT" => SyntaxKind::KwInt,
            "DINT" => SyntaxKind::KwDint,
            "LINT" => SyntaxKind::KwLint,
            "USINT" => SyntaxKind::KwUsint,
            "UINT" => SyntaxKind::KwUint,
            "UDINT" => SyntaxKind::KwUdint,
            "ULINT" => SyntaxKind::KwUlint,
            "BYTE" => SyntaxKind::KwByte,
            "WORD" => SyntaxKind::KwWord,
            "DWORD" => SyntaxKind::KwDword,
            "LWORD" => SyntaxKind::KwLword,
            "REAL" => SyntaxKind::KwReal,
            "LREAL" => SyntaxKind::KwLreal,
            "TIME" => SyntaxKind::KwTime,
            "LTIME" => SyntaxKind::KwLtime,
            "DATE" => SyntaxKind::KwDate,
            "LDATE" => SyntaxKind::KwLdate,
            "TOD" | "TIME_OF_DAY" => SyntaxKind::KwTod,
            "LTOD" | "LTIME_OF_DAY" => SyntaxKind::KwLtod,
            "DT" | "DATE_AND_TIME" => SyntaxKind::KwDt,
            "LDT" | "LDATE_AND_TIME" => SyntaxKind::KwLdt,
            "CHAR" => SyntaxKind::KwChar,
            "WCHAR" => SyntaxKind::KwWchar,
            "UCHAR" => SyntaxKind::KwUchar,
            "STRING" => SyntaxKind::KwString,
            "WSTRING" => SyntaxKind::KwWstring,
            "USTRING" => SyntaxKind::KwUstring,
            _ => return None,
        };
        self.elementary_kind_ty(kind).or_else(|| {
            matches!(
                kind,
                SyntaxKind::KwString | SyntaxKind::KwWstring | SyntaxKind::KwUstring
            )
            .then(|| {
                let kind = match kind {
                    SyntaxKind::KwString => StringKind::Sbcs,
                    SyntaxKind::KwWstring => StringKind::Dbcs,
                    _ => StringKind::Unicode,
                };
                self.hir.types.string(kind, 255)
            })
        })
    }

    fn convert_const(&mut self, to: TyId, value: ConstValue, span: Span) -> Option<ConstValue> {
        let base = self.scalar_base(to);
        let target = self.hir.types.get(base).clone();
        let int_value = match &value {
            ConstValue::Int(value) | ConstValue::Enum { value, .. } => Some(*value),
            ConstValue::Char(value) => Some(i128::from(*value)),
            ConstValue::Bool(value) => Some(i128::from(u8::from(*value))),
            _ => None,
        };
        match target {
            Ty::Int(int) => {
                let value = int_value?;
                if !fits_int(value, int.signed, int.width) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "conversion result is out of range",
                        span,
                    );
                    return None;
                }
                Some(ConstValue::Int(value))
            }
            Ty::BitString(width) => {
                let value = int_value?;
                if !fits_int(value, false, width) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "conversion result is out of range",
                        span,
                    );
                    return None;
                }
                Some(ConstValue::Int(value))
            }
            Ty::Real(_) => match value {
                ConstValue::Int(value) | ConstValue::Enum { value, .. } => {
                    Some(ConstValue::Real(value as f64))
                }
                ConstValue::Real(value) => Some(ConstValue::Real(value)),
                other => Some(other),
            },
            Ty::Char(_) => match value {
                ConstValue::Int(value) => Some(ConstValue::Char(u32::try_from(value).ok()?)),
                ConstValue::Char(value) => Some(ConstValue::Char(value)),
                other => Some(other),
            },
            Ty::Bool => match value {
                ConstValue::Bool(value) => Some(ConstValue::Bool(value)),
                ConstValue::Int(value) => Some(ConstValue::Bool(value != 0)),
                other => Some(other),
            },
            Ty::Time(_) => match value {
                ConstValue::Time(value) => Some(ConstValue::Time(value)),
                ConstValue::Int(value) => Some(ConstValue::Time(value)),
                other => Some(other),
            },
            Ty::Date(destination) => match value {
                ConstValue::Date { kind, value } => {
                    let converted = match (date_is_days(kind), date_is_days(destination)) {
                        (false, true) => value.checked_div(86_400_000_000_000),
                        (true, false) => value.checked_mul(86_400_000_000_000),
                        _ => Some(value),
                    };
                    let Some(converted) = converted else {
                        self.error(
                            crate::codes::CONST_FAILURE,
                            "date constant does not fit the target type",
                            span,
                        );
                        return None;
                    };
                    Some(ConstValue::Date {
                        kind: destination,
                        value: converted,
                    })
                }
                other => Some(other),
            },
            _ => Some(value),
        }
    }

    fn adapt_const(
        &mut self,
        expected: TyId,
        ty: TyId,
        value: ConstValue,
        span: Span,
    ) -> Option<(TyId, ConstValue)> {
        if expected == ty {
            return Some((ty, value));
        }
        if expected == self.hir.types.error() || ty == self.hir.types.error() {
            return Some((expected, value));
        }
        if self.is_enum_like(expected) {
            self.error(
                crate::codes::TYPE_MISMATCH,
                "a constant cannot be implicitly converted to an enumeration type",
                span,
            );
            return None;
        }
        if let Ty::Named(def) = self.hir.types.get(expected)
            && let DefKind::Subrange { lower, upper, .. } = &self.hir.defs[def.index()].kind
            && let ConstValue::Int(value) = &value
            && !(*lower..=*upper).contains(value)
        {
            self.error(
                crate::codes::CONST_FAILURE,
                "constant is outside the subrange bounds",
                span,
            );
            return None;
        }
        let expected_base = self.scalar_base(expected);
        let ty_base = self.scalar_base(ty);
        let expected_kind = self.hir.types.get(expected_base).clone();
        let source_kind = self.hir.types.get(ty_base).clone();
        match (expected_kind, source_kind, &value) {
            (Ty::Real(_), Ty::Int(_), ConstValue::Int(value)) => {
                Some((expected_base, ConstValue::Real(*value as f64)))
            }
            (Ty::Bool, Ty::Int(_), ConstValue::Int(value)) if *value == 0 || *value == 1 => {
                Some((expected_base, ConstValue::Bool(*value == 1)))
            }
            (Ty::Int(int), Ty::Int(_), ConstValue::Int(value)) => {
                if !fits_int(*value, int.signed, int.width) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "constant does not fit the target type",
                        span,
                    );
                    return None;
                }
                Some((expected_base, ConstValue::Int(*value)))
            }
            (Ty::BitString(width), Ty::Int(_), ConstValue::Int(value)) => {
                if !fits_int(*value, false, width) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "constant does not fit the target type",
                        span,
                    );
                    return None;
                }
                Some((expected_base, ConstValue::Int(*value)))
            }
            (Ty::String(expected_string), Ty::String(_), ConstValue::String(units)) => {
                if units.len() as u64 > u64::from(expected_string.capacity) {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "string constant exceeds the target capacity",
                        span,
                    );
                    return None;
                }
                Some((expected_base, ConstValue::String(units.clone())))
            }
            (Ty::Time(_), Ty::Time(_), ConstValue::Time(value)) => {
                Some((expected_base, ConstValue::Time(*value)))
            }
            (Ty::Date(_), Ty::Date(_), ConstValue::Date { kind, value }) => {
                let destination = self.date_kind_of(expected_base);
                let converted = match (date_is_days(*kind), date_is_days(destination)) {
                    (false, true) => value.checked_mul(86_400_000_000_000),
                    (true, false) => value.checked_div(86_400_000_000_000),
                    _ => Some(*value),
                };
                let Some(converted) = converted else {
                    self.error(
                        crate::codes::CONST_FAILURE,
                        "date constant does not fit the target type",
                        span,
                    );
                    return None;
                };
                Some((
                    expected_base,
                    ConstValue::Date {
                        kind: destination,
                        value: converted,
                    },
                ))
            }
            _ => {
                if self.can_assign(expected, ty) != crate::types::Conv::No {
                    Some((expected, value))
                } else {
                    self.error(
                        crate::codes::TYPE_MISMATCH,
                        format!(
                            "cannot use {} where {} is expected",
                            self.type_name(ty),
                            self.type_name(expected)
                        ),
                        span,
                    );
                    None
                }
            }
        }
    }

    fn eval_const_path(&mut self, path: &PathExpr, ns: DefId) -> Option<(TyId, ConstValue)> {
        if path.prefix().is_some() {
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "`THIS`/`SUPER` is not a constant",
                path.token_span(),
            );
            return None;
        }
        let segments = path.segments();
        let mut scope = ns;
        for (index, segment) in segments.iter().enumerate() {
            let symbol = self.hir.intern(segment.text());
            let last = index + 1 == segments.len();
            let lookup = self.lookup_scope(scope, symbol);
            match lookup {
                Lookup::Constant(binding) => {
                    if !last {
                        self.error(
                            crate::codes::CONST_NOT_CONSTANT,
                            "member access is not a constant",
                            path.token_span(),
                        );
                        return None;
                    }
                    return Some((binding.ty, binding.value));
                }
                Lookup::Entries(defs) => {
                    if defs.len() != 1 {
                        self.error(
                            crate::codes::CONST_NOT_CONSTANT,
                            "ambiguous or overloaded name is not a constant",
                            path.token_span(),
                        );
                        return None;
                    }
                    let def = defs[0];
                    if last {
                        match &self.hir.defs[def.index()].kind {
                            DefKind::Global { constant: true, .. } => {
                                let value = self.eval_constant_global(def)?;
                                let ty = match &self.hir.defs[def.index()].kind {
                                    DefKind::Global { ty, .. } => *ty,
                                    _ => self.hir.types.error(),
                                };
                                return Some((ty, value));
                            }
                            DefKind::Enum { values, .. } => {
                                let _ = values;
                                self.error(
                                    crate::codes::CONST_NOT_CONSTANT,
                                    "an enumeration type is not a value",
                                    path.token_span(),
                                );
                                return None;
                            }
                            _ => {
                                self.error(
                                    crate::codes::CONST_NOT_CONSTANT,
                                    "name is not a constant",
                                    path.token_span(),
                                );
                                return None;
                            }
                        }
                    }
                    if !crate::resolve::is_namespace(&self.hir.defs[def.index()].kind) {
                        // Enum/named-value values may be qualified by the type name.
                        let qualified = match &self.hir.defs[def.index()].kind {
                            DefKind::Enum { values, .. } => Some((true, values.clone())),
                            DefKind::NamedValue { values, .. } => Some((false, values.clone())),
                            _ => None,
                        };
                        if let Some((is_enum, values)) = qualified {
                            let next = &segments[index + 1];
                            let next_symbol = self.hir.intern(next.text());
                            if let Some(value) =
                                values.iter().find(|value| value.name == next_symbol)
                            {
                                let ty = self.hir.named_type(def);
                                let value = if is_enum {
                                    ConstValue::Enum {
                                        def,
                                        value: value.value,
                                    }
                                } else {
                                    ConstValue::Int(value.value)
                                };
                                if index + 2 != segments.len() {
                                    self.error(
                                        crate::codes::CONST_NOT_CONSTANT,
                                        "member access is not a constant",
                                        path.token_span(),
                                    );
                                    return None;
                                }
                                return Some((ty, value));
                            }
                        }
                        self.error(
                            crate::codes::CONST_NOT_CONSTANT,
                            "member access is not a constant",
                            path.token_span(),
                        );
                        return None;
                    }
                    scope = def;
                }
                Lookup::Ambiguous(spans) => {
                    let text = path.text().to_string();
                    self.report_ambiguous(&text, path.token_span(), &spans);
                    return None;
                }
                Lookup::NotFound => {
                    self.error(
                        crate::codes::NAME_UNKNOWN,
                        format!("unknown name `{}`", path.text()),
                        path.token_span(),
                    );
                    return None;
                }
            }
        }
        None
    }

    fn eval_constant_global(&mut self, def: DefId) -> Option<ConstValue> {
        if let Some(value) = self.const_globals.get(&def) {
            return Some(value.clone());
        }
        if !self.const_in_progress.insert(def) {
            let span = self.hir.defs[def.index()].span;
            self.error(
                crate::codes::CONST_FAILURE,
                "cyclic constant initializer",
                span,
            );
            return None;
        }
        let Some(site) = self
            .decl_sites
            .iter()
            .find(|site| site.id == crate::DeclId::Global(def))
            .cloned()
        else {
            self.const_in_progress.remove(&def);
            let span = self.hir.defs[def.index()].span;
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "constant has no initializer",
                span,
            );
            return None;
        };
        let Some(init) = site.init else {
            self.const_in_progress.remove(&def);
            self.error(
                crate::codes::CONST_NOT_CONSTANT,
                "constant has no initializer",
                site.name_span,
            );
            return None;
        };
        let expected = self.decl_type(site.id);
        let result = self.eval_initializer_expr(&init, expected, site.ns);
        self.const_in_progress.remove(&def);
        if let Some(value) = &result {
            self.const_globals.insert(def, value.clone());
        }
        result
    }

    fn eval_initializer_expr(
        &mut self,
        init: &Initializer,
        expected: TyId,
        ns: DefId,
    ) -> Option<ConstValue> {
        match init {
            Initializer::Expr(expr) => {
                let (_, value) = self.eval_const_expr(expr, Some(expected), ns)?;
                Some(value)
            }
            _ => {
                self.error(
                    crate::codes::CONST_NOT_CONSTANT,
                    "aggregate initializer is not a scalar constant",
                    init.token_span(),
                );
                None
            }
        }
    }

    /// Lowers an initializer, applying constant evaluation and aggregate shape
    /// checks.
    pub(crate) fn lower_initializer(
        &mut self,
        init: &Initializer,
        expected: TyId,
        ns: DefId,
    ) -> Option<ExprId> {
        let span = init.token_span();
        match init {
            Initializer::Expr(expr) => {
                if let Expr::RefValue(reference) = expr {
                    if reference.is_null() {
                        let ty = if self.is_class_like(expected)
                            || matches!(self.hir.types.get(expected), Ty::Ref(_))
                            || self.is_pointer_ty(expected)
                        {
                            expected
                        } else {
                            self.hir.types.null()
                        };
                        return Some(self.hir.alloc_expr(ExprKindHir::Null, ty, span));
                    }
                    return self.check_expr_in_namespace(expr, Some(expected), ns);
                }
                let (ty, value) = self.eval_const_expr(expr, Some(expected), ns)?;
                Some(
                    self.hir
                        .alloc_expr(ExprKindHir::Literal(LiteralHir { value }), ty, span),
                )
            }
            Initializer::Array(array) => {
                let Ty::Array(array_ty) = self.hir.types.get(expected).clone() else {
                    self.error(
                        crate::codes::INIT_SHAPE,
                        "array initializer requires an array type",
                        span,
                    );
                    return None;
                };
                let elements = array.elements();
                if let Some(count) = array_ty.element_count()
                    && elements.len() as i128 > count
                {
                    self.error(
                        crate::codes::INIT_SHAPE,
                        "too many array initializer elements",
                        span,
                    );
                }
                let mut lowered = Vec::new();
                for element in &elements {
                    if let Some(id) = self.lower_initializer(element, array_ty.element, ns) {
                        lowered.push(id);
                    }
                }
                Some(self.hir.alloc_expr(
                    ExprKindHir::ArrayInit { elements: lowered },
                    expected,
                    span,
                ))
            }
            Initializer::Struct(structure) => {
                self.lower_struct_initializer(structure, expected, ns, span)
            }
            Initializer::Error(_) => None,
        }
    }

    fn lower_struct_initializer(
        &mut self,
        structure: &StructInit,
        expected: TyId,
        ns: DefId,
        span: Span,
    ) -> Option<ExprId> {
        let mut fields = Vec::new();
        for field_init in structure.fields() {
            let names = field_init.names();
            let Some((field, field_ty)) = self.resolve_field_path(expected, &names) else {
                let text = names
                    .iter()
                    .map(stc_syntax::ast::Name::text)
                    .collect::<Vec<_>>()
                    .join(".");
                self.error(
                    crate::codes::INIT_SHAPE,
                    format!("unknown struct initializer field `{text}`"),
                    field_init.token_span(),
                );
                continue;
            };
            let Some(value) = field_init.value() else {
                continue;
            };
            if let Some(id) = self.lower_initializer(&value, field_ty, ns) {
                fields.push((field, id));
            }
        }
        Some(
            self.hir
                .alloc_expr(ExprKindHir::StructInit { fields }, expected, span),
        )
    }

    fn resolve_field_path(
        &self,
        mut ty: TyId,
        names: &[stc_syntax::ast::Name],
    ) -> Option<(FieldId, TyId)> {
        let mut last = None;
        for name in names {
            let symbol = self.hir.interner.get(name.text())?;
            let field = self.field_lookup(ty, symbol)?;
            let field_ty = self.hir.fields[field.index()].ty;
            ty = field_ty;
            last = Some((field, field_ty));
        }
        last
    }

    pub(crate) fn field_lookup(&self, ty: TyId, symbol: stc_hir::Symbol) -> Option<FieldId> {
        let Ty::Named(def) = self.hir.types.get(ty) else {
            return None;
        };
        let mut current = Some(*def);
        let mut visited = std::collections::BTreeSet::new();
        while let Some(next) = current {
            if !visited.insert(next) {
                return None;
            }
            let fields = match &self.hir.defs[next.index()].kind {
                DefKind::Struct { fields, .. }
                | DefKind::FunctionBlock { fields, .. }
                | DefKind::Class { fields, .. }
                | DefKind::Program { fields, .. } => fields,
                _ => return None,
            };
            if let Some(field) = fields
                .iter()
                .find(|field| self.hir.fields[field.index()].name == symbol)
            {
                return Some(*field);
            }
            current = self.hir.defs[next.index()].base();
        }
        None
    }
}

fn parse_int(text: &str, radix: Radix) -> Option<i128> {
    i128::from_str_radix(text, radix.base()).ok()
}

fn int_value(value: &ConstValue) -> Option<i128> {
    match value {
        ConstValue::Int(value) | ConstValue::Enum { value, .. } => Some(*value),
        ConstValue::Char(value) => Some(i128::from(*value)),
        _ => None,
    }
}

fn real_value(value: &ConstValue) -> Option<f64> {
    match value {
        ConstValue::Real(value) => Some(*value),
        ConstValue::Int(value) | ConstValue::Enum { value, .. } => Some(*value as f64),
        _ => None,
    }
}

fn compare_const(lhs: &ConstValue, rhs: &ConstValue) -> Option<std::cmp::Ordering> {
    match (lhs, rhs) {
        (
            ConstValue::Int(a) | ConstValue::Enum { value: a, .. },
            ConstValue::Int(b) | ConstValue::Enum { value: b, .. },
        ) => Some(a.cmp(b)),
        (ConstValue::Real(a), ConstValue::Real(b)) => a.partial_cmp(b),
        (ConstValue::Int(a) | ConstValue::Enum { value: a, .. }, ConstValue::Real(b)) => {
            (*a as f64).partial_cmp(b)
        }
        (ConstValue::Real(a), ConstValue::Int(b) | ConstValue::Enum { value: b, .. }) => {
            a.partial_cmp(&(*b as f64))
        }
        (ConstValue::Bool(a), ConstValue::Bool(b)) => Some(a.cmp(b)),
        (ConstValue::Char(a), ConstValue::Char(b)) => Some(a.cmp(b)),
        (ConstValue::String(a), ConstValue::String(b)) => Some(a.cmp(b)),
        (ConstValue::Time(a), ConstValue::Time(b)) => Some(a.cmp(b)),
        (ConstValue::Date { value: a, .. }, ConstValue::Date { value: b, .. }) => Some(a.cmp(b)),
        (ConstValue::Null, ConstValue::Null) => Some(std::cmp::Ordering::Equal),
        _ => None,
    }
}

fn bit_mask(width: u8) -> i128 {
    if width >= 127 {
        i128::MAX
    } else {
        (1i128 << width) - 1
    }
}

fn bin_op(kind: SyntaxKind) -> Option<BinOp> {
    Some(match kind {
        SyntaxKind::PunctPlus => BinOp::Add,
        SyntaxKind::PunctMinus => BinOp::Sub,
        SyntaxKind::PunctStar => BinOp::Mul,
        SyntaxKind::PunctSlash => BinOp::Div,
        SyntaxKind::KwMod => BinOp::Mod,
        SyntaxKind::PunctPower => BinOp::Power,
        SyntaxKind::PunctEqual => BinOp::Eq,
        SyntaxKind::PunctNotEqual => BinOp::Ne,
        SyntaxKind::PunctLess => BinOp::Lt,
        SyntaxKind::PunctLessEqual => BinOp::Le,
        SyntaxKind::PunctGreater => BinOp::Gt,
        SyntaxKind::PunctGreaterEqual => BinOp::Ge,
        SyntaxKind::KwAnd | SyntaxKind::PunctAmpersand => BinOp::And,
        SyntaxKind::KwOr => BinOp::Or,
        SyntaxKind::KwXor => BinOp::Xor,
        _ => return None,
    })
}

fn duration_ns(duration: &stc_lexer::DurationLiteral) -> Option<i128> {
    let mut total: i128 = 0;
    for component in &duration.components {
        let unit = match component.unit {
            TimeUnit::Day => 86_400_000_000_000i128,
            TimeUnit::Hour => 3_600_000_000_000,
            TimeUnit::Minute => 60_000_000_000,
            TimeUnit::Second => 1_000_000_000,
            TimeUnit::Millisecond => 1_000_000,
            TimeUnit::Microsecond => 1_000,
            TimeUnit::Nanosecond => 1,
        };
        let integer: i128 = component.value.parse().ok()?;
        let mut value = integer.checked_mul(unit)?;
        if let Some(fraction) = &component.fraction {
            let digits: i128 = fraction.parse().ok()?;
            let scale = 10i128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
            value = value.checked_add(digits.checked_mul(unit)? / scale)?;
        }
        total = total.checked_add(value)?;
    }
    if duration.sign == Some(Sign::Minus) {
        total = total.checked_neg()?;
    }
    Some(total)
}

fn tod_ns(hour: u32, minute: u32, second: u32, fraction: Option<&str>) -> Option<i128> {
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut value = i128::from(hour) * 3_600_000_000_000
        + i128::from(minute) * 60_000_000_000
        + i128::from(second) * 1_000_000_000;
    if let Some(fraction) = fraction {
        let digits: i128 = fraction.parse().ok()?;
        let scale = 10i128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
        value += digits.checked_mul(1_000_000_000)? / scale;
    }
    Some(value)
}

fn days_from_civil(year: u32, month: u32, day: u32) -> Option<i128> {
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day > days_in_month {
        return None;
    }
    let y = i128::from(year);
    let m = i128::from(month);
    let d = i128::from(day);
    let y = y - i128::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn date_is_days(kind: DateTy) -> bool {
    matches!(kind, DateTy::Date)
}
