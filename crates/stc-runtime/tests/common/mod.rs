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
use stc_diag::DiagnosticSink;
use stc_got::{Endianness, Manifest, Rename, Target};
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::analyze;
use stc_span::FileId;

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
        let clang = clang()?;
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "stc-runtime-host-{}-{id}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create fixture directory");

        let source_path = directory.join("fixture.c");
        fs::write(&source_path, c_source(&artifacts, behavior)).expect("write fixture source");
        let module_path = if cfg!(windows) {
            directory.join(format!("{name}.dll"))
        } else {
            directory.join(format!("lib{name}.so"))
        };
        let mut command = Command::new(clang);
        command
            .arg("-shared")
            .arg("-O0")
            .arg("-fno-stack-protector");
        if cfg!(windows) {
            command.arg("-nostdlib");
        } else {
            command.arg("-fPIC");
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

fn c_source(artifacts: &ModuleArtifacts, behavior: Behavior) -> String {
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
    source.push_str(
        "typedef struct {\n\
           uint32_t abi_version;\n\
           uint32_t struct_size;\n\
           void (*trap)(int32_t, uint32_t);\n\
           int64_t (*cycle_now_ns)(void);\n\
           uint64_t (*monotonic_now_ns)(void);\n\
           int64_t (*local_date_ns)(void);\n\
           void* (*alloc)(uint64_t, uint64_t);\n\
         } stc_api_prefix;\n",
    );
    for index in 0..array_length {
        source.push_str(&format!("static void stc_fn_{index}(void) {{}}\n"));
    }
    source.push_str(&format!(
        "static const void* const stc_functions[{array_length}] = {{"
    ));
    for index in 0..array_length {
        if index > 0 {
            source.push(',');
        }
        source.push_str(&format!("(const void*)stc_fn_{index}"));
    }
    source.push_str("};\n");
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
STC_EXPORT int32_t stc_module_cycle(uint64_t generation) {{
  (void)generation;
  stc_cycle_count += 1;
  if (stc_cycle_count <= {cycle_failures}u) {{
    return 1;
  }}
  return 0;
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
