//! WL-10 phase 1: test-build statement coverage instrumentation.
//!
//! These tests exercise the code-generation side of the locked WL-10 contract:
//! the `stc_coverage_counters` module global, one increment per instrumented
//! statement, span-keyed site dedupe, and the exclusions for test-only code,
//! unlisted files, synthesized standard FB bodies, and empty statements.

mod common;

use std::collections::BTreeSet;

use stc_codegen::{CoverageKind, CoverageOptions, CoverageSite, Mode, Output};
use stc_hir::{DefId, DefKind, HirCrate, StmtId, StmtKindHir};
use stc_span::{FileId, Span};

/// Production file (`FileId(0)`): instrumented program/function code plus
/// test-only POUs that must never be instrumented.
const PRODUCTION: &str = r#"FUNCTION F : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
END_VAR
  r := 1;
  IF n > 0 THEN
    r := 2;
  ELSE
    ;
  END_IF;
  F := r;
END_FUNCTION

{stc:test}
FUNCTION TestOnly : BOOL
VAR
  t : DINT;
END_VAR
  t := 1;
  TestOnly := TRUE;
END_FUNCTION

{stc:test}
FUNCTION_BLOCK FBTest
VAR
  v : DINT;
END_VAR
METHOD PUBLIC Helper : DINT
  Helper := v + 1;
END_METHOD
  v := v + 1;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  x : DINT;
  t : TON;
END_VAR
  x := 1;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

/// `[test].sources` file (`FileId(1)`); every definition in it is test-only.
const TEST_SOURCE: &str = r#"{stc:test}
FUNCTION TestMain : BOOL
VAR
  x : DINT;
END_VAR
  x := 1;
  x := x + 1;
  TestMain := TRUE;
END_FUNCTION
"#;

/// Production file outside the coverage allow-list (`FileId(2)`).
const OTHER_SOURCE: &str = r#"FUNCTION G : DINT
VAR
  y : DINT;
END_VAR
  y := 1;
  G := y;
END_FUNCTION
"#;

const SOURCES: [&str; 3] = ["src/prod.st", "src/tests.st", "src/other.st"];
const PROD: FileId = FileId::new(0);
const TEST_FILE: FileId = FileId::new(1);
const OTHER_FILE: FileId = FileId::new(2);

fn fixture() -> stc_sema::Analysis {
    let files = common::parse_sources(&[PRODUCTION, TEST_SOURCE, OTHER_SOURCE]);
    common::analyze_test_build(&files, &[1])
}

fn generate(analysis: &stc_sema::Analysis, coverage: Option<&[FileId]>) -> Output {
    let sources: Vec<String> = SOURCES.iter().map(|source| (*source).to_string()).collect();
    let options = common::test_build_options("app", &sources, coverage);
    stc_codegen::generate(&analysis.hir, &options)
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
}

fn generate_fixture(coverage: Option<&[FileId]>) -> (stc_sema::Analysis, Output) {
    let analysis = fixture();
    let output = generate(&analysis, coverage);
    (analysis, output)
}

fn def_in_file(hir: &HirCrate, name: &str, file: FileId) -> DefId {
    hir.defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            (def.span.file == file
                && def
                    .name
                    .is_some_and(|symbol| hir.interner.text(symbol).eq_ignore_ascii_case(name)))
            .then_some(DefId::new(u32::try_from(index).expect("def limit")))
        })
        .unwrap_or_else(|| panic!("no definition `{name}` in {file:?}"))
}

/// Collects statement spans in lowering order, applying the same eligibility
/// filters as the instrumentation point.
fn statement_spans(hir: &HirCrate, stmts: &[StmtId], out: &mut Vec<Span>) {
    for id in stmts {
        let stmt = hir.stmt(*id);
        if !matches!(stmt.kind, StmtKindHir::Empty) && !stmt.span.is_empty() {
            out.push(stmt.span);
        }
        match &stmt.kind {
            StmtKindHir::If { arms, else_body } => {
                for (_, body) in arms {
                    statement_spans(hir, body, out);
                }
                statement_spans(hir, else_body, out);
            }
            StmtKindHir::Case {
                arms, else_body, ..
            } => {
                for arm in arms {
                    statement_spans(hir, &arm.body, out);
                }
                statement_spans(hir, else_body, out);
            }
            StmtKindHir::For { body, .. }
            | StmtKindHir::While { body, .. }
            | StmtKindHir::Repeat { body, .. } => statement_spans(hir, body, out),
            _ => {}
        }
    }
}

fn body_statement_spans(hir: &HirCrate, def: DefId) -> Vec<Span> {
    let body = match &hir.defs[def.index()].kind {
        DefKind::Function {
            body: Some(body), ..
        }
        | DefKind::FunctionBlock {
            body: Some(body), ..
        }
        | DefKind::Program {
            body: Some(body), ..
        }
        | DefKind::Method {
            body: Some(body), ..
        } => *body,
        _ => return Vec::new(),
    };
    let mut spans = Vec::new();
    statement_spans(hir, &hir.bodies[body.index()].stmts, &mut spans);
    spans
}

fn expected_sites(spans: &[Span]) -> Vec<CoverageSite> {
    spans
        .iter()
        .enumerate()
        .map(|(id, span)| CoverageSite {
            id: u32::try_from(id).expect("site limit"),
            span: *span,
            kind: CoverageKind::Statement,
        })
        .collect()
}

fn counter_line(count: usize) -> String {
    format!(
        "@{} = dso_local dllexport global [{count} x i64] zeroinitializer, align 8",
        stc_got::descriptor::COVERAGE_COUNTERS
    )
}

fn increment_count(module: &str, id: u32) -> usize {
    module
        .matches(&format!(
            "getelementptr inbounds i64, ptr @{}, i64 {id}\n",
            stc_got::descriptor::COVERAGE_COUNTERS
        ))
        .count()
}

fn define_lines(module: &str) -> impl Iterator<Item = &str> {
    module.lines().filter(|line| line.starts_with("define "))
}

#[test]
fn instruments_production_statements_in_emission_order() {
    let (analysis, output) = generate_fixture(Some(&[PROD]));
    let hir = &analysis.hir;

    // Functions are emitted in definition order, then program bodies; `F` is
    // the first production function and `Main` the only configured program.
    let mut spans = body_statement_spans(hir, def_in_file(hir, "F", PROD));
    spans.extend(body_statement_spans(hir, def_in_file(hir, "Main", PROD)));
    let expected = expected_sites(&spans);
    assert_eq!(output.coverage_sites, expected);
    assert_eq!(
        output
            .coverage_sites
            .iter()
            .map(|site| site.id)
            .collect::<Vec<_>>(),
        (0..u32::try_from(expected.len()).expect("site limit")).collect::<Vec<_>>(),
        "site IDs are dense and issued in lowering order"
    );
    assert!(output.coverage_sites.len() >= 5);

    // The counter global is exported, zero-initialized, non-constant 8-byte
    // aligned `[N x i64]` module data.
    assert!(
        output
            .module
            .lines()
            .any(|line| line == counter_line(output.coverage_sites.len())),
        "missing coverage counter global:\n{}",
        output.module
    );

    // Exactly one increment per site: the GEP names the site's counter index.
    for site in &output.coverage_sites {
        assert_eq!(
            increment_count(&output.module, site.id),
            1,
            "site {} at {:?} is not incremented exactly once",
            site.id,
            site.span
        );
    }

    // The structural verifier already ran inside `generate`, so reaching here
    // means the direct counter load/store is an allowed local global.
}

#[test]
fn excludes_test_only_definitions_unlisted_files_and_standard_fb_bodies() {
    let (analysis, output) = generate_fixture(Some(&[PROD]));
    let hir = &analysis.hir;

    let mut excluded = Vec::new();
    for (name, file) in [
        ("TestOnly", PROD),
        ("FBTest", PROD),
        ("Helper", PROD),
        ("TestMain", TEST_FILE),
        ("G", OTHER_FILE),
    ] {
        let spans = body_statement_spans(hir, def_in_file(hir, name, file));
        assert!(!spans.is_empty(), "`{name}` has no statements to exclude");
        excluded.extend(spans);
    }

    let site_spans: BTreeSet<Span> = output.coverage_sites.iter().map(|site| site.span).collect();
    for span in &excluded {
        assert!(
            !site_spans.contains(span),
            "excluded statement at {span:?} was instrumented"
        );
    }
    assert!(
        output
            .coverage_sites
            .iter()
            .all(|site| site.span.file == PROD),
        "only the allow-listed production file may contribute sites"
    );

    // The exclusions are meaningful only if the test-only bodies and the
    // synthesized standard FB really are emitted into the test module.
    assert!(
        define_lines(&output.module).any(|line| line.contains("FBTEST")),
        "test-only FB body is not emitted"
    );
    assert!(
        define_lines(&output.module).any(|line| line.contains("_TON")),
        "synthesized TON body is not emitted"
    );
}

#[test]
fn dedupes_sites_by_span() {
    let mut analysis = fixture();
    let hir = &mut analysis.hir;
    let f = def_in_file(hir, "F", PROD);
    let DefKind::Function {
        body: Some(body_id),
        ..
    } = hir.defs[f.index()].kind.clone()
    else {
        panic!("`F` has no body");
    };
    let stmts = hir.bodies[body_id.index()].stmts.clone();
    let first = hir.stmt(stmts[0]).span;
    // Give the second statement (`IF`) the span of the first: both are lowered,
    // but they must share one counter and one site.
    hir.stmts[stmts[1].index()].span = first;
    assert_eq!(hir.stmt(stmts[1]).span, first);

    let output = generate(&analysis, Some(&[PROD]));

    let spans = body_statement_spans(&analysis.hir, f);
    assert_eq!(spans[0], first);
    assert_eq!(spans[1], first);
    let mut unique = Vec::new();
    for span in spans {
        if !unique.contains(&span) {
            unique.push(span);
        }
    }
    unique.extend(body_statement_spans(
        &analysis.hir,
        def_in_file(&analysis.hir, "Main", PROD),
    ));
    assert_eq!(output.coverage_sites, expected_sites(&unique));
    assert_eq!(
        output
            .coverage_sites
            .iter()
            .filter(|site| site.span == first)
            .count(),
        1,
        "the repeated span gets one site"
    );
    assert_eq!(
        increment_count(&output.module, 0),
        2,
        "both lowering passes increment the shared counter"
    );
}

#[test]
fn skips_empty_and_zero_length_statements() {
    let mut analysis = fixture();
    let hir = &mut analysis.hir;
    let f = def_in_file(hir, "F", PROD);
    let DefKind::Function {
        body: Some(body_id),
        ..
    } = hir.defs[f.index()].kind.clone()
    else {
        panic!("`F` has no body");
    };
    let stmts = hir.bodies[body_id.index()].stmts.clone();
    let zeroed = hir.stmt(stmts[0]).span;
    hir.stmts[stmts[0].index()].span = Span::empty(PROD, zeroed.start);

    // The bare `;` in the `ELSE` arm is an `Empty` statement with a real span.
    let empty_span = find_empty_span(&analysis.hir, f);

    let output = generate(&analysis, Some(&[PROD]));
    let spans = body_statement_spans(&analysis.hir, f);
    assert!(
        !spans.contains(&zeroed),
        "the zero-length statement is skipped"
    );
    let mut spans = spans;
    spans.extend(body_statement_spans(
        &analysis.hir,
        def_in_file(&analysis.hir, "Main", PROD),
    ));
    assert_eq!(output.coverage_sites, expected_sites(&spans));
    assert!(
        output
            .coverage_sites
            .iter()
            .all(|site| site.span != empty_span),
        "the Empty statement was instrumented"
    );
    assert!(
        output
            .coverage_sites
            .iter()
            .all(|site| !site.span.is_empty()),
        "a zero-length span was instrumented"
    );
}

fn find_empty_span(hir: &HirCrate, def: DefId) -> Span {
    let DefKind::Function {
        body: Some(body_id),
        ..
    } = &hir.defs[def.index()].kind
    else {
        panic!("`{def:?}` has no body");
    };
    let mut pending: Vec<StmtId> = hir.bodies[body_id.index()].stmts.clone();
    while let Some(id) = pending.pop() {
        let stmt = hir.stmt(id);
        if matches!(stmt.kind, StmtKindHir::Empty) {
            return stmt.span;
        }
        match &stmt.kind {
            StmtKindHir::If { arms, else_body } => {
                for (_, body) in arms {
                    pending.extend(body.iter().copied());
                }
                pending.extend(else_body.iter().copied());
            }
            StmtKindHir::Case {
                arms, else_body, ..
            } => {
                for arm in arms {
                    pending.extend(arm.body.iter().copied());
                }
                pending.extend(else_body.iter().copied());
            }
            StmtKindHir::For { body, .. }
            | StmtKindHir::While { body, .. }
            | StmtKindHir::Repeat { body, .. } => pending.extend(body.iter().copied()),
            _ => {}
        }
    }
    panic!("`F` has no Empty statement");
}

#[test]
fn disabled_coverage_leaves_the_module_unchanged() {
    let analysis = fixture();
    let disabled = generate(&analysis, None);
    assert!(disabled.coverage_sites.is_empty());
    assert!(
        !disabled
            .module
            .contains(stc_got::descriptor::COVERAGE_COUNTERS),
        "a coverage-disabled build defines the counter global"
    );

    // Enabling coverage with an empty allow-list is observationally identical.
    let empty = generate(&analysis, Some(&[]));
    assert!(empty.coverage_sites.is_empty());
    assert_eq!(empty.module, disabled.module);
}

#[test]
fn coverage_is_rejected_outside_a_test_build() {
    let hir =
        common::clean_hir("PROGRAM Main\nVAR\n  x : DINT;\nEND_VAR\n  x := 1;\nEND_PROGRAM\n");
    let sources = vec!["src/main.st".to_string()];
    let mut options = common::options("app", Mode::Executable, &sources);
    options.coverage = Some(CoverageOptions {
        files: [FileId::new(0)].into_iter().collect(),
    });
    assert!(!options.test_build);
    let errors = stc_codegen::generate(&hir, &options).expect_err("coverage must be rejected");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), stc_codegen::codes::CODEGEN_INTERNAL);
    assert!(
        errors[0].message().contains("test build"),
        "unexpected message: {}",
        errors[0].message()
    );
}

#[test]
fn coverage_is_deterministic() {
    let analysis = fixture();
    let first = generate(&analysis, Some(&[PROD]));
    let second = generate(&analysis, Some(&[PROD]));
    assert_eq!(first.module, second.module);
    assert_eq!(first.coverage_sites, second.coverage_sites);
    assert_eq!(
        first.metadata.to_json().render(),
        second.metadata.to_json().render()
    );
}
