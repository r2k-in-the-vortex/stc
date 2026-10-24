//! The standard prelude: function signatures and standard FB signatures.

use stc_hir::{
    DefKind, FieldHir, GenericKind, RetainPolicy, StdFnKind, StdFunction, StdParam, StdResult,
    StdRetType, TimeTy, Ty, VarSectionKind, Visibility,
};

use crate::{Analyzer, EMPTY_SPAN};

impl Analyzer<'_> {
    pub(crate) fn install_prelude(&mut self) {
        for kind in ALL_STD_FNS {
            let name = self.hir.intern(kind.as_str());
            let (params, result) = std_signature(*kind);
            let raw = u32::try_from(self.hir.std_fns.len()).expect("std fn limit exceeded");
            self.hir.std_fns.push(StdFunction {
                name,
                kind: *kind,
                params,
                result,
            });
            self.std_fns
                .entry(name)
                .or_default()
                .push(stc_hir::StdFnId::new(raw));
        }
        for (name, inputs, outputs) in STANDARD_FBS.iter().copied() {
            let symbol = self.hir.intern(name);
            let def = self.hir.alloc_def(
                DefKind::FunctionBlock {
                    base: None,
                    interfaces: Vec::new(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    body: None,
                    is_final: false,
                    is_abstract: false,
                    vtable: Vec::new(),
                },
                Some(symbol),
                EMPTY_SPAN,
                Some(self.root_ns),
                Visibility::Public,
            );
            let bool_ty = self.hir.types.bool();
            let time_ty = self.hir.types.time(TimeTy::Time);
            let int_ty = self.hir.types.int(true, 16);
            let mut fields = Vec::new();
            for (field_name, kind) in inputs.iter().chain(outputs.iter()).copied() {
                let ty = match kind {
                    StdFbType::Bool => bool_ty,
                    StdFbType::Time => time_ty,
                    StdFbType::Int => int_ty,
                };
                let section = if inputs.iter().any(|(name, _)| *name == field_name) {
                    VarSectionKind::Input
                } else {
                    VarSectionKind::Output
                };
                let field_name = self.hir.intern(field_name);
                let field = self.hir.alloc_field(FieldHir {
                    name: field_name,
                    ty,
                    owner: def,
                    visibility: Visibility::Public,
                    section,
                    retain: RetainPolicy::None,
                    constant: false,
                    at: None,
                    init: None,
                    span: EMPTY_SPAN,
                });
                fields.push(field);
            }
            if let DefKind::FunctionBlock { fields: slot, .. } =
                &mut self.hir.defs[def.index()].kind
            {
                *slot = fields;
            }
            self.std_fbs.insert(symbol, def);
        }
    }
}

#[derive(Clone, Copy)]
enum StdFbType {
    Bool,
    Time,
    Int,
}

type StdFbSpec<'a> = (
    &'a str,
    &'a [(&'a str, StdFbType)],
    &'a [(&'a str, StdFbType)],
);

const STANDARD_FBS: &[StdFbSpec<'static>] = &[
    (
        "TON",
        &[("IN", StdFbType::Bool), ("PT", StdFbType::Time)],
        &[("Q", StdFbType::Bool), ("ET", StdFbType::Time)],
    ),
    (
        "TOF",
        &[("IN", StdFbType::Bool), ("PT", StdFbType::Time)],
        &[("Q", StdFbType::Bool), ("ET", StdFbType::Time)],
    ),
    (
        "TP",
        &[("IN", StdFbType::Bool), ("PT", StdFbType::Time)],
        &[("Q", StdFbType::Bool), ("ET", StdFbType::Time)],
    ),
    (
        "CTU",
        &[
            ("CU", StdFbType::Bool),
            ("R", StdFbType::Bool),
            ("PV", StdFbType::Int),
        ],
        &[("Q", StdFbType::Bool), ("CV", StdFbType::Int)],
    ),
    (
        "CTD",
        &[
            ("CD", StdFbType::Bool),
            ("LD", StdFbType::Bool),
            ("PV", StdFbType::Int),
        ],
        &[("Q", StdFbType::Bool), ("CV", StdFbType::Int)],
    ),
    (
        "CTUD",
        &[
            ("CU", StdFbType::Bool),
            ("CD", StdFbType::Bool),
            ("R", StdFbType::Bool),
            ("LD", StdFbType::Bool),
            ("PV", StdFbType::Int),
        ],
        &[
            ("QU", StdFbType::Bool),
            ("QD", StdFbType::Bool),
            ("CV", StdFbType::Int),
        ],
    ),
    (
        "R_TRIG",
        &[("CLK", StdFbType::Bool)],
        &[("Q", StdFbType::Bool)],
    ),
    (
        "F_TRIG",
        &[("CLK", StdFbType::Bool)],
        &[("Q", StdFbType::Bool)],
    ),
    (
        "SR",
        &[("SET1", StdFbType::Bool), ("RESET", StdFbType::Bool)],
        &[("Q", StdFbType::Bool)],
    ),
    (
        "RS",
        &[("SET", StdFbType::Bool), ("RESET1", StdFbType::Bool)],
        &[("Q", StdFbType::Bool)],
    ),
];

const ALL_STD_FNS: &[StdFnKind] = &[
    StdFnKind::Abs,
    StdFnKind::Sqrt,
    StdFnKind::Ln,
    StdFnKind::Log,
    StdFnKind::Exp,
    StdFnKind::Sin,
    StdFnKind::Cos,
    StdFnKind::Tan,
    StdFnKind::Asin,
    StdFnKind::Acos,
    StdFnKind::Atan,
    StdFnKind::Atan2,
    StdFnKind::Expt,
    StdFnKind::Trunc,
    StdFnKind::Add,
    StdFnKind::Sub,
    StdFnKind::Mul,
    StdFnKind::Div,
    StdFnKind::Mod,
    StdFnKind::Move,
    StdFnKind::Shl,
    StdFnKind::Shr,
    StdFnKind::Rol,
    StdFnKind::Ror,
    StdFnKind::And,
    StdFnKind::Or,
    StdFnKind::Xor,
    StdFnKind::Not,
    StdFnKind::Sel,
    StdFnKind::Max,
    StdFnKind::Min,
    StdFnKind::Limit,
    StdFnKind::Mux,
    StdFnKind::Gt,
    StdFnKind::Ge,
    StdFnKind::Eq,
    StdFnKind::Le,
    StdFnKind::Lt,
    StdFnKind::Ne,
    StdFnKind::Len,
    StdFnKind::LenMax,
    StdFnKind::LenCodeUnit,
    StdFnKind::Left,
    StdFnKind::Right,
    StdFnKind::Mid,
    StdFnKind::Concat,
    StdFnKind::Insert,
    StdFnKind::Delete,
    StdFnKind::Replace,
    StdFnKind::Find,
];

fn std_signature(kind: StdFnKind) -> (Vec<StdParam>, StdResult) {
    use GenericKind::{Any, AnyBit, AnyInt, AnyMagnitude, AnyNum, AnyReal, AnyString};
    let param = |name, generic, variadic| StdParam {
        name,
        generic,
        variadic,
        output: false,
    };
    match kind {
        StdFnKind::Abs => (vec![param("IN", AnyNum, false)], StdResult::SameAs(0)),
        StdFnKind::Sqrt
        | StdFnKind::Ln
        | StdFnKind::Log
        | StdFnKind::Exp
        | StdFnKind::Sin
        | StdFnKind::Cos
        | StdFnKind::Tan
        | StdFnKind::Asin
        | StdFnKind::Acos
        | StdFnKind::Atan => (vec![param("IN", AnyReal, false)], StdResult::RealOfArgs),
        StdFnKind::Atan2 | StdFnKind::Expt => (
            vec![param("IN1", AnyReal, false), param("IN2", AnyReal, false)],
            StdResult::RealOfArgs,
        ),
        StdFnKind::Trunc => (
            vec![param("IN", AnyReal, false)],
            StdResult::Concrete(StdRetType::Dint),
        ),
        StdFnKind::Add | StdFnKind::Sub | StdFnKind::Mul => (
            vec![param("IN1", AnyNum, false), param("IN2", AnyNum, true)],
            StdResult::SameAs(0),
        ),
        StdFnKind::Div | StdFnKind::Mod => (
            vec![param("IN1", AnyNum, false), param("IN2", AnyNum, false)],
            StdResult::SameAs(0),
        ),
        StdFnKind::Move => (vec![param("IN", Any, false)], StdResult::SameAs(0)),
        StdFnKind::Shl | StdFnKind::Shr | StdFnKind::Rol | StdFnKind::Ror => (
            vec![param("IN", AnyBit, false), param("N", AnyInt, false)],
            StdResult::SameAs(0),
        ),
        StdFnKind::And | StdFnKind::Or | StdFnKind::Xor => (
            vec![param("IN1", AnyBit, false), param("IN2", AnyBit, true)],
            StdResult::SameAs(0),
        ),
        StdFnKind::Not => (vec![param("IN", AnyBit, false)], StdResult::SameAs(0)),
        StdFnKind::Sel => (
            vec![
                param("G", Any, false),
                param("IN0", Any, false),
                param("IN1", Any, false),
            ],
            StdResult::SameAs(1),
        ),
        StdFnKind::Max | StdFnKind::Min => (
            vec![
                param("IN1", AnyMagnitude, false),
                param("IN2", AnyMagnitude, true),
            ],
            StdResult::SameAs(0),
        ),
        StdFnKind::Limit => (
            vec![
                param("MN", AnyMagnitude, false),
                param("IN", AnyMagnitude, false),
                param("MX", AnyMagnitude, false),
            ],
            StdResult::SameAs(1),
        ),
        StdFnKind::Mux => (
            vec![param("K", AnyInt, false), param("IN", Any, true)],
            StdResult::SameAs(1),
        ),
        StdFnKind::Gt
        | StdFnKind::Ge
        | StdFnKind::Eq
        | StdFnKind::Le
        | StdFnKind::Lt
        | StdFnKind::Ne => (
            vec![param("IN1", Any, false), param("IN2", Any, false)],
            StdResult::Bool,
        ),
        StdFnKind::Len | StdFnKind::LenMax => {
            (vec![param("IN", AnyString, false)], StdResult::Dint)
        }
        StdFnKind::LenCodeUnit => (vec![param("IN", AnyString, false)], StdResult::Dint),
        StdFnKind::Left | StdFnKind::Right => (
            vec![param("IN", AnyString, false), param("L", AnyInt, false)],
            StdResult::StringOf(0),
        ),
        StdFnKind::Mid => (
            vec![
                param("IN", AnyString, false),
                param("L", AnyInt, false),
                param("P", AnyInt, false),
            ],
            StdResult::StringOf(0),
        ),
        StdFnKind::Concat => (
            vec![
                param("IN1", AnyString, false),
                param("IN2", AnyString, true),
            ],
            StdResult::StringOf(0),
        ),
        StdFnKind::Insert => (
            vec![
                param("IN1", AnyString, false),
                param("IN2", AnyString, false),
                param("P", AnyInt, false),
            ],
            StdResult::StringOf(0),
        ),
        StdFnKind::Delete => (
            vec![
                param("IN", AnyString, false),
                param("L", AnyInt, false),
                param("P", AnyInt, false),
            ],
            StdResult::StringOf(0),
        ),
        StdFnKind::Replace => (
            vec![
                param("IN1", AnyString, false),
                param("IN2", AnyString, false),
                param("L", AnyInt, false),
                param("P", AnyInt, false),
            ],
            StdResult::StringOf(0),
        ),
        StdFnKind::Find => (
            vec![
                param("IN1", AnyString, false),
                param("IN2", AnyString, false),
            ],
            StdResult::Dint,
        ),
    }
}

/// Returns `true` when `name` is a standard function or FB name.
#[must_use]
pub fn is_standard_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ALL_STD_FNS.iter().any(|kind| kind.as_str() == upper)
        || STANDARD_FBS.iter().any(|(fb, _, _)| *fb == upper.as_str())
}

/// Returns the concrete type category check used by the prelude.
#[must_use]
pub fn ty_matches_generic(ty: &Ty, generic: GenericKind) -> bool {
    match generic {
        GenericKind::Any => true,
        GenericKind::AnyDerived => matches!(
            ty,
            Ty::Named(_) | Ty::Array(_) | Ty::Ref(_) | Ty::String(_) | Ty::Null
        ),
        GenericKind::AnyElementary => !matches!(ty, Ty::Error | Ty::Generic(_)),
        GenericKind::AnyMagnitude => matches!(
            ty,
            Ty::Int(_) | Ty::Real(_) | Ty::BitString(_) | Ty::Time(_) | Ty::Date(_)
        ),
        GenericKind::AnyNum => matches!(ty, Ty::Int(_) | Ty::Real(_)),
        GenericKind::AnyReal => matches!(ty, Ty::Real(_)),
        GenericKind::AnyInt => matches!(ty, Ty::Int(_)),
        GenericKind::AnyUnsigned => matches!(ty, Ty::Int(int) if !int.signed),
        GenericKind::AnySigned => matches!(ty, Ty::Int(int) if int.signed),
        GenericKind::AnyDuration => matches!(ty, Ty::Time(_)),
        GenericKind::AnyBit => matches!(ty, Ty::BitString(_) | Ty::Bool),
        GenericKind::AnyChars => matches!(ty, Ty::Char(_) | Ty::String(_)),
        GenericKind::AnyString => matches!(ty, Ty::String(_)),
        GenericKind::AnyChar => matches!(ty, Ty::Char(_)),
        GenericKind::AnyDate => matches!(ty, Ty::Date(_)),
    }
}
