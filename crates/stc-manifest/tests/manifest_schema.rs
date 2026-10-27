mod common;

use stc_diag::Diagnostic;
use stc_manifest::{
    Baseline, DurationNs, EmitKind, IoBackend, IoDirection, LibType, LintLevel, OptLevel,
    SyntaxMode, parse_lockfile, parse_manifest,
};
use stc_span::FileId;

fn parse(text: &str) -> Result<stc_manifest::ParsedManifest, Vec<Diagnostic>> {
    parse_manifest(text, FileId::new(0))
}

fn parse_ok(text: &str) -> stc_manifest::ParsedManifest {
    match parse(text) {
        Ok(parsed) => parsed,
        Err(diagnostics) => panic!("expected success, got {diagnostics:#?}"),
    }
}

fn parse_err(text: &str) -> Vec<Diagnostic> {
    match parse(text) {
        Ok(parsed) => panic!("expected an error, got {:#?}", parsed.manifest),
        Err(diagnostics) => diagnostics,
    }
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn assert_code(diagnostics: &[Diagnostic], code: &str) {
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {:?}",
        codes(diagnostics)
    );
}

const MINIMAL: &str = r#"schema = 1

[package]
name = "app"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["src/**/*.st"]
"#;

const FULL: &str = r#"schema = 1

[package]
name = "motor-control"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[dependencies]
motor-lib = { path = "../motor-lib" }

[project]
sources = ["src/**/*.st"]
stdlib = true
configuration = "Main"

[build]
target = "x86_64-pc-windows-msvc"
opt-level = 2
debug = true
syntax = "strict"
emit = ["module", "ir"]
lib-type = "static"
out-dir = "target/stc"
clang = "C:/LLVM/bin/clang.exe"
runtime = "target/debug/stc_runtime.lib"

[io]
backend = "null"
file = "io.bin"
input-bytes = 16
output-bytes = 16
marker-bytes = 64

[[io.devices]]
name = "drive1"
kind = "ethercat"
config = { vendor = "0x1234", revision = 3 }

  [[io.devices.channels]]
  name = "DI5"
  direction = "input"
  width = 1

[[io.bindings]]
variable = "Main.MyBool"
device = "drive1"
channel = 5
direction = "input"

[retain]
enabled = true
file = "motor.stcstate"
on-mismatch = "error"

[runtime]
module-dir = "runtime/modules"
manifest-dir = "runtime/manifests"
max-retired-generations = 1

[online]
baseline = "active"
cold-policy = "require-confirmation"
allow-untracked-removal = false

[debug]
default = true
protocol = "pipe"
force-sessions-end-on-swap = true

[lints]
warnings = "warn"
"STC-W-NAME-002" = "allow"
"STC-W-LEX-008" = "deny"

[run]
cycles = 10
cycle-time = "10ms"

[test]
sources = ["tests/**/*.st"]
timeout = "5s"
default-cycles = 1
mock-clock = "0ns"
io = "memory"

[[test.case]]
name = "TimerBlink"
cycles = 25
timeout = "2s"
expect-trap = "STC_TRAP_DIV_ZERO"
"#;

#[test]
fn minimal_manifest_applies_documented_defaults() {
    let parsed = parse_ok(MINIMAL);
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let manifest = parsed.manifest;
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.package.name, "app");
    assert_eq!(manifest.project.sources[0].pattern, "src/**/*.st");
    assert!(manifest.project.stdlib);
    assert_eq!(manifest.project.configuration, None);
    assert_eq!(manifest.build.opt_level, OptLevel::O0);
    assert_eq!(manifest.build.out_dir, "target/stc/debug");
    assert_eq!(manifest.build.lib_type, LibType::Static);
    assert!(manifest.build.debug);
    assert_eq!(manifest.build.syntax, SyntaxMode::Strict);
    assert_eq!(manifest.build.emit, vec![EmitKind::Module]);
    assert_eq!(manifest.io.backend, IoBackend::Null);
    assert!(manifest.retain.enabled);
    assert_eq!(manifest.runtime.max_retired_generations, 1);
    assert_eq!(manifest.online.baseline, Baseline::Active);
    assert!(!manifest.online.allow_untracked_removal);
    assert!(manifest.debug.default);
    assert_eq!(manifest.debug.protocol.as_str(), "pipe");
    assert_eq!(manifest.lints.warnings, LintLevel::Warn);
    assert_eq!(manifest.run.cycles, 0);
    assert_eq!(manifest.run.cycle_time, None);
    assert_eq!(manifest.test.timeout, DurationNs(5_000_000_000));
    assert_eq!(manifest.test.default_cycles, 1);
    assert_eq!(manifest.test.mock_clock, DurationNs(0));
    assert_eq!(manifest.test.effective_io(), IoBackend::Memory);
    assert!(manifest.test.sources.is_empty());
}

#[test]
fn full_manifest_parses() {
    let parsed = parse_ok(FULL);
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let manifest = parsed.manifest;
    assert_eq!(manifest.dependencies.len(), 1);
    assert_eq!(manifest.dependencies[0].alias, "motor-lib");
    assert_eq!(manifest.dependencies[0].path, "../motor-lib");
    assert_eq!(manifest.build.opt_level, OptLevel::O2);
    assert_eq!(manifest.build.emit, vec![EmitKind::Ir, EmitKind::Module]);
    assert_eq!(manifest.io.backend, IoBackend::Null);
    assert_eq!(manifest.io.input_bytes, Some(16));
    assert_eq!(manifest.io.devices.len(), 1);
    assert_eq!(manifest.io.devices[0].name, "drive1");
    assert_eq!(manifest.io.devices[0].channels.len(), 1);
    assert_eq!(
        manifest.io.devices[0].channels[0].direction,
        IoDirection::Input
    );
    assert_eq!(manifest.io.bindings.len(), 1);
    assert_eq!(manifest.retain.file.as_deref(), Some("motor.stcstate"));
    assert_eq!(manifest.run.cycles, 10);
    assert_eq!(manifest.run.cycle_time, Some(DurationNs(10_000_000)));
    assert_eq!(manifest.test.cases.len(), 1);
    assert_eq!(manifest.test.cases[0].name, "TimerBlink");
    assert_eq!(manifest.test.cases[0].cycles, Some(25));
    assert_eq!(
        manifest.test.cases[0].timeout,
        Some(DurationNs(2_000_000_000))
    );
    assert_eq!(
        manifest.test.cases[0].expect_trap.as_deref(),
        Some("STC_TRAP_DIV_ZERO")
    );
}

#[test]
fn unknown_keys_get_did_you_mean_suggestions() {
    let text = format!("projet = 1\n{MINIMAL}");
    let diagnostics = parse_err(&text);
    assert_code(&diagnostics, "STC-E-PROJECT-002");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("projet"))
        .expect("unknown-key diagnostic");
    assert_eq!(diagnostic.suggestions[0].replacement, "project");
    let span = diagnostic.primary;
    assert_eq!(
        &text[span.start as usize..span.end as usize],
        "projet",
        "diagnostic points at the key"
    );
}

#[test]
fn snake_case_keys_are_unknown_with_kebab_suggestions() {
    let text = MINIMAL.replace("sources =", "Sources =") + "\n[build]\nout_dir = \"x\"\n";
    let diagnostics = parse_err(&text);
    assert_code(&diagnostics, "STC-E-PROJECT-002");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("out_dir"))
        .expect("out_dir diagnostic");
    assert_eq!(diagnostic.suggestions[0].replacement, "out-dir");
}

#[test]
fn duplicate_toml_keys_are_parse_errors() {
    let diagnostics = parse_err("schema = 1\nschema = 2\n");
    assert_code(&diagnostics, "STC-E-PROJECT-001");
}

#[test]
fn schema_must_be_one() {
    assert_code(
        &parse_err(&MINIMAL.replace("schema = 1", "schema = 2")),
        "STC-E-PROJECT-002",
    );
    assert_code(
        &parse_err(&MINIMAL.replace("schema = 1", "schema = \"1\"")),
        "STC-E-PROJECT-002",
    );
    assert_code(
        &parse_err(&MINIMAL.replace("schema = 1\n", "")),
        "STC-E-PROJECT-002",
    );
}

#[test]
fn package_fields_are_validated() {
    assert_code(
        &parse_err(&MINIMAL.replace("name = \"app\"", "name = \"1app\"")),
        "STC-E-PROJECT-003",
    );
    assert_code(
        &parse_err(&MINIMAL.replace("name = \"app\"", "name = \"a.b\"")),
        "STC-E-PROJECT-003",
    );
    assert_code(
        &parse_err(&MINIMAL.replace("version = \"0.1.0\"", "version = \"1.0\"")),
        "STC-E-PROJECT-003",
    );
    assert_code(
        &parse_err(&MINIMAL.replace(
            "language = \"stc-2013-oop-utf8-v1\"",
            "language = \"other\"",
        )),
        "STC-E-PROJECT-003",
    );
    assert_code(
        &parse_err(&MINIMAL.replace("[package]", "[packag]")),
        "STC-E-PROJECT-002",
    );
}

#[test]
fn project_sources_are_required() {
    let without_sources = MINIMAL.replace("sources = [\"src/**/*.st\"]\n", "");
    assert_code(&parse_err(&without_sources), "STC-E-PROJECT-004");
    let empty = MINIMAL.replace("[\"src/**/*.st\"]", "[]");
    assert_code(&parse_err(&empty), "STC-E-PROJECT-004");
    let wrong_type = MINIMAL.replace("[\"src/**/*.st\"]", "\"src/*.st\"");
    assert_code(&parse_err(&wrong_type), "STC-E-PROJECT-004");
    let escaping = MINIMAL.replace("src/**/*.st", "../src/*.st");
    assert_code(&parse_err(&escaping), "STC-E-PROJECT-004");
    let backslash = MINIMAL.replace("src/**/*.st", "src\\\\*.st");
    assert_code(&parse_err(&backslash), "STC-E-PROJECT-004");
}

#[test]
fn build_settings_are_validated() {
    let cases = [
        ("opt-level = 2", "opt-level = 9", "STC-E-PROJECT-010"),
        ("opt-level = 2", "opt-level = true", "STC-E-PROJECT-002"),
        (
            "lib-type = \"static\"",
            "lib-type = \"din\"",
            "STC-E-PROJECT-010",
        ),
        (
            "syntax = \"strict\"",
            "syntax = \"loose\"",
            "STC-E-PROJECT-010",
        ),
        (
            "emit = [\"module\", \"ir\"]",
            "emit = []",
            "STC-E-PROJECT-010",
        ),
        (
            "emit = [\"module\", \"ir\"]",
            "emit = [\"module\", \"module\"]",
            "STC-E-PROJECT-010",
        ),
        (
            "out-dir = \"target/stc\"",
            "out-dir = \"../out\"",
            "STC-E-PROJECT-010",
        ),
        (
            "target = \"x86_64-pc-windows-msvc\"",
            "target = 1",
            "STC-E-PROJECT-002",
        ),
    ];
    for (from, to, code) in cases {
        let text = FULL.replace(from, to);
        assert_code(&parse_err(&text), code);
    }
}

#[test]
fn io_settings_are_validated() {
    assert_code(
        &parse_err(&FULL.replace("backend = \"null\"", "backend = \"hardware\"")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("input-bytes = 16", "input-bytes = -1")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("name = \"drive1\"", "name = \"\"")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("kind = \"ethercat\"\n", "")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("device = \"drive1\"", "device = \"missing\"")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("channel = 5", "channel = \"DI9\"")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "direction = \"input\"\n  width = 1",
            "direction = \"sideways\"\n  width = 1",
        )),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&FULL.replace("width = 1", "width = 0")),
        "STC-E-PROJECT-009",
    );
}

#[test]
fn retain_runtime_online_and_debug_are_validated() {
    assert_code(
        &parse_err(&FULL.replace("on-mismatch = \"error\"", "on-mismatch = \"ignore\"")),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace("file = \"motor.stcstate\"", "file = \"../state.bin\"")),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "max-retired-generations = 1",
            "max-retired-generations = -1",
        )),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "manifest-dir = \"runtime/manifests\"",
            "manifest-dir = \"/abs\"",
        )),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace("baseline = \"active\"", "baseline = \"explicit\"")),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace("protocol = \"pipe\"", "protocol = \"tcp\"")),
        "STC-E-PROJECT-010",
    );
    assert_code(
        &parse_err(&FULL.replace("cycle-time = \"10ms\"", "cycle-time = \"ten\"")),
        "STC-E-PROJECT-010",
    );
}

#[test]
fn lints_are_validated() {
    assert_code(
        &parse_err(&FULL.replace(
            "\"STC-W-NAME-002\" = \"allow\"",
            "\"STC-W-NOPE-001\" = \"allow\"",
        )),
        "STC-E-PROJECT-011",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "\"STC-W-NAME-002\" = \"allow\"",
            "\"STC-E-TYPE-001\" = \"warn\"",
        )),
        "STC-E-PROJECT-011",
    );
    assert_code(
        &parse_err(&FULL.replace("warnings = \"warn\"", "warnings = \"loud\"")),
        "STC-E-PROJECT-011",
    );
    let parsed = parse_ok(FULL);
    assert_eq!(parsed.manifest.lints.warnings, LintLevel::Warn);
    assert_eq!(
        parsed.manifest.lints.codes.get("STC-W-NAME-002"),
        Some(&LintLevel::Allow)
    );
    assert_eq!(
        parsed.manifest.lints.codes.get("STC-W-LEX-008"),
        Some(&LintLevel::Deny)
    );
}

#[test]
fn test_settings_are_validated() {
    assert_code(
        &parse_err(&FULL.replace("timeout = \"5s\"", "timeout = \"soon\"")),
        "STC-E-PROJECT-008",
    );
    assert_code(
        &parse_err(&FULL.replace("default-cycles = 1", "default-cycles = 0")),
        "STC-E-PROJECT-008",
    );
    assert_code(
        &parse_err(&FULL.replace("mock-clock = \"0ns\"", "mock-clock = \"0\"")),
        "STC-E-PROJECT-008",
    );
    assert_code(
        &parse_err(&FULL.replace("io = \"memory\"", "io = \"disk\"")),
        "STC-E-PROJECT-008",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "expect-trap = \"STC_TRAP_DIV_ZERO\"",
            "expect-trap = \"STC_TRAP_NOPE\"",
        )),
        "STC-E-PROJECT-008",
    );
    let duplicate = FULL.replace(
        "expect-trap = \"STC_TRAP_DIV_ZERO\"",
        "expect-trap = \"STC_TRAP_DIV_ZERO\"\n\n[[test.case]]\nname = \"TimerBlink\"",
    );
    assert_code(&parse_err(&duplicate), "STC-E-TEST-002");
}

#[test]
fn dependencies_are_validated() {
    assert_code(
        &parse_err(&FULL.replace(
            "motor-lib = { path = \"../motor-lib\" }",
            "motor-lib = { version = \"1\" }",
        )),
        "STC-E-PROJECT-005",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "motor-lib = { path = \"../motor-lib\" }",
            "motor-lib = { path = \"\" }",
        )),
        "STC-E-PROJECT-005",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "motor-lib = { path = \"../motor-lib\" }",
            "1bad = { path = \"../motor-lib\" }",
        )),
        "STC-E-PROJECT-005",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "motor-lib = { path = \"../motor-lib\" }",
            "motor-lib = { path = \"../motor-lib\", registery = \"x\" }",
        )),
        "STC-E-PROJECT-002",
    );
    let aliased = FULL.replace(
        "motor-lib = { path = \"../motor-lib\" }",
        "motor-lib = { path = \"../motor-lib\", package = \"real-lib\", version = \"0.2\" }",
    );
    let parsed = parse_ok(&aliased);
    assert_eq!(
        parsed.manifest.dependencies[0].package.as_deref(),
        Some("real-lib")
    );
}

#[test]
fn debug_settings_record_explicit_values_without_obsolete_warnings() {
    let build_only = parse_ok(&FULL.replace("default = true\n", ""));
    assert!(build_only.warnings.is_empty());
    assert!(build_only.manifest.build.debug);
    assert!(build_only.manifest.build.debug_explicit);
    assert!(build_only.manifest.debug.default);
    assert!(!build_only.manifest.debug.default_explicit);

    let default_only = parse_ok(&FULL.replace("debug = true\n", ""));
    assert!(default_only.warnings.is_empty());
    assert!(default_only.manifest.build.debug);
    assert!(!default_only.manifest.build.debug_explicit);
    assert!(default_only.manifest.debug.default);
    assert!(default_only.manifest.debug.default_explicit);

    let explicit_false = parse_ok(
        &FULL
            .replace("debug = true", "debug = false")
            .replace("default = true", "default = false"),
    );
    assert!(explicit_false.warnings.is_empty());
    assert!(!explicit_false.manifest.build.debug);
    assert!(explicit_false.manifest.build.debug_explicit);
    assert!(!explicit_false.manifest.debug.default);
    assert!(explicit_false.manifest.debug.default_explicit);
}

#[test]
fn conflicting_explicit_debug_settings_are_rejected() {
    let conflicted = FULL.replace("debug = true", "debug = false");
    assert_code(&parse_err(&conflicted), "STC-E-PROJECT-010");

    let agreeing = FULL
        .replace("debug = true", "debug = false")
        .replace("default = true", "default = false");
    let parsed = parse_ok(&agreeing);
    assert!(!parsed.manifest.build.debug);
    assert!(!parsed.manifest.debug.default);
}

#[test]
fn table_shapes_are_validated() {
    assert_code(
        &parse_err(&format!("{MINIMAL}\n[io]\ndevices = \"drive1\"\n")),
        "STC-E-PROJECT-009",
    );
    assert_code(
        &parse_err(&MINIMAL.replace(
            "[project]",
            "[dependencies]\nlib = \"not-a-table\"\n\n[project]",
        )),
        "STC-E-PROJECT-005",
    );
    assert_code(
        &parse_err(&FULL.replace(
            "expect-trap = \"STC_TRAP_DIV_ZERO\"",
            "expect-trap = \"STC_TRAP_DIV_ZERO\"\nsurprise = true",
        )),
        "STC-E-PROJECT-002",
    );
}

#[test]
fn io_device_and_channel_names_are_unique() {
    let duplicate = FULL.replace(
        "[[io.bindings]]",
        "[[io.devices]]\nname = \"drive1\"\nkind = \"motor\"\n\n[[io.bindings]]",
    );
    assert_code(&parse_err(&duplicate), "STC-E-PROJECT-009");
    let duplicate_channel = FULL.replace(
        "  [[io.devices.channels]]\n  name = \"DI5\"\n  direction = \"input\"\n  width = 1",
        "  [[io.devices.channels]]\n  name = \"DI5\"\n  direction = \"input\"\n  width = 1\n\n  [[io.devices.channels]]\n  name = \"DI5\"\n  direction = \"output\"\n  width = 1",
    );
    assert_code(&parse_err(&duplicate_channel), "STC-E-PROJECT-009");
}

#[test]
fn lockfile_parser_rejects_malformed_documents() {
    let diagnostics = parse_lockfile("schema = 2\n", FileId::new(0)).expect_err("bad schema");
    assert_code(&diagnostics, "STC-E-LOCK-003");
    let diagnostics = parse_lockfile("nonsense = true\n", FileId::new(0)).expect_err("bad key");
    assert_code(&diagnostics, "STC-E-LOCK-003");
    let diagnostics = parse_lockfile(
        "schema = 1\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\nsource = \"root\"\ndependencies = []\n",
        FileId::new(0),
    );
    assert!(diagnostics.is_ok());
}

#[test]
fn bom_and_crlf_are_supported_and_spans_stay_aligned() {
    let text = format!("\u{feff}{}", MINIMAL.replace('\n', "\r\n"));
    let parsed = parse_ok(&text);
    assert_eq!(parsed.manifest.package.name, "app");

    let text = format!("\u{feff}projet = 1\r\n{MINIMAL}");
    let diagnostics = parse_err(&text);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("projet"))
        .expect("unknown-key diagnostic");
    let span = diagnostic.primary;
    assert_eq!(&text[span.start as usize..span.end as usize], "projet");
}

#[test]
fn manifest_diagnostics_tolerate_arbitrary_text() {
    for text in [
        "",
        "\u{feff}",
        "schema = ",
        "\0\u{1}\u{2}",
        "[package]\nname = \"x\"\n[[[[",
        "schema = 1\n[package]\nname = \"x\"\nversion = \"1.0.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n[project]\nsources = [\"[\"]\n",
    ] {
        let _ = parse_manifest(text, FileId::new(0));
    }
}
