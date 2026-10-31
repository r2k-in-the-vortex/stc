//! M6.5 phase-2 checkpoint table tests: eligibility, dense ordering, golden
//! snapshots, determinism, and the debug/release/test descriptor contract.

mod common;

use std::path::Path;

use common::clean_hir;
use stc_codegen::{DebugCheckpointMetadata, Json, Mode, Options};
use stc_got::descriptor::{entry_flag, feature};
use stc_got::{Descriptor, ImportService};
use stc_span::SourceMap;

const SOURCE: &str = r#"FUNCTION F : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
  k : DINT;
END_VAR
  r := 0;
  IF n > 0 THEN
    ;
    r := 1;
  ELSIF n < 0 THEN
    r := 2;
  ELSE
    r := 3;
  END_IF;
  CASE n OF
    0: r := 4;
    1: r := 5;
  ELSE
    r := 6;
  END_CASE;
  WHILE r < 10 DO
    r := r + 1;
  END_WHILE;
  FOR k := 0 TO 2 DO
    r := r + k;
  END_FOR;
  F := r;
  RETURN;
  r := 99;
END_FUNCTION

FUNCTION_BLOCK FB
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
END_FUNCTION_BLOCK

CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id + 1;
END_METHOD
END_CLASS

PROGRAM Main
VAR
  n : DINT;
  fb : FB;
  s : Shape;
  t : TON;
  total : DINT;
END_VAR
  total := F(1);
  fb(inc := total);
  n := s.Area();
  t(IN := TRUE, PT := T#1s);
  IF n > 0 THEN
    total := total + n;
  END_IF;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const EMPTY: &str = r#"PROGRAM Main
VAR
  n : DINT;
END_VAR
END_PROGRAM
"#;

#[derive(Clone, Copy)]
struct BuildForm {
    debug: bool,
    release: bool,
    test_build: bool,
}

const DEBUG: BuildForm = BuildForm {
    debug: true,
    release: false,
    test_build: false,
};
const RELEASE: BuildForm = BuildForm {
    // A release build wins even when a caller also sets the debug switch.
    debug: true,
    release: true,
    test_build: false,
};
const TEST: BuildForm = BuildForm {
    debug: true,
    release: false,
    test_build: true,
};

fn line_of(source: &str, needle: &str) -> usize {
    let offset = source
        .find(needle)
        .unwrap_or_else(|| panic!("source does not contain `{needle}`"));
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn generate(source: &str, form: BuildForm) -> stc_codegen::Output {
    let hir = clean_hir(source);
    let mut source_map = SourceMap::new();
    source_map.add_file("src/main.st", source);
    let sources = vec!["src/main.st".to_string()];
    let options = Options {
        artifact_name: "app",
        target: Some("x86_64-unknown-linux-gnu"),
        debug: form.debug,
        emit_metadata: true,
        mode: Mode::Executable,
        relaxed: false,
        release: form.release,
        sources: &sources,
        source_paths: Some(&sources),
        source_map: Some(&source_map),
        module: stc_codegen::ModuleBuildOptions::default(),
        project: stc_codegen::ProjectOptions::default(),
        test_build: form.test_build,
        coverage: None,
    };
    stc_codegen::generate(&hir, &options)
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
}

fn debug_of(output: &stc_codegen::Output) -> &stc_codegen::DebugMetadata {
    output
        .metadata
        .debug
        .as_ref()
        .expect("the debug object is present")
}

fn checkpoint_at(
    checkpoints: &[DebugCheckpointMetadata],
    line: usize,
) -> Option<&DebugCheckpointMetadata> {
    let line = u32::try_from(line).expect("line fits");
    checkpoints
        .iter()
        .find(|checkpoint| checkpoint.line == line)
}

fn debug_json(output: &stc_codegen::Output) -> Json {
    let root = output.metadata.to_json();
    let Json::Object(values) = root else {
        panic!("metadata JSON is not an object");
    };
    values.get("debug").cloned().expect("debug object")
}

fn snapshot_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join("checkpoints.json")
}

#[test]
fn checkpoint_golden_snapshot() {
    let output = generate(SOURCE, DEBUG);
    let path = snapshot_path();
    if std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshot directory"))
            .expect("create snapshot directory");
        std::fs::write(&path, debug_json(&output).render_pretty()).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot; run with STC_UPDATE_SNAPSHOTS=1"));
    assert_eq!(expected, debug_json(&output).render_pretty());
}

#[test]
fn checkpoints_cover_the_documented_eligibility_rules() {
    let output = generate(SOURCE, DEBUG);
    let debug = debug_of(&output);
    assert!(debug.hooks);
    assert!(!debug.checkpoints.is_empty());

    let unreachable_line = line_of(SOURCE, "r := 99;");
    let empty_line = SOURCE
        .lines()
        .position(|line| line.trim() == ";")
        .map(|index| index + 1)
        .expect("the fixture contains an empty statement");
    for checkpoint in &debug.checkpoints {
        assert_ne!(
            usize::try_from(checkpoint.line).expect("line fits"),
            unreachable_line,
            "unreachable code is not checkpoint-eligible"
        );
        assert_ne!(
            usize::try_from(checkpoint.line).expect("line fits"),
            empty_line,
            "empty statements are not checkpoint-eligible"
        );
        assert_ne!(checkpoint.line, 0, "line numbers are 1-based");
        assert_ne!(checkpoint.column, 0, "columns are 1-based");
        assert!(
            matches!(checkpoint.flags, 1 | 3 | 4 | 8),
            "unknown checkpoint flags {:#x} at line {}",
            checkpoint.flags,
            checkpoint.line
        );
    }

    let flags: Vec<u32> = debug
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.flags)
        .collect();
    assert!(
        flags.contains(&1),
        "top-level statements are statement-only entries: {flags:?}"
    );
    assert!(
        flags.iter().any(|flags| flags & 2 != 0),
        "the first statement of each control-flow block is a block entry"
    );
    assert!(
        flags.iter().any(|flags| flags & 4 != 0),
        "calls have call-site checkpoints"
    );
    assert!(
        flags.iter().any(|flags| flags & 8 != 0),
        "loops have back-edge checkpoints"
    );

    // The empty statement precedes the first executable statement of the IF
    // arm; BLOCK belongs to the assignment, not the empty statement.
    let block_line = line_of(SOURCE, "r := 1;");
    let block = checkpoint_at(&debug.checkpoints, block_line).expect("block-entry checkpoint");
    assert_eq!(block.flags, 1 | 2, "line {block_line}");

    for needle in [
        "total := F(1);",
        "fb(inc := total);",
        "n := s.Area();",
        "t(IN := TRUE, PT := T#1s);",
    ] {
        let line = line_of(SOURCE, needle);
        let found = debug.checkpoints.iter().any(|checkpoint| {
            usize::try_from(checkpoint.line).expect("line fits") == line
                && checkpoint.flags & 4 != 0
        });
        assert!(found, "missing call-site checkpoint at line {line}");
    }

    // Synthetic standard-function bodies contribute no sites.
    for checkpoint in &debug.checkpoints {
        let function = output
            .got
            .manifest
            .functions
            .iter()
            .find(|function| function.got_id.to_hex() == checkpoint.function)
            .unwrap_or_else(|| panic!("unknown checkpoint function {}", checkpoint.function));
        assert_eq!(
            function.flags & stc_got::descriptor::function_flag::STD_FUNCTION,
            0,
            "synthetic standard-function body `{}` contributed a checkpoint",
            function.key
        );
    }
}

#[test]
fn checkpoint_ids_are_dense_and_sorted_by_the_contract_key() {
    let output = generate(SOURCE, DEBUG);
    let debug = debug_of(&output);
    let spans: std::collections::BTreeMap<u32, (u32, u32)> = output
        .spans
        .iter()
        .map(|span| (span.id, (span.start, span.end)))
        .collect();
    let mut keys: Vec<(String, u32, u32, u32)> = Vec::new();
    for (index, checkpoint) in debug.checkpoints.iter().enumerate() {
        assert_eq!(
            checkpoint.id,
            u32::try_from(index + 1).expect("checkpoint index fits"),
            "IDs are dense from 1"
        );
        let (start, end) = spans.get(&checkpoint.span).copied().unwrap_or_else(|| {
            panic!(
                "checkpoint span {} is not in the span table",
                checkpoint.span
            )
        });
        keys.push((checkpoint.function.clone(), start, end, checkpoint.flags));
    }
    assert!(
        keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "checkpoints do not follow (function, span start, span end, flags): {keys:?}"
    );
}

#[test]
fn checkpoint_output_is_deterministic_and_path_free() {
    let first = generate(SOURCE, DEBUG);
    let second = generate(SOURCE, DEBUG);
    assert_eq!(first.module, second.module);
    assert_eq!(
        first.metadata.to_json().render(),
        second.metadata.to_json().render()
    );
    assert_eq!(first.got.descriptor_bytes, second.got.descriptor_bytes);
    assert_eq!(first.got.manifest_binary, second.got.manifest_binary);
    let debug = debug_json(&first).render();
    assert!(!debug.contains("C:\\"), "{debug}");
    assert!(!debug.contains("/tmp/"), "{debug}");
    assert!(!debug.contains("src/main.st"), "{debug}");
}

fn assert_no_service_13(descriptor: &Descriptor) {
    assert!(
        !descriptor
            .slots
            .iter()
            .any(|slot| slot.service == ImportService::DebugCheck.id()),
        "release/hook-free builds do not import service 13"
    );
}

#[test]
fn debug_builds_import_the_checkpoint_service_and_set_both_hook_bits() {
    let output = generate(SOURCE, DEBUG);
    let debug = debug_of(&output);
    assert!(debug.hooks);
    assert_eq!(debug.format, 1);
    assert_eq!(debug.info, "dwarf");
    let descriptor = &output.got.descriptor;
    assert_eq!(descriptor.host_abi_minor, stc_got::DEBUG_ABI_MINOR);
    assert_eq!(descriptor.host_abi_minor, 3);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_ne!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert!(
        descriptor
            .slots
            .iter()
            .any(|slot| slot.service == ImportService::DebugCheck.id()),
        "debug builds import debug_check into a slot"
    );
    assert_eq!(output.got.manifest.header.generator_flags & 1, 1);
    assert_eq!(output.got.manifest.header.generator_flags & 2, 0);

    // The IR calls through the service slot with the metadata's IDs.
    let lines: Vec<&str> = output.module.lines().collect();
    for checkpoint in &debug.checkpoints {
        let needle = format!("(i32 {}, i32 {})", checkpoint.id, checkpoint.span);
        let index = lines
            .iter()
            .position(|line| line.contains("call void") && line.contains(&needle))
            .unwrap_or_else(|| {
                panic!(
                    "missing host call for checkpoint {} in:\n{}",
                    checkpoint.id, output.module
                )
            });
        assert!(
            lines[index - 1].contains("load ptr, ptr"),
            "the host call loads the service entry point from its import cell"
        );
        assert!(
            lines[index - 2].contains("load ptr, ptr"),
            "the host call first loads the import cell address from the slot table"
        );
        assert!(
            lines[index - 3].contains("getelementptr ptr, ptr")
                && lines[index - 3].contains(", i64 "),
            "the host call addresses the debug_check import slot"
        );
    }
}

#[test]
fn release_builds_are_hook_free_with_the_production_abi() {
    let output = generate(SOURCE, RELEASE);
    assert!(output.metadata.debug.is_none());
    assert!(!output.module.contains("!llvm.dbg"), "{}", output.module);
    let descriptor = &output.got.descriptor;
    assert_eq!(descriptor.host_abi_minor, stc_got::PRODUCTION_ABI_MINOR);
    assert_eq!(descriptor.host_abi_minor, 1);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_eq!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert_no_service_13(descriptor);
    assert_eq!(output.got.manifest.header.generator_flags & 1, 0);
    assert_eq!(output.got.manifest.header.generator_flags & 2, 2);

    let debug = generate(SOURCE, DEBUG);
    assert_ne!(
        output.got.manifest.header.build_id, debug.got.manifest.header.build_id,
        "debug and release have distinct build identities"
    );
    assert_ne!(
        output.got.manifest_digest, debug.got.manifest_digest,
        "debug and release have distinct manifests"
    );
}

#[test]
fn test_builds_stay_hook_free_at_abi_2_2() {
    let output = generate(SOURCE, TEST);
    let debug = debug_of(&output);
    assert!(!debug.hooks);
    assert!(debug.checkpoints.is_empty());
    assert_eq!(debug.info, "dwarf");
    let descriptor = &output.got.descriptor;
    assert_eq!(descriptor.host_abi_minor, stc_got::TEST_ABI_MINOR);
    assert_eq!(descriptor.host_abi_minor, 2);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_eq!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert_no_service_13(descriptor);
    assert_eq!(output.got.manifest.header.generator_flags & 1, 1);
}

#[test]
fn a_debug_build_with_no_sites_is_hook_free_but_still_a_debug_build() {
    let output = generate(EMPTY, DEBUG);
    let debug = debug_of(&output);
    assert!(!debug.hooks);
    assert!(debug.checkpoints.is_empty());
    let descriptor = &output.got.descriptor;
    assert_eq!(descriptor.host_abi_minor, stc_got::PRODUCTION_ABI_MINOR);
    assert_ne!(descriptor.entry_flags & entry_flag::DEBUG_BUILD, 0);
    assert_eq!(descriptor.entry_flags & entry_flag::DEBUG_HOOKS, 0);
    assert_eq!(descriptor.required_features & feature::DEBUG_HOOKS, 0);
    assert_no_service_13(descriptor);
    assert_eq!(output.got.manifest.header.generator_flags & 1, 1);
}
