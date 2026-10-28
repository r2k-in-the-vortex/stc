//! M6.5 phase-6 frame-hint tests: debug builds carry deterministic per-function
//! locals for the frame sidecar and force a walkable frame pointer; release and
//! test builds carry neither.

mod common;

use common::clean_hir;
use stc_codegen::{Mode, Options};
use stc_span::SourceMap;

const SOURCE: &str = r#"FUNCTION F : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
END_VAR
  r := n + 1;
  F := r;
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
  total : DINT;
END_VAR
  total := F(1);
  fb(inc := total);
  n := s.Area();
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
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
    debug: true,
    release: true,
    test_build: false,
};
const TEST: BuildForm = BuildForm {
    debug: true,
    release: false,
    test_build: true,
};

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
    };
    stc_codegen::generate(&hir, &options)
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
}

fn define_line<'a>(module: &'a str, symbol: &str) -> &'a str {
    let needle = format!(" @{symbol}(");
    module
        .lines()
        .find(|line| line.starts_with("define ") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no define for `{symbol}`"))
}
#[test]
fn debug_builds_emit_sorted_frame_hints_with_checkpoint_ranges() {
    let output = generate(SOURCE, DEBUG);
    assert!(!output.frames.is_empty(), "debug build has frame hints");
    let mut sorted = output.frames.clone();
    sorted.sort_by_key(|frame| frame.function);
    assert_eq!(output.frames, sorted, "hints are ordered by function id");
    for frame in &output.frames {
        assert_eq!(
            frame
                .locals
                .iter()
                .map(|local| local.declaration)
                .collect::<Vec<_>>(),
            {
                let mut declarations = frame
                    .locals
                    .iter()
                    .map(|local| local.declaration)
                    .collect::<Vec<_>>();
                declarations.sort_unstable();
                declarations
            },
            "locals keep declaration order: {frame:#?}"
        );
        for local in &frame.locals {
            assert_eq!(local.name, local.name.to_ascii_uppercase());
            assert!(!local.type_key.is_empty());
            assert!(local.size > 0);
            assert!(local.first <= local.last || (local.first == 0 && local.last == 0));
        }
    }
    let checkpoints = output
        .metadata
        .debug
        .as_ref()
        .expect("debug metadata")
        .checkpoints
        .clone();
    let mut expected: std::collections::BTreeMap<String, (u32, u32)> =
        std::collections::BTreeMap::new();
    for checkpoint in &checkpoints {
        let range = expected
            .entry(checkpoint.function.clone())
            .or_insert((checkpoint.id, checkpoint.id));
        range.0 = range.0.min(checkpoint.id);
        range.1 = range.1.max(checkpoint.id);
    }
    for frame in &output.frames {
        let function = frame.function.to_string();
        for local in &frame.locals {
            if let Some((first, last)) = expected.get(&function) {
                assert_eq!((local.first, local.last), (*first, *last), "{frame:#?}");
            }
        }
    }
}

#[test]
fn debug_builds_force_frame_pointers_and_disable_tail_calls() {
    let output = generate(SOURCE, DEBUG);
    for frame in &output.frames {
        let line = define_line(&output.module, &frame.symbol);
        assert!(
            line.contains("\"frame-pointer\"=\"all\""),
            "missing frame pointer on {line}"
        );
        assert!(
            line.contains("\"disable-tail-calls\"=\"true\""),
            "tail calls enabled on {line}"
        );
    }
}

#[test]
fn release_and_test_builds_omit_frame_hints_and_attributes() {
    for form in [RELEASE, TEST] {
        let output = generate(SOURCE, form);
        assert!(
            output.frames.is_empty(),
            "no frame hints for a non-debug build"
        );
        assert!(
            !output.module.contains("\"frame-pointer\""),
            "no forced frame pointer outside debug builds"
        );
        assert!(
            !output.module.contains("\"disable-tail-calls\""),
            "no tail-call attribute outside debug builds"
        );
    }
}
