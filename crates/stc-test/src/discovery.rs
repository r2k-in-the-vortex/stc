//! `{stc:test}` discovery and manifest overrides (Testing and Verification §2.1).
//!
//! Discovery is syntax-level: it finds POUs marked `{stc:test}` in the current
//! package's production and test-only sources, validates the test signature,
//! merges the `{stc:test}` payload with `[[test.case]]`, and reports
//! `STC-E-TEST-002/003` and `STC-E-PROJECT-008`. It does not analyze names or
//! types; the driver runs the merged semantic analysis afterwards.

use stc_diag::Diagnostic;
use stc_manifest::{DurationNs, TestSettings};
use stc_parser::ParsedFile;
use stc_span::Span;
use stc_syntax::SyntaxKind;
use stc_syntax::ast::{
    AstNode, ConfigDecl, DataTypeDecl, FunctionBlockDecl, FunctionDecl, Item, NamespaceDecl,
    ProgramDecl, TypeDecl, VarDeclLike, VarSection,
};

/// Reserved synthetic configuration name.
pub const TEST_CONFIGURATION: &str = "STC_TEST";
/// Reserved synthetic resource name.
pub const TEST_RESOURCE: &str = "STC_TEST_RES";
/// Reserved synthetic task name.
pub const TEST_TASK: &str = "STC_TEST_TASK";
/// Reserved synthetic processor name.
pub const TEST_CPU: &str = "STC_TEST_CPU";
/// Reserved synthetic wrapper program name.
pub const TEST_WRAPPER: &str = "STC_TEST_WRAPPER";
/// Reserved synthetic program instance name.
pub const TEST_INSTANCE: &str = "STC_TEST_INSTANCE";
/// Reserved wrapper field holding the FB test's `PASSED` value.
pub const TEST_PASSED: &str = "STC_TEST_PASSED";
/// Reserved wrapper field holding the FB test instance.
pub const TEST_FB: &str = "STC_TEST_FB";
/// Reserved function-test result field.
pub const TEST_VALUE: &str = "STC_TEST_VALUE";
/// Reserved function-test one-shot guard.
pub const TEST_DONE: &str = "STC_TEST_DONE";

const RESERVED: &[&str] = &[
    TEST_CONFIGURATION,
    TEST_RESOURCE,
    TEST_TASK,
    TEST_CPU,
    TEST_WRAPPER,
    TEST_INSTANCE,
    TEST_PASSED,
    TEST_FB,
    TEST_VALUE,
    TEST_DONE,
];

const TRAPS: &[&str] = &[
    "OVERFLOW",
    "DIV_ZERO",
    "RANGE",
    "BOUNDS",
    "NULL_REF",
    "INVALID_UTF8",
    "CONVERSION",
    "WATCHDOG",
    "ASSERT",
    "UNIMPLEMENTED",
];

/// One source file considered for discovery.
pub struct SourceFileRef<'a> {
    /// Package-relative path with forward slashes.
    pub relative: &'a str,
    /// Parsed file; its tree carries the `FileId` used by diagnostics.
    pub parsed: &'a ParsedFile,
}

/// Test POU kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestKind {
    /// `FUNCTION` returning `BOOL` with no inputs.
    Function,
    /// `FUNCTION_BLOCK` with an optional `PASSED : BOOL` output.
    FunctionBlock,
    /// `PROGRAM`.
    Program,
}

impl TestKind {
    /// Returns the lowercase kind for reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::FunctionBlock => "function_block",
            Self::Program => "program",
        }
    }
}

/// One discovered test with its effective settings.
#[derive(Clone, Debug)]
pub struct TestCase {
    /// POU name as written.
    pub name: String,
    /// Namespace-qualified name used by the generated wrapper.
    pub qualified: String,
    /// POU kind.
    pub kind: TestKind,
    /// Package-relative source path.
    pub file: String,
    /// Declaration span (used for wrapper failures and reports).
    pub span: Span,
    /// Whether the FB test declares `PASSED : BOOL`.
    pub has_passed: bool,
    /// Effective scan count (function tests always run once).
    pub cycles: u32,
    /// Effective timeout in nanoseconds.
    pub timeout_ns: u64,
    /// Normalized `STC_TRAP_*` expectation.
    pub expect_trap: Option<String>,
    /// The pragma set `cycles` (used for manifest-conflict detection).
    pub(crate) pragma_cycles: bool,
    /// The pragma set `timeout`.
    pub(crate) pragma_timeout: bool,
    /// Span of the test pragma.
    pub(crate) pragma_span: Span,
}

/// Discovery result.
#[derive(Debug)]
pub struct Discovery {
    /// Tests sorted by normalized name (deterministic order).
    pub tests: Vec<TestCase>,
    /// Discovery errors (`STC-E-TEST-*`, `STC-E-PROJECT-008`).
    pub diagnostics: Vec<Diagnostic>,
}

impl Discovery {
    /// Returns `true` when discovery produced errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// Discovers tests in the given files and merges `[test]` settings.
#[must_use]
pub fn discover(
    files: &[SourceFileRef<'_>],
    settings: &TestSettings,
    cli_timeout_ns: Option<u64>,
) -> Discovery {
    let mut collector = Collector::default();
    for file in files {
        collector.relative = Some(file.relative);
        let root = file.parsed.tree().root();
        if let Some(source) = stc_syntax::ast::SourceFile::cast(&root) {
            collector.walk(&source.items(), &mut Vec::new());
        }
    }

    // Duplicate test names are unique per package, case-insensitively.
    let mut seen: std::collections::BTreeMap<String, Span> = std::collections::BTreeMap::new();
    for test in &collector.tests {
        let key = test.name.to_ascii_uppercase();
        if let Some(previous) = seen.get(&key) {
            collector.diagnostics.push(
                Diagnostic::error(
                    "STC-E-TEST-002",
                    format!("duplicate test name `{}` in this package", test.name),
                    test.span,
                )
                .with_label(*previous, "previous test is here"),
            );
        } else {
            seen.insert(key, test.span);
        }
    }

    apply_manifest_cases(&mut collector.tests, settings, &mut collector.diagnostics);
    for test in &mut collector.tests {
        test.timeout_ns = cli_timeout_ns.unwrap_or(test.timeout_ns);
        if test.kind == TestKind::Function {
            test.cycles = 1;
        }
    }
    collector.tests.sort_by(|left, right| {
        (
            left.name.to_ascii_uppercase(),
            left.file.clone(),
            left.span.start,
        )
            .cmp(&(
                right.name.to_ascii_uppercase(),
                right.file.clone(),
                right.span.start,
            ))
    });
    Discovery {
        tests: collector.tests,
        diagnostics: collector.diagnostics,
    }
}

/// Returns the tests matching any of the case-insensitive substring filters.
#[must_use]
pub fn filter_tests(tests: &[TestCase], filters: &[String]) -> Vec<TestCase> {
    if filters.is_empty() {
        return tests.to_vec();
    }
    tests
        .iter()
        .filter(|test| {
            let name = test.name.to_ascii_uppercase();
            filters
                .iter()
                .any(|filter| name.contains(&filter.to_ascii_uppercase()))
        })
        .cloned()
        .collect()
}

#[derive(Default)]
struct Collector<'a> {
    relative: Option<&'a str>,
    tests: Vec<TestCase>,
    diagnostics: Vec<Diagnostic>,
}

struct TestPragma {
    cycles: Option<u32>,
    timeout_ns: Option<u64>,
    expect_trap: Option<String>,
    span: Span,
}

impl Default for TestPragma {
    fn default() -> Self {
        Self {
            cycles: None,
            timeout_ns: None,
            expect_trap: None,
            span: Span::new(stc_span::FileId::new(0), 0, 0),
        }
    }
}
impl Collector<'_> {
    fn walk(&mut self, items: &[Item], namespace: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Namespace(decl) => self.walk_namespace(decl, namespace),
                Item::Function(decl) => self.function(decl, namespace),
                Item::FunctionBlock(decl) => self.function_block(decl, namespace),
                Item::Program(decl) => self.program(decl, namespace),
                Item::Class(decl) => {
                    self.reserved(decl.name(), decl.token_span());
                    self.reject_test_pragmas(decl.pragmas());
                    for method in decl.methods() {
                        self.reject_test_pragmas(method.pragmas());
                    }
                }
                Item::Interface(decl) => {
                    self.reserved(decl.name(), decl.token_span());
                    self.reject_test_pragmas(decl.pragmas());
                }
                Item::DataType(decl) => self.data_type(decl),
                Item::Configuration(decl) => self.configuration(decl),
                Item::GlobalVars(section) => {
                    for declaration in section.declarations() {
                        let names = match declaration {
                            VarDeclLike::Var(variable) => variable.names(),
                            VarDeclLike::Edge(edge) => edge.names(),
                        };
                        for name in names {
                            self.reserved(Some(name), section.token_span());
                        }
                    }
                }
                Item::Using(_) | Item::Error(_) => {}
            }
        }
    }

    fn walk_namespace(&mut self, decl: &NamespaceDecl, namespace: &mut Vec<String>) {
        let Some(path) = decl.name() else {
            return;
        };
        let pushed = path.segments().len();
        for segment in path.segments() {
            namespace.push(segment.text().to_owned());
        }
        let elements = decl.elements();
        self.walk(&elements, namespace);
        namespace.truncate(namespace.len().saturating_sub(pushed));
    }

    fn reject_test_pragmas(&mut self, pragmas: Vec<stc_syntax::ast::Pragma>) {
        for pragma in pragmas {
            if pragma
                .annotation()
                .is_some_and(|annotation| annotation.name == "test")
            {
                self.diagnostics.push(Diagnostic::error(
                    "STC-E-TEST-003",
                    "`{stc:test}` is only valid on FUNCTION, FUNCTION_BLOCK, or PROGRAM",
                    pragma.token_span(),
                ));
            }
        }
    }

    fn data_type(&mut self, decl: &DataTypeDecl) {
        for type_decl in decl.types() {
            let (name, span) = match &type_decl {
                TypeDecl::Simple(node) => (node.name(), node.token_span()),
                TypeDecl::Subrange(node) => (node.name(), node.token_span()),
                TypeDecl::Enum(node) => (node.name(), node.token_span()),
                TypeDecl::NamedValue(node) => (node.name(), node.token_span()),
                TypeDecl::Array(node) => (node.name(), node.token_span()),
                TypeDecl::Struct(node) => (node.name(), node.token_span()),
                TypeDecl::String(node) => (node.name(), node.token_span()),
                TypeDecl::Ref(node) => (node.name(), node.token_span()),
                TypeDecl::Error(node) => (None, node.token_span()),
            };
            self.reserved(name, span);
        }
    }

    fn configuration(&mut self, decl: &ConfigDecl) {
        self.reserved(decl.name(), decl.token_span());
    }

    fn reserved(&mut self, name: Option<stc_syntax::ast::Name>, span: Span) {
        let Some(name) = name else {
            return;
        };
        let upper = name.normalized();
        if RESERVED.contains(&upper.as_str()) {
            self.diagnostics.push(Diagnostic::error(
                "STC-E-TEST-003",
                format!(
                    "`{}` is reserved for the generated test configuration",
                    name.text()
                ),
                span,
            ));
        }
    }

    fn function(&mut self, decl: &FunctionDecl, namespace: &[String]) {
        let Some(name) = decl.name() else {
            return;
        };
        self.reserved(Some(name.clone()), decl.token_span());
        let Some(pragma) = self.test_pragma(decl.pragmas()) else {
            return;
        };
        let mut errors = Vec::new();
        let returns_bool = decl
            .result_type()
            .is_some_and(|type_ref| type_ref.text().trim().eq_ignore_ascii_case("BOOL"));
        if !returns_bool {
            errors.push("a test FUNCTION must return BOOL".to_string());
        }
        for section in decl.var_sections() {
            if is_input_section(&section) && !section.declarations().is_empty() {
                errors.push("a test FUNCTION must not declare inputs".to_string());
            }
        }
        if let Some(message) = errors.first() {
            self.invalid(name.text(), decl.token_span(), message);
            return;
        }
        self.push_test(
            name.text(),
            namespace,
            TestKind::Function,
            false,
            decl.token_span(),
            pragma,
        );
    }

    fn function_block(&mut self, decl: &FunctionBlockDecl, namespace: &[String]) {
        let Some(name) = decl.name() else {
            return;
        };
        self.reserved(Some(name.clone()), decl.token_span());
        self.reject_method_test_pragmas(decl);
        let Some(pragma) = self.test_pragma(decl.pragmas()) else {
            return;
        };
        let mut passed = None;
        let mut invalid = None;
        for section in decl.var_sections() {
            let output = section
                .kind()
                .is_some_and(|token| token.kind() == SyntaxKind::KwVarOutput);
            for declaration in section.declarations() {
                let VarDeclLike::Var(variable) = declaration else {
                    continue;
                };
                for variable_name in variable.names() {
                    if !variable_name.text().eq_ignore_ascii_case("PASSED") {
                        continue;
                    }
                    if !output {
                        invalid = Some("`PASSED` must be declared in a `VAR_OUTPUT` section");
                        continue;
                    }
                    let bool_output = variable.type_ref().is_some_and(|type_ref| {
                        type_ref.text().trim().eq_ignore_ascii_case("BOOL")
                    });
                    if !bool_output {
                        invalid = Some("`PASSED` must be declared as `BOOL`");
                    } else if passed.is_some() {
                        invalid = Some("a test FUNCTION_BLOCK declares `PASSED` more than once");
                    } else {
                        passed = Some(());
                    }
                }
            }
        }
        if let Some(message) = invalid {
            self.invalid(name.text(), decl.token_span(), message);
            return;
        }
        self.push_test(
            name.text(),
            namespace,
            TestKind::FunctionBlock,
            passed.is_some(),
            decl.token_span(),
            pragma,
        );
    }

    fn reject_method_test_pragmas(&mut self, decl: &FunctionBlockDecl) {
        for method in decl.methods() {
            self.reject_test_pragmas(method.pragmas());
        }
    }

    fn program(&mut self, decl: &ProgramDecl, namespace: &[String]) {
        let Some(name) = decl.name() else {
            return;
        };
        self.reserved(Some(name.clone()), decl.token_span());
        let Some(pragma) = self.test_pragma(decl.pragmas()) else {
            return;
        };
        self.push_test(
            name.text(),
            namespace,
            TestKind::Program,
            false,
            decl.token_span(),
            pragma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_test(
        &mut self,
        name: &str,
        namespace: &[String],
        kind: TestKind,
        has_passed: bool,
        span: Span,
        pragma: TestPragma,
    ) {
        let Some(file) = self.relative else {
            return;
        };
        let mut qualified = namespace.join(".");
        if !qualified.is_empty() {
            qualified.push('.');
        }
        qualified.push_str(name);
        let cycles = pragma.cycles.filter(|cycles| *cycles > 0);
        self.tests.push(TestCase {
            name: name.to_owned(),
            qualified,
            kind,
            file: file.to_owned(),
            span,
            has_passed,
            // FB/program defaults are applied when manifest cases are merged.
            cycles: cycles.unwrap_or(0),
            timeout_ns: pragma.timeout_ns.unwrap_or(0),
            expect_trap: pragma.expect_trap,
            pragma_cycles: pragma.cycles.is_some(),
            pragma_timeout: pragma.timeout_ns.is_some(),
            pragma_span: pragma.span,
        });
    }

    fn test_pragma(&mut self, pragmas: Vec<stc_syntax::ast::Pragma>) -> Option<TestPragma> {
        let pragma = pragmas.into_iter().find(|pragma| {
            pragma
                .annotation()
                .is_some_and(|annotation| annotation.name == "test")
        })?;
        let Some(annotation) = pragma.annotation() else {
            return Some(TestPragma {
                span: pragma.token_span(),
                ..TestPragma::default()
            });
        };
        let span = annotation.span;
        let mut result = TestPragma {
            span,
            ..TestPragma::default()
        };
        let Some(object) = annotation.object() else {
            return Some(result);
        };
        result.cycles = object
            .get("cycles")
            .and_then(serde_json::Value::as_u64)
            .and_then(|cycles| u32::try_from(cycles).ok())
            .filter(|cycles| *cycles > 0);
        if let Some(timeout) = object.get("timeout").and_then(serde_json::Value::as_str) {
            match stc_manifest::parse_duration(timeout) {
                Some(DurationNs(ns)) => result.timeout_ns = Some(ns),
                None => self.diagnostics.push(Diagnostic::error(
                    "STC-E-PROJECT-008",
                    format!("invalid `timeout` duration `{timeout}` in `{{stc:test}}`"),
                    span,
                )),
            }
        }
        if let Some(trap) = object
            .get("expect-trap")
            .and_then(serde_json::Value::as_str)
        {
            match normalize_trap(trap) {
                Some(trap) => result.expect_trap = Some(trap),
                None => self.diagnostics.push(Diagnostic::error(
                    "STC-E-PROJECT-008",
                    format!("unknown trap `{trap}` in `{{stc:test}}`"),
                    span,
                )),
            }
        }
        Some(result)
    }

    fn invalid(&mut self, name: &str, span: Span, message: &str) {
        self.diagnostics.push(Diagnostic::error(
            "STC-E-TEST-003",
            format!("invalid test `{name}`: {message}"),
            span,
        ));
    }
}

fn is_input_section(section: &VarSection) -> bool {
    section.kind().is_some_and(|token| {
        matches!(
            token.kind(),
            SyntaxKind::KwVarInput | SyntaxKind::KwVarInOut
        )
    })
}

fn normalize_trap(text: &str) -> Option<String> {
    let upper = text.trim().to_ascii_uppercase();
    let name = upper.strip_prefix("STC_TRAP_").unwrap_or(&upper);
    TRAPS.contains(&name).then(|| format!("STC_TRAP_{name}"))
}

/// Merges `[[test.case]]` overrides and applies `[test]` defaults.
fn apply_manifest_cases(
    tests: &mut [TestCase],
    settings: &TestSettings,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for case in &settings.cases {
        let Some(test) = tests
            .iter_mut()
            .find(|test| test.name.eq_ignore_ascii_case(&case.name))
        else {
            diagnostics.push(Diagnostic::error(
                "STC-E-PROJECT-008",
                format!(
                    "`[[test.case]]` names `{}`, which is not a `{{stc:test}}` POU in this package",
                    case.name
                ),
                case.span,
            ));
            continue;
        };
        // The pragma fields were folded into the test; a manifest override of
        // the same field is a conflict even when the values agree.
        if let Some(cycles) = case.cycles {
            if test.pragma_cycles {
                diagnostics.push(conflict(&case.name, "cycles", case.span, test.pragma_span));
            } else {
                test.cycles = cycles;
            }
        }
        if let Some(timeout) = case.timeout {
            if test.pragma_timeout {
                diagnostics.push(conflict(&case.name, "timeout", case.span, test.pragma_span));
            } else {
                test.timeout_ns = timeout.as_nanos();
            }
        }
        if let Some(trap) = &case.expect_trap {
            if test.expect_trap.is_some() {
                diagnostics.push(conflict(
                    &case.name,
                    "expect-trap",
                    case.span,
                    test.pragma_span,
                ));
            } else {
                match normalize_trap(trap) {
                    Some(trap) => test.expect_trap = Some(trap),
                    None => diagnostics.push(Diagnostic::error(
                        "STC-E-PROJECT-008",
                        format!("unknown trap `{trap}` in `[[test.case]]`"),
                        case.span,
                    )),
                }
            }
        }
    }
    for test in tests {
        if test.kind != TestKind::Function && test.cycles == 0 {
            test.cycles = settings.default_cycles.max(1);
        }
        if test.timeout_ns == 0 {
            test.timeout_ns = settings.timeout.as_nanos();
        }
    }
}

fn conflict(name: &str, field: &str, case_span: Span, pragma_span: Span) -> Diagnostic {
    Diagnostic::error(
        "STC-E-PROJECT-008",
        format!("`{name}` sets `{field}` in both the `{{stc:test}}` pragma and `[[test.case]]`"),
        case_span,
    )
    .with_label(pragma_span, "the pragma sets it here")
}

/// Generates the synthetic wrapper and configuration for one test.
///
/// Function tests call the function once behind a one-shot guard and assert
/// its `BOOL` result. FB tests instantiate the block, call it once per scan,
/// and copy `PASSED` into a reserved field the host reads at the end. Program
/// tests configure the program directly. Configuration `STC_TEST` has one
/// resource and one task with no interval, priority, or `SINGLE`.
#[must_use]
pub fn wrapper_source(test: &TestCase) -> String {
    let mut source = String::new();
    match test.kind {
        TestKind::Function => {
            source.push_str("{stc:test}\n");
            source.push_str(&format!("PROGRAM {TEST_WRAPPER}\n"));
            source.push_str("VAR\n");
            source.push_str(&format!("  {TEST_VALUE} : BOOL;\n"));
            source.push_str(&format!("  {TEST_DONE} : BOOL;\n"));
            source.push_str("END_VAR\n");
            source.push_str(&format!("  IF NOT {TEST_DONE} THEN\n"));
            source.push_str(&format!("    {TEST_DONE} := TRUE;\n"));
            source.push_str(&format!("    {TEST_VALUE} := {}();\n", test.qualified));
            source.push_str(&format!(
                "    STC_ASSERT({TEST_VALUE}, \"test function returned FALSE\");\n"
            ));
            source.push_str("  END_IF;\n");
            source.push_str("END_PROGRAM\n\n");
            source.push_str(&configuration(TEST_WRAPPER));
        }
        TestKind::FunctionBlock => {
            source.push_str("{stc:test}\n");
            source.push_str(&format!("PROGRAM {TEST_WRAPPER}\n"));
            source.push_str("VAR\n");
            source.push_str(&format!("  {TEST_FB} : {};\n", test.qualified));
            source.push_str("END_VAR\n");
            source.push_str(&format!("  {TEST_FB}();\n"));
            source.push_str("END_PROGRAM\n\n");
            source.push_str(&configuration(TEST_WRAPPER));
        }
        TestKind::Program => {
            source.push_str(&configuration(&test.qualified));
        }
    }
    source
}

fn configuration(program: &str) -> String {
    format!(
        "CONFIGURATION {TEST_CONFIGURATION}\n\
         RESOURCE {TEST_RESOURCE} ON {TEST_CPU}\n\
         \x20 TASK {TEST_TASK} ();\n\
         \x20 PROGRAM {TEST_INSTANCE} WITH {TEST_TASK} : {program};\n\
         END_RESOURCE\n\
         END_CONFIGURATION\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_lexer::tokenize_bytes;
    use stc_manifest::TestSettings;
    use stc_parser::{ParseOptions, parse};
    use stc_span::FileId;

    fn parse_at(raw: u32, source: &str) -> ParsedFile {
        let mut sink = stc_diag::DiagnosticSink::unlimited();
        let file = FileId::new(raw);
        let lexed = tokenize_bytes(file, source.as_bytes(), &mut sink);
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        assert!(
            !sink.has_errors(),
            "parse errors in fixture:\n{source}\n{:#?}",
            sink.diagnostics()
        );
        parsed
    }

    fn discover_source(source: &str, settings: &TestSettings) -> Discovery {
        let parsed = parse_at(0, source);
        discover(
            &[SourceFileRef {
                relative: "tests/all.st",
                parsed: &parsed,
            }],
            settings,
            None,
        )
    }

    #[test]
    fn discovers_function_fb_and_program_tests_with_defaults() {
        let source = r#"
{stc:test}
FUNCTION AddWorks : BOOL
  AddWorks := TRUE;
END_FUNCTION

{stc:test {"cycles": 7}}
FUNCTION_BLOCK FbWorks
VAR_OUTPUT
  PASSED : BOOL;
END_VAR
  PASSED := TRUE;
END_FUNCTION_BLOCK

{stc:test {"timeout": "2s", "expect-trap": "DIV_ZERO"}}
PROGRAM DeepTimer
END_PROGRAM
"#;
        let discovery = discover_source(source, &TestSettings::default());
        assert!(
            discovery.diagnostics.is_empty(),
            "{:#?}",
            discovery.diagnostics
        );
        assert_eq!(discovery.tests.len(), 3);
        let names: Vec<&str> = discovery
            .tests
            .iter()
            .map(|test| test.name.as_str())
            .collect();
        assert_eq!(names, vec!["AddWorks", "DeepTimer", "FbWorks"]);
        let function = &discovery.tests[0];
        assert_eq!(function.kind, TestKind::Function);
        assert_eq!(function.cycles, 1);
        assert_eq!(function.timeout_ns, 5_000_000_000);
        let fb = &discovery.tests[2];
        assert!(fb.has_passed);
        assert_eq!(fb.cycles, 7);
        let program = &discovery.tests[1];
        assert_eq!(program.qualified, "DeepTimer");
        assert_eq!(program.timeout_ns, 2_000_000_000);
        assert_eq!(program.expect_trap.as_deref(), Some("STC_TRAP_DIV_ZERO"));
    }

    #[test]
    fn applies_manifest_overrides_and_rejects_conflicts() {
        let source = r#"
{stc:test}
FUNCTION First : BOOL
  First := TRUE;
END_FUNCTION

{stc:test {"cycles": 3, "timeout": "1s"}}
PROGRAM Second
END_PROGRAM
"#;
        let parsed = parse_at(0, source);
        let file = parsed.tree().file();
        let mut settings = TestSettings::default();
        settings.cases.push(stc_manifest::TestCase {
            name: "First".into(),
            cycles: Some(9),
            timeout: Some(DurationNs(2_000_000_000)),
            expect_trap: None,
            span: Span::new(file, 0, 1),
        });
        settings.cases.push(stc_manifest::TestCase {
            name: "Second".into(),
            cycles: Some(4),
            timeout: None,
            expect_trap: None,
            span: Span::new(file, 0, 1),
        });
        let discovery = discover(
            &[SourceFileRef {
                relative: "tests/all.st",
                parsed: &parsed,
            }],
            &settings,
            None,
        );
        assert!(
            discovery
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "STC-E-PROJECT-008"),
            "{:#?}",
            discovery.diagnostics
        );
        let first = discovery
            .tests
            .iter()
            .find(|test| test.name == "First")
            .expect("First");
        assert_eq!(first.cycles, 1, "function tests always run once");
        assert_eq!(first.timeout_ns, 2_000_000_000);
        let second = discovery
            .tests
            .iter()
            .find(|test| test.name == "Second")
            .expect("Second");
        assert_eq!(second.cycles, 3, "the pragma wins until the conflict");
    }

    #[test]
    fn rejects_duplicate_and_invalid_test_declarations() {
        let source = r#"
{stc:test}
FUNCTION Dup : BOOL
  Dup := TRUE;
END_FUNCTION

{stc:test}
FUNCTION dup : BOOL
  dup := TRUE;
END_FUNCTION

{stc:test}
FUNCTION BadInput : BOOL
VAR_INPUT
  x : DINT;
END_VAR
  BadInput := TRUE;
END_FUNCTION

{stc:test}
FUNCTION BadReturn : DINT
  BadReturn := 1;
END_FUNCTION

{stc:test}
FUNCTION_BLOCK BadPassed
VAR_OUTPUT
  PASSED : DINT;
END_VAR
END_FUNCTION_BLOCK

{stc:test}
FUNCTION STC_TEST_WRAPPER : BOOL
  STC_TEST_WRAPPER := TRUE;
END_FUNCTION

{stc:test}
CLASS NotATest
END_CLASS
"#;
        let discovery = discover_source(source, &TestSettings::default());
        let codes: Vec<&str> = discovery
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&"STC-E-TEST-002"), "{codes:?}");
        assert!(
            codes
                .iter()
                .filter(|code| **code == "STC-E-TEST-003")
                .count()
                >= 4,
            "{codes:?}"
        );
    }

    #[test]
    fn wrapper_source_is_a_synthetic_single_task_configuration() {
        let source = r#"
{stc:test}
FUNCTION Deep.Fn : BOOL
  Deep.Fn := TRUE;
END_FUNCTION
"#;
        // Use a real namespace so the qualified path is exercised.
        let source = source.replace("FUNCTION Deep.Fn", "FUNCTION Fn").replace(
            "{stc:test}\nFUNCTION",
            "NAMESPACE Deep\n{stc:test}\nFUNCTION",
        );
        let source = format!("{source}\nEND_NAMESPACE\n");
        let discovery = discover_source(&source, &TestSettings::default());
        let test = &discovery.tests[0];
        assert_eq!(test.qualified, "Deep.Fn");
        let wrapper = wrapper_source(test);
        assert!(wrapper.contains("CONFIGURATION STC_TEST"), "{wrapper}");
        assert!(wrapper.contains("TASK STC_TEST_TASK ()"), "{wrapper}");
        assert!(
            wrapper.contains("PROGRAM STC_TEST_INSTANCE WITH STC_TEST_TASK : STC_TEST_WRAPPER"),
            "{wrapper}"
        );
        assert!(wrapper.contains("STC_TEST_VALUE := Deep.Fn()"), "{wrapper}");
    }
}
