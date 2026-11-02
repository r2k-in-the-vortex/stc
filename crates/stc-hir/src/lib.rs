//! Resolved HIR, stable IDs, the session interner, and canonical types.
//!
//! This crate does not depend on `stc-syntax` or `stc-parser`: HIR owns all
//! spans and never references CST nodes (AST §5, Architecture §2). Codegen
//! consumes only this crate.

#![forbid(unsafe_code)]

mod core_dump;
mod dump;
mod hir;
mod id;
mod interner;
mod ty;

pub use core_dump::{
    CORE_DUMP_FORMAT, CORE_DUMP_FORMAT_VERSION, CORE_DUMP_PROFILE, CORE_DUMP_SEMANTICS_VERSION,
    CoreDumpSummary, dump_core_hir, verify_core_dump,
};
pub use dump::dump_hir;
pub use hir::{
    ArgDirection, ArgHir, BinOp, BindingHir, Body, CalleeHir, CaseArmHir, CaseLabelHir, CastKind,
    ConfigInitHir, ConfigurationHir, ConstValue, Def, DefKind, DirectAddress, DirectArea,
    DirectSize, EnumValueHir, ExprHir, ExprKindHir, FieldHir, HirCrate, IoDirection, IoHint,
    LiteralHir, LocalHir, PackageInfo, ParamHir, PlaceHir, ProgramInstanceHir, ResourceHir,
    RetainPolicy, SizeOfHir, StdFnKind, StdFunction, StdParam, StdResult, StdRetType, StmtHir,
    StmtKindHir, TaskHir, UnOp, VarAccessHir, VarSectionKind, Visibility,
};
pub use id::{BodyId, DefId, ExprId, FieldId, LocalId, PackageId, ParamId, StdFnId, StmtId, TyId};
pub use interner::{Interner, Symbol};
pub use ty::{
    ArrayBoundTy, ArrayTy, CharTy, DateTy, GenericKind, IntTy, PtrTy, RealTy, RefTy, StringKind,
    StringTy, TimeTy, Ty, TyArena,
};
