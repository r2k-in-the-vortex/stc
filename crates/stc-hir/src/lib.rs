//! Resolved HIR, stable IDs, the session interner, and canonical types.
//!
//! This crate does not depend on `stc-syntax` or `stc-parser`: HIR owns all
//! spans and never references CST nodes (AST §5, Architecture §2). Codegen
//! consumes only this crate.

#![forbid(unsafe_code)]

mod dump;
mod hir;
mod id;
mod interner;
mod ty;

pub use dump::dump_hir;
pub use hir::{
    ArgDirection, ArgHir, BinOp, BindingHir, Body, CalleeHir, CaseArmHir, CaseLabelHir, CastKind,
    ConfigInitHir, ConfigurationHir, ConstValue, Def, DefKind, DirectAddress, DirectArea,
    DirectSize, EnumValueHir, ExprHir, ExprKindHir, FieldHir, HirCrate, LiteralHir, LocalHir,
    ParamHir, PlaceHir, ProgramInstanceHir, ResourceHir, RetainPolicy, StdFnKind, StdFunction,
    StdParam, StdResult, StdRetType, StmtHir, StmtKindHir, TaskHir, UnOp, VarAccessHir,
    VarSectionKind, Visibility,
};
pub use id::{BodyId, DefId, ExprId, FieldId, LocalId, PackageId, ParamId, StdFnId, StmtId, TyId};
pub use interner::{Interner, Symbol};
pub use ty::{
    ArrayBoundTy, ArrayTy, CharTy, DateTy, GenericKind, IntTy, RealTy, RefTy, StringKind, StringTy,
    TimeTy, Ty, TyArena,
};
