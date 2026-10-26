//! Multi-package analysis: merged HIR, package resolution, `INTERNAL`
//! visibility, package aliases, configuration selection, and stdlib control
//! (M6 phase 2, D-153).

use stc_diag::DiagnosticSink;
use stc_hir::{DefId, PackageId};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, PackageSession, ProjectSession, analyze_project};
use stc_span::FileId;

struct Spec<'a> {
    name: &'a str,
    namespace: &'a str,
    dependencies: Vec<usize>,
    sources: Vec<&'a str>,
    is_root: bool,
}

impl<'a> Spec<'a> {
    fn package(name: &'a str, sources: Vec<&'a str>) -> Self {
        Self {
            name,
            namespace: name,
            dependencies: Vec::new(),
            sources,
            is_root: false,
        }
    }

    fn root(name: &'a str, sources: Vec<&'a str>) -> Self {
        Self {
            is_root: true,
            ..Self::package(name, sources)
        }
    }

    fn depends_on(mut self, packages: &[usize]) -> Self {
        self.dependencies = packages.to_vec();
        self
    }

    fn namespace(mut self, namespace: &'a str) -> Self {
        self.namespace = namespace;
        self
    }
}

fn analyze(
    specs: &[Spec<'_>],
    configuration: Option<&str>,
    stdlib: bool,
) -> (Analysis, DiagnosticSink) {
    let mut sink = DiagnosticSink::unlimited();
    let mut parsed: Vec<Vec<ParsedFile>> = Vec::new();
    let mut next_file = 0_u32;
    for spec in specs {
        let mut files = Vec::new();
        for source in &spec.sources {
            let id = FileId::new(next_file);
            next_file += 1;
            let lexed = tokenize_bytes(id, source.as_bytes(), &mut sink);
            let parsed_file = parse(
                id,
                lexed.text.as_str(),
                &lexed.stream,
                ParseOptions::default(),
                &mut sink,
            );
            files.push(parsed_file);
        }
        parsed.push(files);
    }
    let packages = specs
        .iter()
        .zip(&parsed)
        .map(|(spec, files)| PackageSession {
            name: spec.name.to_owned(),
            root_namespace: spec.namespace.to_owned(),
            dependencies: spec.dependencies.clone(),
            files,
            is_root: spec.is_root,
        })
        .collect();
    let session = ProjectSession {
        packages,
        configuration: configuration.map(str::to_owned),
        stdlib,
        test_build: false,
        test_files: &[],
    };
    let analysis = analyze_project(&session, &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    (analysis, sink)
}

fn errors(sink: &DiagnosticSink) -> Vec<&'static str> {
    sink.iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn find_def(hir: &stc_hir::HirCrate, name: &str, package: PackageId) -> DefId {
    hir.defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            (def.package == package
                && def
                    .name
                    .is_some_and(|symbol| hir.interner.text(symbol).eq_ignore_ascii_case(name)))
            .then_some(DefId::new(u32::try_from(index).unwrap()))
        })
        .unwrap_or_else(|| panic!("definition `{name}` in package {}", package.raw()))
}

const LIB: &str = "\
TYPE T : DINT; END_TYPE
VAR_GLOBAL G : DINT; END_VAR
FUNCTION Double : DINT
VAR_INPUT x : DINT; END_VAR
  Double := x * 2;
END_FUNCTION
FUNCTION_BLOCK Counter
VAR_INPUT delta : DINT; END_VAR
VAR_OUTPUT count : DINT; END_VAR
  count := count + delta;
END_FUNCTION_BLOCK
";

#[test]
fn cross_package_functions_fbs_types_and_globals_resolve() {
    let app = "\
PROGRAM Main
VAR
  c : lib.Counter;
  t : lib.T;
  g : DINT;
END_VAR
  lib.G := lib.Double(t);
  c(delta := 1);
  g := lib.G;
END_PROGRAM
";
    let (analysis, sink) = analyze(
        &[
            Spec::package("lib", vec![LIB]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    let lib = PackageId::new(0);
    assert_eq!(analysis.hir.root_package, PackageId::new(1));
    assert_eq!(analysis.hir.packages.len(), 2);
    assert_eq!(
        analysis.hir.defs[find_def(&analysis.hir, "Double", lib).index()].package,
        lib
    );
    assert_eq!(
        analysis.hir.defs[find_def(&analysis.hir, "Counter", lib).index()].package,
        lib
    );
    assert_eq!(
        analysis.hir.defs[find_def(&analysis.hir, "T", lib).index()].package,
        lib
    );
}

#[test]
fn dependency_alias_is_the_namespace_and_name_is_the_identity() {
    let app = "\
FUNCTION Use : DINT
  Use := drive.Helper(3);
END_FUNCTION
";
    let lib = "\
FUNCTION Helper : DINT
VAR_INPUT x : DINT; END_VAR
  Helper := x + 1;
END_FUNCTION
";
    let (analysis, sink) = analyze(
        &[
            Spec::package("motor-lib", vec![lib]).namespace("drive"),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    assert_eq!(analysis.hir.packages[0].name, "motor-lib");
    assert_eq!(analysis.hir.packages[0].root_namespace, "drive");
    let helper = find_def(&analysis.hir, "Helper", PackageId::new(0));
    assert_eq!(analysis.hir.defs[helper.index()].package, PackageId::new(0));
}

#[test]
fn plain_package_name_is_not_visible_when_an_alias_is_used() {
    let app = "\
FUNCTION Use : DINT
  Use := motorlib.Helper(3);
END_FUNCTION
";
    let lib = "\
FUNCTION Helper : DINT
  Helper := 1;
END_FUNCTION
";
    let (_, sink) = analyze(
        &[
            Spec::package("motorlib", vec![lib]).namespace("drive"),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_UNKNOWN),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn using_imports_dependency_namespaces() {
    let app = "\
PROGRAM Main
USING lib;
VAR x : DINT; END_VAR
  x := Double(2);
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![LIB]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
}

#[test]
fn using_two_packages_with_the_same_name_is_ambiguous() {
    let first = "\
FUNCTION F : DINT
  F := 1;
END_FUNCTION
";
    let second = "\
FUNCTION F : DINT
  F := 2;
END_FUNCTION
";
    let app = "\
PROGRAM Main
USING a, b;
VAR x : DINT; END_VAR
  x := F();
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("a", vec![first]),
            Spec::package("b", vec![second]),
            Spec::root("app", vec![app]).depends_on(&[0, 1]),
        ],
        None,
        true,
    );
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_AMBIGUOUS),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn internal_globals_are_package_private() {
    let lib = "\
VAR_GLOBAL INTERNAL Secret : DINT; END_VAR
";
    let app = "\
PROGRAM Main
VAR x : DINT; END_VAR
  lib.Secret := 1;
  x := lib.Secret;
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_INACCESSIBLE),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn internal_namespaces_are_package_private() {
    let lib = "\
NAMESPACE INTERNAL Hidden
FUNCTION F : DINT
  F := 1;
END_FUNCTION
END_NAMESPACE
";
    let app = "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := lib.Hidden.F();
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_INACCESSIBLE),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn internal_visibility_is_allowed_inside_the_package() {
    let lib = "\
VAR_GLOBAL INTERNAL Secret : DINT; END_VAR
FUNCTION Read : DINT
  Read := Secret;
END_FUNCTION
";
    let app = "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := lib.Secret + lib.Read();
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    let errors = errors(&sink);
    // `lib.Secret` is rejected; `lib.Read` (which uses `Secret` internally) is
    // not.
    assert_eq!(
        errors
            .iter()
            .filter(|code| **code == stc_sema::codes::NAME_INACCESSIBLE)
            .count(),
        1,
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn duplicate_top_level_names_across_packages_are_allowed() {
    let shared = "\
FUNCTION F : DINT
  F := 1;
END_FUNCTION
TYPE T : DINT; END_TYPE
";
    let app = "\
PROGRAM Main
VAR x : DINT; y : a.T; z : b.T; END_VAR
  x := a.F() + b.F();
  y := 1;
  z := 2;
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("a", vec![shared]),
            Spec::package("b", vec![shared]),
            Spec::root("app", vec![app]).depends_on(&[0, 1]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
}

#[test]
fn duplicate_top_level_names_in_one_package_are_errors() {
    let lib = "\
FUNCTION F : DINT
  F := 1;
END_FUNCTION
FUNCTION F : DINT
  F := 2;
END_FUNCTION
";
    let (_, sink) = analyze(&[Spec::root("app", vec![lib])], None, true);
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_DUPLICATE),
        "{:#?}",
        sink.diagnostics()
    );
}

const MAIN_PROGRAM: &str = "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
";

#[test]
fn only_root_package_configurations_select_the_build() {
    let root_config = "\
CONFIGURATION RootMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main;
END_RESOURCE
END_CONFIGURATION
";
    let dependency_config = "\
PROGRAM Dep
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION DepMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Dep;
END_RESOURCE
END_CONFIGURATION
";
    let (analysis, sink) = analyze(
        &[
            Spec::package("lib", vec![dependency_config]),
            Spec::root("app", vec![MAIN_PROGRAM, root_config]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    let entry = analysis
        .hir
        .entry_configuration
        .expect("root configuration");
    assert_eq!(analysis.hir.defs[entry.index()].package, PackageId::new(1));
    assert!(
        analysis
            .hir
            .def_name(entry)
            .eq_ignore_ascii_case("RootMain")
    );
}

#[test]
fn dependency_configuration_alone_is_not_a_build_entry() {
    let lib = "\
PROGRAM Dep
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION DepMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Dep;
END_RESOURCE
END_CONFIGURATION
";
    let (analysis, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![MAIN_PROGRAM]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    assert!(analysis.hir.entry_configuration.is_none());
}

#[test]
fn explicit_configuration_selection_uses_root_configurations() {
    let root_configs = "\
CONFIGURATION RootMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main;
END_RESOURCE
END_CONFIGURATION
CONFIGURATION Alternate
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Main;
END_RESOURCE
END_CONFIGURATION
";
    let (analysis, sink) = analyze(
        &[Spec::root("app", vec![MAIN_PROGRAM, root_configs])],
        Some("Alternate"),
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    let entry = analysis
        .hir
        .entry_configuration
        .expect("selected configuration");
    assert!(
        analysis
            .hir
            .def_name(entry)
            .eq_ignore_ascii_case("Alternate")
    );
}

#[test]
fn selected_configuration_must_belong_to_the_root_package() {
    let lib = "\
PROGRAM Dep
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
CONFIGURATION DepMain
RESOURCE R ON CPU
  TASK t (INTERVAL := T#10ms);
  PROGRAM inst WITH t : Dep;
END_RESOURCE
END_CONFIGURATION
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![MAIN_PROGRAM]).depends_on(&[0]),
        ],
        Some("DepMain"),
        true,
    );
    assert!(
        errors(&sink).contains(&stc_sema::codes::PROJECT_CONFIGURATION),
        "{:#?}",
        sink.diagnostics()
    );
}

#[test]
fn stdlib_false_suppresses_standard_fbs_but_keeps_functions() {
    let source = "\
PROGRAM Main
VAR
  t : TON;
  x : DINT;
END_VAR
  x := ABS(-3);
END_PROGRAM
";
    let (_, sink) = analyze(&[Spec::root("app", vec![source])], None, false);
    assert!(
        errors(&sink).contains(&stc_sema::codes::NAME_UNKNOWN),
        "TON must be unknown with stdlib = false: {:#?}",
        sink.diagnostics()
    );
    assert!(
        !sink
            .iter()
            .any(|diagnostic| diagnostic.message.contains("ABS")),
        "standard functions remain available: {:#?}",
        sink.diagnostics()
    );

    let user_fb = "\
FUNCTION_BLOCK TON
VAR_INPUT IN : BOOL; END_VAR
VAR_OUTPUT Q : BOOL; END_VAR
  Q := IN;
END_FUNCTION_BLOCK
PROGRAM Main
VAR t : TON; END_VAR
  t(IN := TRUE);
END_PROGRAM
";
    let (_, sink) = analyze(&[Spec::root("app", vec![user_fb])], None, false);
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
}

#[test]
fn stdlib_true_synthesizes_standard_fbs() {
    let source = "\
PROGRAM Main
VAR t : TON; END_VAR
  t(IN := TRUE, PT := T#1s);
END_PROGRAM
";
    let (analysis, sink) = analyze(&[Spec::root("app", vec![source])], None, true);
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
    assert!(analysis.hir.defs.iter().any(|def| {
        def.name_text(&analysis.hir.interner)
            .eq_ignore_ascii_case("TON")
    }));
}

#[test]
fn deterministic_package_and_file_order() {
    let app = "\
PROGRAM Main
VAR x : DINT; END_VAR
  x := lib.F() + lib.G;
END_PROGRAM
";
    let lib = "\
FUNCTION F : DINT
  F := 1;
END_FUNCTION
VAR_GLOBAL G : DINT; END_VAR
";
    let analyze_once = || {
        let (analysis, sink) = analyze(
            &[
                Spec::package("lib", vec![lib]),
                Spec::root("app", vec![app]).depends_on(&[0]),
            ],
            None,
            true,
        );
        assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
        stc_hir::dump_hir(&analysis.hir)
    };
    assert_eq!(analyze_once(), analyze_once());
}

#[test]
fn a_separate_test_project_depends_on_the_code_under_test() {
    let code = "\
FUNCTION Clamp : DINT
VAR_INPUT x : DINT; END_VAR
  IF x < 0 THEN
    Clamp := 0;
  ELSE
    Clamp := x;
  END_IF;
END_FUNCTION
";
    let tests = "\
FUNCTION ClampAcceptsPositive : BOOL
  ClampAcceptsPositive := code.Clamp(4) = 4;
END_FUNCTION
";
    let (_, sink) = analyze(
        &[
            Spec::package("code", vec![code]).namespace("code"),
            Spec::root("tests", vec![tests]).depends_on(&[0]),
        ],
        None,
        true,
    );
    assert!(errors(&sink).is_empty(), "{:#?}", sink.diagnostics());
}

#[test]
fn internal_methods_are_package_private() {
    let lib = "\
FUNCTION_BLOCK B
METHOD INTERNAL Hidden : DINT
  Hidden := 1;
END_METHOD
METHOD PUBLIC Call : DINT
  Call := Hidden();
END_METHOD
END_FUNCTION_BLOCK
";
    let app = "\
PROGRAM Main
VAR
  b : lib.B;
  y : DINT;
END_VAR
  y := b.Hidden();
  y := b.Call();
END_PROGRAM
";
    let (_, sink) = analyze(
        &[
            Spec::package("lib", vec![lib]),
            Spec::root("app", vec![app]).depends_on(&[0]),
        ],
        None,
        true,
    );
    let errors = errors(&sink);
    assert_eq!(
        errors
            .iter()
            .filter(|code| **code == stc_sema::codes::NAME_INACCESSIBLE)
            .count(),
        1,
        "{:#?}",
        sink.diagnostics()
    );
}
