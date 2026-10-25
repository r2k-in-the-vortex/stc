//! Test-only helpers that build real module artifacts through the compiler.

use stc_codegen::module::{ModuleArtifacts, ModuleOptions, project, source_fingerprint};
use stc_diag::DiagnosticSink;
use stc_got::{Descriptor, Endianness, Manifest, Rename, Result, Target};
use stc_hir::HirCrate;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_sema::analyze;
use stc_span::FileId;

/// A simple scalar program with persistent roots.
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

/// A source-compatible variant of [`SIMPLE`] with a different body and digest.
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

/// An OOP surface with class/FB types, descriptors, and hidden roots.
pub const OOP: &str = r#"
CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS

FUNCTION_BLOCK MyCounter
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
END_FUNCTION_BLOCK

VAR_GLOBAL
  frame : Shape;
  counter : MyCounter;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  counter(inc := 1);
END_PROGRAM
"#;

/// Returns the fixed test target.
#[must_use]
pub fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

/// Builds artifacts without a baseline.
#[must_use]
pub fn artifacts(source: &str, name: &str) -> ModuleArtifacts {
    build(source, name, None, false, Vec::new(), false)
}

/// Builds artifacts with an optional baseline.
#[must_use]
pub fn artifacts_with(source: &str, name: &str, baseline: Manifest, cold: bool) -> ModuleArtifacts {
    build(source, name, Some(baseline), cold, Vec::new(), false)
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
    )
}

fn build(
    source: &str,
    name: &str,
    baseline: Option<Manifest>,
    cold: bool,
    renames: Vec<Rename>,
    allow_untracked_removal: bool,
) -> ModuleArtifacts {
    let hir = clean_hir(source);
    let sources = vec![format!("{name}.st")];
    let fingerprint = source_fingerprint(&[(format!("{name}.st"), source.as_bytes().to_vec())])
        .expect("source fingerprint");
    let mut options = ModuleOptions::new(target(), name, "stc 0.1.0", fingerprint);
    options.baseline = baseline.as_ref();
    options.cold = cold;
    options.renames = renames;
    options.allow_untracked_removal = allow_untracked_removal;
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

/// Recomputes the catalog and plan digests a mutation invalidated.
///
/// Test mutations deliberately bypass the compiler so the host validator can
/// be exercised on plausible but inconsistent artifacts.
pub fn reseal_manifest(manifest: &mut Manifest) -> Result<()> {
    manifest.header.catalog_digest = manifest.catalog_digest()?;
    manifest.header.plan_digest = manifest.plan_digest()?;
    Ok(())
}

/// Re-seals a descriptor/manifest pair after a test mutation.
///
/// Returns the encoded descriptor and binary manifest bytes.
pub fn reseal_pair(
    descriptor: &mut Descriptor,
    manifest: &mut Manifest,
) -> Result<(Vec<u8>, Vec<u8>)> {
    reseal_manifest(manifest)?;
    descriptor.manifest_digest = stc_got::Digest32::ZERO;
    manifest.header.descriptor_digest = descriptor.digest()?;
    descriptor.manifest_digest = manifest.digest()?;
    Ok((descriptor.encode()?, manifest.encode_binary()?))
}
