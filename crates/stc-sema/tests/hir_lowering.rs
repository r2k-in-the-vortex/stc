mod common;

use common::assert_clean;
use stc_hir::{CalleeHir, CastKind, DefKind, ExprKindHir, PlaceHir, StmtKindHir, TyId};

fn program_fields(analysis: &stc_sema::Analysis, program_name: &str) -> Vec<(String, TyId)> {
    let def = analysis
        .hir
        .defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            (matches!(&def.kind, DefKind::Program { .. })
                && def
                    .name_text(&analysis.hir.interner)
                    .eq_ignore_ascii_case(program_name))
            .then(|| stc_hir::DefId::new(index as u32))
        })
        .expect("program definition");
    analysis
        .hir
        .def(def)
        .field_ids()
        .iter()
        .map(|field| {
            let field = analysis.hir.field(*field);
            (analysis.hir.interner.text(field.name).to_string(), field.ty)
        })
        .collect()
}

fn program_body(analysis: &stc_sema::Analysis, program_name: &str) -> Vec<stc_hir::StmtId> {
    let def = analysis
        .hir
        .defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            (matches!(&def.kind, DefKind::Program { .. })
                && def
                    .name_text(&analysis.hir.interner)
                    .eq_ignore_ascii_case(program_name))
            .then(|| stc_hir::DefId::new(index as u32))
        })
        .expect("program definition");
    match &analysis.hir.def(def).kind {
        DefKind::Program {
            body: Some(body), ..
        } => analysis.hir.bodies[body.index()].stmts.clone(),
        _ => panic!("program body"),
    }
}

#[test]
fn aliases_and_arrays_canonicalize() {
    let analysis = assert_clean(
        "TYPE
  Alias : DINT;
  S1 : INT(0..10);
  S2 : INT(0..10);
END_TYPE
PROGRAM P
VAR
  x : Alias;
  y : DINT;
  s1 : S1;
  s2 : S2;
  a : ARRAY[1..2] OF DINT;
  b : ARRAY[1..2] OF DINT;
END_VAR
  x := y;
  s1 := s2;
  a[1] := b[1];
END_PROGRAM
",
    );
    let fields = program_fields(&analysis, "P");
    let ty = |name: &str| {
        fields
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, ty)| *ty)
            .unwrap_or_else(|| panic!("field {name}"))
    };
    assert_eq!(ty("x"), ty("y"), "aliases are transparent");
    assert_eq!(ty("a"), ty("b"), "equal array types are canonical");
    assert_ne!(ty("s1"), ty("s2"), "named subranges are nominal");
    assert_ne!(ty("s1"), ty("y"), "subranges are distinct from their base");
}

#[test]
fn implicit_conversions_become_explicit_hir_casts() {
    let analysis = assert_clean(
        "PROGRAM P
VAR
  x : DINT;
  r : REAL;
END_VAR
  r := x;
END_PROGRAM
",
    );
    let stmts = program_body(&analysis, "P");
    let some = stmts
        .iter()
        .find_map(|stmt| match &analysis.hir.stmt(*stmt).kind {
            StmtKindHir::Assign { value, .. } => Some(*value),
            _ => None,
        })
        .expect("assignment");
    let ExprKindHir::Cast { to, kind, .. } = analysis.hir.expr(some).kind else {
        panic!("expected a cast, got {:?}", analysis.hir.expr(some).kind);
    };
    assert_eq!(kind, CastKind::Implicit);
    assert_eq!(
        analysis.hir.type_name(to),
        "REAL",
        "cast targets the destination type"
    );
}

#[test]
fn calls_and_places_lower_with_resolved_targets() {
    let analysis = assert_clean(
        "FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION
PROGRAM P
VAR
  x : DINT;
END_VAR
  x := Add(1, 2);
END_PROGRAM
",
    );
    let stmts = program_body(&analysis, "P");
    let assign = stmts
        .iter()
        .find(|stmt| matches!(analysis.hir.stmt(**stmt).kind, StmtKindHir::Assign { .. }))
        .copied()
        .expect("assignment");
    let StmtKindHir::Assign { place, value } = &analysis.hir.stmt(assign).kind else {
        unreachable!();
    };
    assert!(matches!(
        analysis.hir.expr(*place).kind,
        ExprKindHir::Place(PlaceHir::Field { .. })
    ));
    let ExprKindHir::Call { callee, args } = &analysis.hir.expr(*value).kind else {
        panic!("expected a call");
    };
    assert!(matches!(callee, CalleeHir::Function(_)));
    assert_eq!(args.len(), 2);
}
