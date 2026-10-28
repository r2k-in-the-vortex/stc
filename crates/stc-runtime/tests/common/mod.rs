//! Shared helpers for the phase-6 host lifecycle integration tests.
//!
//! Each fixture compiles a tiny C shared library with `clang`. The library
//! embeds a real compiler-produced descriptor and exposes the ABI 2 entry
//! points; its prepare/cycle bodies are no-ops (or fail on request) so the host
//! state machine can be exercised without execution lowering.

// The helper module is shared by several integration-test binaries; not every
// binary uses every helper.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use stc_codegen::module::{ModuleArtifacts, ModuleOptions, project, source_fingerprint};
use stc_codegen::{Mode, ModuleBuildOptions, Options, Output, ProjectOptions};
use stc_diag::DiagnosticSink;
use stc_got::{Endianness, GotId, Json, Manifest, Rename, Target};
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::analyze;
use stc_span::{FileId, SourceMap};

static COUNTER: AtomicU64 = AtomicU64::new(0);

static IMAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serializes tests that configure or observe the process-global image.
///
/// The runtime has one process image per host process, so tests that assert on
/// its contents must not run concurrently with each other.
pub fn image_guard() -> std::sync::MutexGuard<'static, ()> {
    IMAGE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A program with two scalar globals and one program body.
pub const SIMPLE: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Flag := Counter > 3;
END_PROGRAM
"#;

/// A code-only variant of [`SIMPLE`].
pub const SIMPLE_ALTERNATE: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 2;
  Counter := ticks;
  Flag := Counter > 4;
END_PROGRAM
"#;

/// [`SIMPLE`] with a second scalar global added.
pub const SIMPLE_ADDED: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Flag : BOOL;
  Added : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Flag := Counter > 3;
  Added := Added + 1;
END_PROGRAM
"#;

/// [`SIMPLE`] with `Flag` removed and `Added` introduced.
pub const SIMPLE_REPLACED: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Added : DINT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
  Added := Added + 1;
END_PROGRAM
"#;

/// A program with a direct `AT` variable.
pub const DIRECT: &str = r#"
PROGRAM Main
VAR
  input AT %IX1.2 : BOOL;
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// A program with an exported function.
pub const EXPORTED: &str = r#"
{stc:export}
FUNCTION Read : DINT
  Read := 1;
END_FUNCTION

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// A program without the exported function.
pub const EXPORT_REMOVED: &str = r#"
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// Returns the fixed test target used by compiler artifacts and host policy.
#[must_use]
pub fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

/// Returns the target of the running test host.
#[must_use]
pub fn host_target() -> Target {
    stc_codegen::resolve_target(Some(&stc_codegen::host_triple()))
}

/// Builds artifacts without a baseline.
#[must_use]
pub fn artifacts(source: &str, name: &str) -> ModuleArtifacts {
    build(source, name, None, false, Vec::new(), false, target())
}

/// Builds artifacts for the running host (for the C embedding surface).
#[must_use]
pub fn artifacts_native(source: &str, name: &str) -> ModuleArtifacts {
    build(source, name, None, false, Vec::new(), false, host_target())
}

/// Builds artifacts with a baseline.
#[must_use]
pub fn artifacts_with(source: &str, name: &str, baseline: Manifest, cold: bool) -> ModuleArtifacts {
    build(
        source,
        name,
        Some(baseline),
        cold,
        Vec::new(),
        false,
        target(),
    )
}

/// Build-shape flags used by debug capability tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildFlags {
    /// Optimised, stripped build (`release` debug mode).
    pub release: bool,
    /// Test build (test-clock service, ABI minor 2).
    pub test_build: bool,
    /// Descriptor debug checkpoint hooks (ABI 2.3, service 13).
    pub debug_hooks: bool,
}

/// Builds artifacts with explicit build-shape flags and no baseline.
#[must_use]
pub fn artifacts_with_flags(source: &str, name: &str, flags: BuildFlags) -> ModuleArtifacts {
    build_flags(
        source,
        name,
        None,
        false,
        Vec::new(),
        false,
        target(),
        flags,
    )
}

/// Runs the complete code generator for explicit build-shape flags.
///
/// The returned [`Output`] carries the additive debug metadata and frame
/// hints that the runtime tests turn into the metadata and frame sidecars.
#[must_use]
pub fn output_with_flags(source: &str, name: &str, flags: BuildFlags) -> Output {
    let hir = clean_hir(source);
    let mut source_map = SourceMap::new();
    source_map.add_file("src/main.st", source);
    let sources = vec!["src/main.st".to_string()];
    let target = target();
    let options = Options {
        artifact_name: name,
        target: Some(target.triple.as_str()),
        debug: true,
        emit_metadata: true,
        mode: Mode::Executable,
        relaxed: false,
        release: flags.release,
        sources: &sources,
        source_paths: Some(&sources),
        source_map: Some(&source_map),
        module: ModuleBuildOptions::default(),
        project: ProjectOptions::default(),
        test_build: flags.test_build,
    };
    stc_codegen::generate(&hir, &options)
        .unwrap_or_else(|errors| panic!("code generation failed: {errors:#?}"))
}

/// Builds artifacts with a baseline, renames, and the untracked-removal policy.
#[must_use]
pub fn artifacts_with_renames(
    source: &str,
    name: &str,
    baseline: Manifest,
    cold: bool,
    renames: Vec<Rename>,
    allow_untracked_removal: bool,
) -> ModuleArtifacts {
    build(
        source,
        name,
        Some(baseline),
        cold,
        renames,
        allow_untracked_removal,
        target(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build(
    source: &str,
    name: &str,
    baseline: Option<Manifest>,
    cold: bool,
    renames: Vec<Rename>,
    allow_untracked_removal: bool,
    target: Target,
) -> ModuleArtifacts {
    build_flags(
        source,
        name,
        baseline,
        cold,
        renames,
        allow_untracked_removal,
        target,
        BuildFlags::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_flags(
    source: &str,
    name: &str,
    baseline: Option<Manifest>,
    cold: bool,
    renames: Vec<Rename>,
    allow_untracked_removal: bool,
    target: Target,
    flags: BuildFlags,
) -> ModuleArtifacts {
    let hir = clean_hir(source);
    let sources = vec![format!("{name}.st")];
    let fingerprint = source_fingerprint(&[(format!("{name}.st"), source.as_bytes().to_vec())])
        .expect("source fingerprint");
    let mut options = ModuleOptions::new(target, name, "stc 0.1.0", fingerprint);
    options.baseline = baseline.as_ref();
    options.cold = cold;
    options.renames = renames;
    options.allow_untracked_removal = allow_untracked_removal;
    options.release = flags.release;
    options.test_build = flags.test_build;
    options.debug_hooks = flags.debug_hooks;
    project(&hir, &sources, &options)
        .unwrap_or_else(|errors| panic!("module projection failed: {errors:#?}"))
}

fn clean_hir(source: &str) -> HirCrate {
    let mut sink = DiagnosticSink::unlimited();
    let file = FileId::new(0);
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
        "parse errors for:\n{source}\n{:?}",
        sink.diagnostics()
    );
    let analysis = analyze(&[parsed], &mut sink);
    analysis
        .hir
        .check_invariants()
        .unwrap_or_else(|message| panic!("HIR invariant: {message}"));
    let errors: Vec<_> = sink
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert!(
        errors.is_empty(),
        "semantic errors for:\n{source}\n{errors:#?}"
    );
    analysis.hir
}

/// Requested entry-point behavior for a fixture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Behavior {
    /// Value `stc_module_attach` returns.
    pub attach: i32,
    /// Value `stc_module_prepare` returns.
    pub prepare: i32,
    /// Number of leading cycle calls that return 1.
    pub cycle_failures: u32,
    /// Value `stc_module_shutdown` returns.
    pub shutdown: i32,
    /// Transient bytes requested through the host API during prepare.
    pub prepare_alloc_bytes: u64,
}

/// A compiled no-op module paired with its manifest.
pub struct Fixture {
    /// Loadable shared-library path.
    pub module_path: PathBuf,
    /// Binary manifest path.
    pub manifest_path: PathBuf,
    directory: PathBuf,
}

impl Fixture {
    /// Compiles a fixture, returning `None` when clang is unavailable and not
    /// required.
    pub fn build(name: &str, artifacts: ModuleArtifacts, behavior: Behavior) -> Option<Self> {
        let text = c_source(&artifacts, behavior, None);
        let (module_path, manifest_path, directory) = compile(name, &text, &artifacts)?;
        Some(Self {
            module_path,
            manifest_path,
            directory,
        })
    }

    /// Returns the fixture directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // A still-loaded module can keep the file open on Windows; failing to
        // clean the temporary directory is harmless.
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn clang() -> Option<String> {
    let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
    match Command::new(&candidate).arg("--version").output() {
        Ok(output) if output.status.success() => Some(candidate),
        _ => {
            assert!(
                std::env::var_os("STC_REQUIRE_CLANG").is_none(),
                "clang is required (STC_REQUIRE_CLANG=1) but was not found"
            );
            eprintln!("note: clang not found; skipping fixture module test");
            None
        }
    }
}

/// Compiles one fixture C source and writes its manifest beside the module.
fn compile(
    name: &str,
    text: &str,
    artifacts: &ModuleArtifacts,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let clang = clang()?;
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "stc-runtime-host-{}-{id}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create fixture directory");

    let source_path = directory.join("fixture.c");
    fs::write(&source_path, text).expect("write fixture source");
    let module_path = if cfg!(windows) {
        directory.join(format!("{name}.dll"))
    } else {
        directory.join(format!("lib{name}.so"))
    };
    let mut command = Command::new(clang);
    command
        .arg("-shared")
        .arg("-O0")
        .arg("-fno-stack-protector")
        .arg("-fno-omit-frame-pointer")
        .arg("-fno-optimize-sibling-calls");
    if cfg!(windows) {
        command.arg("-nostdlib");
    } else {
        command.arg("-fPIC");
    }
    if cfg!(target_arch = "aarch64") {
        // Keep saved link registers unsigned so the format-1 frame walk can
        // match caller addresses to the synthetic sidecar (D-202).
        command.arg("-mbranch-protection=none");
    }
    command.arg("-o").arg(&module_path).arg(&source_path);
    let output = command.output().expect("run clang");
    assert!(
        output.status.success(),
        "clang rejected the fixture module:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The host derives the manifest name from the module file stem, which
    // carries a `lib` prefix on Unix; use the same rule here.
    let manifest_path = stc_runtime::module_info::manifest_path(&module_path);
    fs::write(&manifest_path, &artifacts.manifest_binary).expect("write manifest");
    Some((module_path, manifest_path, directory))
}

// ---------------------------------------------------------------------------
// Phase-6 stopped-frame fixtures
// ---------------------------------------------------------------------------

/// ST program used by the phase-6 stop/step/stack fixtures.
///
/// `Inner` and `Outer` both carry multiple checkpoints so the C fixture can
/// call service 13 at those identities; the program body keeps the module
/// deployable.
pub const STOP_SOURCE: &str = r#"VAR_GLOBAL
  G_COUNTER : DINT;
END_VAR

FUNCTION Inner : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
END_VAR
  r := n + 1;
  r := r + 1;
  r := r + 1;
  Inner := r;
END_FUNCTION

FUNCTION Outer : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
END_VAR
  r := n + 1;
  r := r + 2;
  r := r + 2;
  Outer := r;
END_FUNCTION

PROGRAM Main
VAR
  total : DINT;
END_VAR
  total := Outer(Inner(1));
  G_COUNTER := G_COUNTER + total;
END_PROGRAM
"#;

/// C call shape emitted by a stopped-frame fixture.
#[derive(Clone, Copy, Debug)]
pub enum StopFlow {
    /// `Outer` hooks, calls `Inner` (two hooks), then hooks again.
    Nested,
    /// One function spins on `monotonic_now_ns`, then calls two hooks.
    Pause,
    /// Recursion `depth` frames deep, then one hook at the deepest level.
    Recursive(u32),
    /// Calls one hook with a span that does not match its checkpoint.
    Mismatch,
}

/// One checkpoint exposed to a stop test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopCheckpoint {
    /// Dense checkpoint identity.
    pub id: u32,
    /// Span-table index.
    pub span: u32,
    /// 1-based source line.
    pub line: u32,
    /// 1-based source column.
    pub column: u32,
    /// Package-relative source file.
    pub file: String,
}

/// The four hook sites used by the nested flow.
#[derive(Clone, Debug)]
pub struct StopHooks {
    /// Outer hook before the call.
    pub outer_before: StopCheckpoint,
    /// First inner hook.
    pub inner_first: StopCheckpoint,
    /// Second inner hook.
    pub inner_second: StopCheckpoint,
    /// Outer hook after the call.
    pub outer_after: StopCheckpoint,
}

/// A compiled stop-capable module with paired metadata and frame sidecars.
pub struct StopFixture {
    /// Loadable shared-library path.
    pub module_path: PathBuf,
    /// Binary manifest path.
    pub manifest_path: PathBuf,
    /// GOT artifacts used to locate roots in host storage.
    pub artifacts: ModuleArtifacts,
    /// Manifest identity of `Inner`.
    pub inner: GotId,
    /// Manifest identity of `Outer`.
    pub outer: GotId,
    /// Symbol reported for `Inner`.
    pub inner_symbol: String,
    /// Symbol reported for `Outer`.
    pub outer_symbol: String,
    /// Hook sites in the compiled flow.
    pub hooks: StopHooks,
    /// Dense checkpoint count in the metadata sidecar.
    pub checkpoint_count: u32,
    /// Paired build identity.
    pub build_id: GotId,
    directory: PathBuf,
}

impl StopFixture {
    /// Builds a stopped-frame fixture from a debug-hook code generation output.
    ///
    /// The frame sidecar is synthesized from the manifest identities and the
    /// layout of the hand-written C functions. It uses the same format the
    /// driver's DWARF/CodeView extraction writes, so the runtime validates it
    /// exactly as a production sidecar.
    pub fn build(name: &str, output: &Output, flow: StopFlow) -> Option<Self> {
        let artifacts = output.got.clone();
        let inner_record = function_record(&artifacts, "Inner");
        let outer_record = function_record(&artifacts, "Outer");
        let inner = inner_record.got_id;
        let outer = outer_record.got_id;
        let debug = output
            .metadata
            .debug
            .as_ref()
            .expect("stop fixture requires debug metadata");
        assert!(debug.hooks, "stop fixture requires hooks");
        let checkpoint_count = u32::try_from(debug.checkpoints.len()).expect("checkpoint count");
        let inner_checkpoints = checkpoints_for(output, inner);
        let outer_checkpoints = checkpoints_for(output, outer);
        assert!(
            inner_checkpoints.len() >= 2,
            "`Inner` needs two checkpoints, found {}",
            inner_checkpoints.len()
        );
        assert!(
            outer_checkpoints.len() >= 2,
            "`Outer` needs two checkpoints, found {}",
            outer_checkpoints.len()
        );
        let hooks = StopHooks {
            outer_before: outer_checkpoints[0].clone(),
            inner_first: inner_checkpoints[0].clone(),
            inner_second: inner_checkpoints[1].clone(),
            outer_after: outer_checkpoints[1].clone(),
        };
        let inner_symbol = match flow {
            StopFlow::Nested | StopFlow::Mismatch => "stc_inner",
            StopFlow::Pause => "stc_pause",
            StopFlow::Recursive(_) => "stc_rec",
        };
        let config = StopCConfig {
            inner_index: descriptor_index(&artifacts, inner),
            outer_index: descriptor_index(&artifacts, outer),
            inner_symbol: inner_symbol.to_owned(),
            hooks: hooks.clone(),
            flow,
        };
        let text = c_source(&artifacts, Behavior::default(), Some(&config));
        let (module_path, manifest_path, directory) = compile(name, &text, &artifacts)?;

        let metadata_path = stc_runtime::module_info::metadata_path(&module_path);
        fs::write(&metadata_path, output.metadata.to_json().render_pretty())
            .expect("write metadata sidecar");
        let build_id = artifacts.manifest.header.build_id;
        let frames_path = stc_runtime::module_info::frames_path(&module_path);
        fs::write(
            &frames_path,
            frame_sidecar(
                &artifacts,
                inner,
                outer,
                inner_symbol,
                "stc_outer",
                checkpoint_count,
                &hooks,
            ),
        )
        .expect("write frame sidecar");

        Some(Self {
            module_path,
            manifest_path,
            artifacts,
            inner,
            outer,
            inner_symbol: inner_symbol.to_owned(),
            outer_symbol: "stc_outer".to_owned(),
            hooks,
            checkpoint_count,
            build_id,
            directory,
        })
    }

    /// Returns the synthetic frame sidecar path.
    #[must_use]
    pub fn frames_path(&self) -> PathBuf {
        stc_runtime::module_info::frames_path(&self.module_path)
    }

    /// Returns the metadata sidecar path.
    #[must_use]
    pub fn metadata_path(&self) -> PathBuf {
        stc_runtime::module_info::metadata_path(&self.module_path)
    }

    /// Returns the fixture directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for StopFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn function_record<'a>(artifacts: &'a ModuleArtifacts, name: &str) -> &'a stc_got::FunctionRecord {
    let expected = format!("::{}", name.to_ascii_uppercase());
    artifacts
        .manifest
        .functions
        .iter()
        .find(|function| {
            let base = function.key.split('@').next().unwrap_or_default();
            base.ends_with(&expected)
        })
        .unwrap_or_else(|| {
            let keys: Vec<_> = artifacts
                .manifest
                .functions
                .iter()
                .map(|function| function.key.as_str())
                .collect();
            panic!("manifest function `{name}`; functions: {keys:?}")
        })
}

fn descriptor_index(artifacts: &ModuleArtifacts, got_id: GotId) -> usize {
    artifacts
        .descriptor
        .functions
        .iter()
        .position(|function| function.got_id == got_id)
        .unwrap_or_else(|| panic!("descriptor function {}", got_id.to_hex()))
}

fn checkpoints_for(output: &Output, function: GotId) -> Vec<StopCheckpoint> {
    let debug = output.metadata.debug.as_ref().expect("debug metadata");
    let spans = &output.metadata.spans;
    debug
        .checkpoints
        .iter()
        .filter(|checkpoint| GotId::parse(&checkpoint.function).ok() == Some(function))
        .map(|checkpoint| {
            let entry = &spans.entries[usize::try_from(checkpoint.span).expect("span index")];
            let file = spans.files[usize::try_from(entry.file).expect("file index")].clone();
            StopCheckpoint {
                id: checkpoint.id,
                span: checkpoint.span,
                line: checkpoint.line,
                column: checkpoint.column,
                file,
            }
        })
        .collect()
}

fn frame_sidecar(
    artifacts: &ModuleArtifacts,
    inner: GotId,
    outer: GotId,
    inner_symbol: &str,
    outer_symbol: &str,
    checkpoint_count: u32,
    hooks: &StopHooks,
) -> String {
    let builtin = |name: &str| {
        artifacts
            .manifest
            .types
            .iter()
            .find(|record| record.key == name)
            .map_or_else(|| name.to_owned(), |record| record.type_key.clone())
    };
    let dint = builtin("DINT");
    let local = |name: &str, offset: u32, first: u32, last: u32| {
        Json::object([
            (
                "live".into(),
                Json::object([
                    ("first".into(), Json::Number(i128::from(first))),
                    ("last".into(), Json::Number(i128::from(last))),
                ]),
            ),
            ("location".into(), Json::string(format!("fp-{offset}"))),
            ("name".into(), Json::string(name)),
            ("size".into(), Json::Number(4)),
            ("type".into(), Json::string(dint.clone())),
        ])
    };
    let function = |got_id: GotId, symbol: &str, locals: Vec<Json>| {
        Json::object([
            ("cfa".into(), Json::string("fp+16")),
            ("frame_size".into(), Json::Number(64)),
            ("function".into(), Json::string(got_id.to_hex())),
            ("high_pc".into(), Json::Number(512)),
            ("locals".into(), Json::Array(locals)),
            ("low_pc".into(), Json::Number(0)),
            ("return_address".into(), Json::Number(-8)),
            ("saved_fp".into(), Json::Number(-16)),
            ("symbol".into(), Json::string(symbol)),
        ])
    };
    let mut functions = vec![
        (
            inner,
            function(
                inner,
                inner_symbol,
                vec![
                    local("A", 4, 1, checkpoint_count),
                    local("B", 8, 1, checkpoint_count),
                    // Described but not live at the inner hooks: `frame.read`
                    // must report it as unavailable rather than guessing.
                    local("C", 12, hooks.outer_before.id, hooks.outer_after.id),
                ],
            ),
        ),
        (
            outer,
            function(
                outer,
                outer_symbol,
                vec![
                    local("CA", 4, 1, checkpoint_count),
                    local("CB", 8, 1, checkpoint_count),
                ],
            ),
        ),
    ];
    functions.sort_by_key(|(got_id, _)| *got_id);
    Json::object([
        (
            "build_id".into(),
            Json::string(artifacts.manifest.header.build_id.to_hex()),
        ),
        ("format".into(), Json::Number(1)),
        (
            "functions".into(),
            Json::Array(functions.into_iter().map(|(_, value)| value).collect()),
        ),
        ("pointer_width".into(), Json::Number(8)),
        (
            "target".into(),
            Json::string(artifacts.manifest.header.target_triple.clone()),
        ),
    ])
    .render()
}

struct StopCConfig {
    inner_index: usize,
    outer_index: usize,
    inner_symbol: String,
    hooks: StopHooks,
    flow: StopFlow,
}

fn c_source(artifacts: &ModuleArtifacts, behavior: Behavior, stop: Option<&StopCConfig>) -> String {
    let descriptor = &artifacts.descriptor_bytes;
    let function_count = artifacts.descriptor.functions.len();
    let array_length = function_count.max(1);
    let mut source = String::new();
    source.push_str("#include <stdint.h>\n#include <stddef.h>\n");
    source.push_str("#if defined(_WIN32)\n");
    source.push_str("int _DllMainCRTStartup(void* a, unsigned long b, void* c) {\n");
    source.push_str("  (void)a; (void)b; (void)c; return 1;\n}\n");
    source.push_str("#define STC_EXPORT __declspec(dllexport)\n");
    source.push_str("#else\n");
    source.push_str("#define STC_EXPORT __attribute__((visibility(\"default\")))\n");
    source.push_str("#endif\n");
    source.push_str("#define STC_ALIGN __attribute__((noinline, aligned(4096)))\n");
    source.push_str("static const unsigned char stc_descriptor[] = {");
    for (index, byte) in descriptor.iter().enumerate() {
        if index > 0 {
            source.push(',');
        }
        source.push_str(&format!("0x{byte:02x}"));
    }
    source.push_str("};\n");
    source.push_str("static uint32_t stc_cycle_count = 0;\n");
    source.push_str("static const void* stc_api = 0;\n");
    source.push_str("static volatile int32_t stc_sink = 0;\n");
    source.push_str("static volatile int32_t stc_depth = 0;\n");
    source.push_str(
        "typedef struct {\n\
           uint32_t abi_version;\n\
           uint32_t struct_size;\n\
           void (*trap)(int32_t, uint32_t);\n\
           int64_t (*cycle_now_ns)(void);\n\
           uint64_t (*monotonic_now_ns)(void);\n\
           int64_t (*local_date_ns)(void);\n\
           void* (*alloc)(uint64_t, uint64_t);\n\
           void (*free_)(void*);\n\
           uint8_t* (*io_image)(uint32_t);\n\
           uint64_t (*io_size)(uint32_t);\n\
           int32_t (*state_load)(uint8_t*, int64_t, int64_t);\n\
           int32_t (*state_save)(const uint8_t*, int64_t, int64_t);\n\
           void (*assert_)(int32_t, int32_t, uint32_t);\n\
           void (*set_cycle_time_ns)(int64_t);\n\
           void (*debug_check)(uint32_t, uint32_t);\n\
         } stc_api_prefix;\n",
    );
    source.push_str(
        "#define STC_HOOK(cp, span) do { if (stc_api != 0) { \
         ((const stc_api_prefix*)stc_api)->debug_check((uint32_t)(cp), (uint32_t)(span)); \
         } } while (0)\n",
    );
    if let Some(config) = stop {
        let inner = &config.inner_symbol;
        source.push_str(&format!(
            "static void STC_ALIGN {inner}(void);\nstatic void STC_ALIGN stc_outer(void);\n"
        ));
        match config.flow {
            StopFlow::Nested => {
                source.push_str(&format!(
                    r#"
static void STC_ALIGN {inner}(void) {{
  volatile int32_t stc_a = 11;
  volatile int32_t stc_b = 22;
  STC_HOOK({i1}, {i1s});
  STC_HOOK({i2}, {i2s});
  stc_sink = stc_a + stc_b;
}}
static void STC_ALIGN stc_outer(void) {{
  volatile int32_t stc_c = 33;
  volatile int32_t stc_d = 44;
  STC_HOOK({o1}, {o1s});
  {inner}();
  STC_HOOK({o2}, {o2s});
  stc_sink = stc_c + stc_d;
}}
"#,
                    i1 = config.hooks.inner_first.id,
                    i1s = config.hooks.inner_first.span,
                    i2 = config.hooks.inner_second.id,
                    i2s = config.hooks.inner_second.span,
                    o1 = config.hooks.outer_before.id,
                    o1s = config.hooks.outer_before.span,
                    o2 = config.hooks.outer_after.id,
                    o2s = config.hooks.outer_after.span,
                ));
            }
            StopFlow::Pause => {
                source.push_str(&format!(
                    r#"
static void STC_ALIGN {inner}(void) {{
  volatile int32_t stc_a = 11;
  volatile int32_t stc_b = 22;
  uint64_t stc_start = stc_api == 0 ? 0
      : ((const stc_api_prefix*)stc_api)->monotonic_now_ns();
  while (stc_api != 0
         && ((const stc_api_prefix*)stc_api)->monotonic_now_ns() - stc_start
                < 750000000ULL) {{ }}
  STC_HOOK({i1}, {i1s});
  STC_HOOK({i2}, {i2s});
  stc_sink = stc_a + stc_b;
}}
static void STC_ALIGN stc_outer(void) {{
  volatile int32_t stc_c = 33;
  volatile int32_t stc_d = 44;
  stc_sink = stc_c + stc_d;
}}
"#,
                    i1 = config.hooks.inner_first.id,
                    i1s = config.hooks.inner_first.span,
                    i2 = config.hooks.inner_second.id,
                    i2s = config.hooks.inner_second.span,
                ));
            }
            StopFlow::Recursive(depth) => {
                source.push_str(&format!(
                    r#"
static void STC_ALIGN {inner}(void) {{
  volatile int32_t stc_a = 11;
  volatile int32_t stc_b = 22;
  if (stc_depth > 0) {{
    stc_depth -= 1;
    {inner}();
  }} else {{
    STC_HOOK({i1}, {i1s});
  }}
  stc_sink = stc_a + stc_b;
}}
static void STC_ALIGN stc_outer(void) {{
  volatile int32_t stc_c = 33;
  volatile int32_t stc_d = 44;
  stc_sink = stc_c + stc_d;
}}
"#,
                    i1 = config.hooks.inner_first.id,
                    i1s = config.hooks.inner_first.span,
                ));
                source.push_str(&format!(
                    "static const uint32_t stc_rec_depth = {depth}u;\n"
                ));
            }
            StopFlow::Mismatch => {
                source.push_str(&format!(
                    r#"
static void STC_ALIGN {inner}(void) {{
  volatile int32_t stc_a = 11;
  volatile int32_t stc_b = 22;
  STC_HOOK({i1}, {mismatch_span});
  stc_sink = stc_a + stc_b;
}}
static void STC_ALIGN stc_outer(void) {{
  volatile int32_t stc_c = 33;
  volatile int32_t stc_d = 44;
  STC_HOOK({o1}, {o1s});
  stc_sink = stc_c + stc_d;
}}
"#,
                    i1 = config.hooks.inner_first.id,
                    mismatch_span = config.hooks.inner_first.span.wrapping_add(1),
                    o1 = config.hooks.outer_before.id,
                    o1s = config.hooks.outer_before.span,
                ));
            }
        }
    }
    for index in 0..array_length {
        if let Some(config) = stop
            && (index == config.inner_index || index == config.outer_index)
        {
            continue;
        }
        source.push_str(&format!(
            "static void STC_ALIGN stc_fn_{index}(void) {{}}\n"
        ));
    }
    source.push_str(&format!(
        "static const void* const stc_functions[{array_length}] = {{"
    ));
    for index in 0..array_length {
        if index > 0 {
            source.push(',');
        }
        let name = if let Some(config) = stop {
            if index == config.inner_index {
                config.inner_symbol.clone()
            } else if index == config.outer_index {
                "stc_outer".to_owned()
            } else {
                format!("stc_fn_{index}")
            }
        } else {
            format!("stc_fn_{index}")
        };
        source.push_str(&format!("(const void*){name}"));
    }
    source.push_str("};\n");
    let cycle_body = match stop.map(|config| config.flow) {
        Some(StopFlow::Nested) => "  stc_outer();\n".to_owned(),
        Some(StopFlow::Pause) => "  stc_pause();\n".to_owned(),
        Some(StopFlow::Recursive(_)) => {
            "  stc_depth = (int32_t)(stc_rec_depth - 1u);\n  stc_rec();\n".to_owned()
        }
        Some(StopFlow::Mismatch) => "  stc_inner();\n  stc_outer();\n".to_owned(),
        None => String::new(),
    };
    source.push_str(&format!(
        r#"
STC_EXPORT const uint8_t* stc_module_descriptor(uint64_t* out_size) {{
  *out_size = sizeof(stc_descriptor);
  return stc_descriptor;
}}
STC_EXPORT const void* const* stc_module_functions(uint32_t* out_count) {{
  *out_count = {function_count}u;
  return stc_functions;
}}
STC_EXPORT int32_t stc_module_attach(const void* api, void* const* slots,
                                     uint32_t slot_count, uint64_t generation) {{
  (void)slots; (void)slot_count; (void)generation;
  stc_api = api;
  return {attach};
}}
STC_EXPORT int32_t stc_module_prepare(uint32_t mode, uint64_t generation,
                                      const uint8_t* fresh_mask, uint32_t mask_size) {{
  (void)mode; (void)generation; (void)fresh_mask; (void)mask_size;
  if (stc_api != 0 && {prepare_alloc_bytes}u > 0) {{
    ((const stc_api_prefix*)stc_api)->alloc({prepare_alloc_bytes}u, 8);
  }}
  return {prepare};
}}
STC_EXPORT int32_t STC_ALIGN stc_module_cycle(uint64_t generation) {{
  (void)generation;
  stc_cycle_count += 1;
  if (stc_cycle_count <= {cycle_failures}u) {{
    return 1;
  }}
{cycle_body}  return 0;
}}
STC_EXPORT int32_t stc_module_shutdown(uint32_t mode) {{
  (void)mode;
  return {shutdown};
}}
STC_EXPORT void stc_module_detach(uint64_t generation) {{
  (void)generation;
}}
"#,
        attach = behavior.attach,
        prepare = behavior.prepare,
        prepare_alloc_bytes = behavior.prepare_alloc_bytes,
        cycle_failures = behavior.cycle_failures,
        shutdown = behavior.shutdown,
    ));
    source
}
