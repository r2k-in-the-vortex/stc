mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::clean_hir;
use stc_codegen::{Mode, Options};
use stc_span::SourceMap;

const SOURCE: &str = r#"
TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS

FUNCTION_BLOCK FB
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
END_FUNCTION_BLOCK

VAR_GLOBAL
  shared : Shape;
  gpoint : Point;
  garr : ARRAY[1..2] OF DINT;
  gstr : STRING[8];
END_VAR

FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
VAR
  sum : DINT;
END_VAR
  sum := a + b;
  Add := sum;
END_FUNCTION

PROGRAM Main
VAR RETAIN
  held : Shape;
END_VAR
VAR
  fb : FB;
  count : DINT;
  p : Point;
  arr : ARRAY[1..3] OF DINT;
  str : STRING[10];
END_VAR
  held.id := held.id + 1;
  p.x := held.id;
  p.y := 2;
  arr[1] := p.x + p.y;
  str := 'abc';
  count := Add(a := arr[1], b := p.y);
  fb(inc := count);
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn line_of(needle: &str) -> usize {
    let offset = SOURCE
        .find(needle)
        .unwrap_or_else(|| panic!("source does not contain `{needle}`"));
    SOURCE[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn absolute_source_path() -> &'static str {
    if cfg!(windows) {
        r"C:\host-only\package\src\main.st"
    } else {
        "/host-only/package/src/main.st"
    }
}

fn generate_ir(
    source: &str,
    debug: bool,
    target: &str,
    source_map_path: &str,
    source_paths: Option<&[String]>,
) -> String {
    let hir = clean_hir(source);
    let mut source_map = SourceMap::new();
    source_map.add_file(source_map_path, source);
    let sources = vec!["src/main.st".to_string()];
    let options = Options {
        artifact_name: "app",
        target: Some(target),
        debug,
        emit_metadata: true,
        mode: Mode::Executable,
        relaxed: false,
        release: false,
        sources: &sources,
        source_paths,
        source_map: Some(&source_map),
        module: stc_codegen::ModuleBuildOptions::default(),
        project: stc_codegen::ProjectOptions::default(),
        test_build: false,
    };
    stc_codegen::generate(&hir, &options)
        .unwrap_or_else(|errors| panic!("codegen failed: {errors:#?}"))
        .module
}

fn source_paths() -> Vec<String> {
    vec!["src/main.st".to_string()]
}

#[test]
fn dwarf_metadata_covers_the_user_source_surface() {
    let paths = source_paths();
    let module = generate_ir(
        SOURCE,
        true,
        "x86_64-unknown-linux-gnu",
        absolute_source_path(),
        Some(&paths),
    );

    // Anchor and version metadata. DWARF 5 and debug-info version 3 match what
    // current clang accepts for textual IR.
    for expected in [
        "!llvm.dbg.cu = !{!0}",
        "!llvm.module.flags = !{!",
        "!\"Dwarf Version\", i32 5",
        "!\"Debug Info Version\", i32 3",
        "emissionKind: FullDebug",
        "language: DW_LANG_C99",
    ] {
        assert!(
            module.contains(expected),
            "missing `{expected}` in debug IR:\n{module}"
        );
    }

    // File identity is package-relative and forward-slashed. The SourceMap
    // path is deliberately host-absolute and must not leak into the module.
    assert_eq!(
        module
            .matches("!DIFile(filename: \"src/main.st\", directory: \".\")")
            .count(),
        1,
        "the primary file must be allocated exactly once:\n{module}"
    );
    assert!(!module.contains("host-only"), "{module}");
    assert!(!module.contains("C:\\"), "{module}");

    // Function definitions carry real definition lines, prototype types, and
    // a scope line taken from the body, not from span 0.
    let add_line = line_of("FUNCTION Add");
    let add_body_line = line_of("  sum := a + b;");
    let main_line = line_of("PROGRAM Main");
    let main_body_line = line_of("  held.id := held.id + 1;");
    let fb_line = line_of("FUNCTION_BLOCK FB");
    let fb_body_line = line_of("  cv := cv + inc;");
    let method_line = line_of("METHOD PUBLIC Area");
    let method_body_line = line_of("  Area := id;");
    for expected in [
        "!DISubprogram(name: \"ADD\", linkageName: \"stc_fn__ROOT_n_ADD_".to_string(),
        format!("line: {add_line}, type:"),
        format!("scopeLine: {add_body_line}"),
        "!DISubprogram(name: \"Main\", linkageName: \"stc_prog__ROOT_n_MAIN_body\"".to_string(),
        format!("line: {main_line}, type:"),
        format!("scopeLine: {main_body_line}"),
        "!DISubprogram(name: \"FB\", linkageName: \"stc_fb_body__ROOT_n_FB\"".to_string(),
        format!("line: {fb_line}, type:"),
        format!("scopeLine: {fb_body_line}"),
        "!DISubprogram(name: \"Shape.Area\"".to_string(),
        format!("line: {method_line}, type:"),
        format!("scopeLine: {method_body_line}"),
        format!("!DILocation(line: {main_body_line}, column: 3,"),
        format!("!DILocation(line: {add_body_line}, column: 3,"),
    ] {
        assert!(
            module.contains(&expected),
            "missing `{expected}` in debug IR:\n{module}"
        );
    }
    assert!(!module.contains("line: 0"), "{module}");
    assert!(!module.contains("column: 0"), "{module}");

    // Parameters and locals have real declaration lines and arg indexes.
    let a_line = line_of("  a : DINT;");
    let b_line = line_of("  b : DINT;");
    let sum_line = line_of("  sum : DINT;");
    for expected in [
        "!DILocalVariable(name: \"a\"".to_string(),
        "!DILocalVariable(name: \"b\"".to_string(),
        "!DILocalVariable(name: \"sum\"".to_string(),
        "arg: 1".to_string(),
        "arg: 2".to_string(),
        format!("line: {a_line}, type:"),
        format!("line: {b_line}, type:"),
        format!("line: {sum_line}, type:"),
    ] {
        assert!(
            module.contains(&expected),
            "missing `{expected}` in debug IR:\n{module}"
        );
    }
    assert_eq!(
        module.matches("call void @llvm.dbg.declare(").count(),
        3,
        "the three Add variables are declared exactly once:\n{module}"
    );
    assert!(
        module.contains("declare void @llvm.dbg.declare(metadata, metadata, metadata)"),
        "{module}"
    );
    assert!(
        module.contains("retainedNodes: !"),
        "parameters/locals are retained by their DISubprogram:\n{module}"
    );

    // Canonical scalar types.
    assert!(
        module.contains("!DIBasicType(name: \"DINT\", size: 32, encoding: DW_ATE_signed)"),
        "{module}"
    );
    assert!(
        module.contains("!DIBasicType(name: \"CHAR\", size: 8, encoding: DW_ATE_unsigned_char)"),
        "{module}"
    );

    // Aggregates, fields, interfaces, classes, enums, arrays, and strings.
    for expected in [
        "DW_TAG_structure_type, name: \"Point\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"x\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"y\"",
        "DW_TAG_class_type, name: \"Shape\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"id\"",
        "DW_TAG_structure_type, name: \"IShape\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"record\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"vtable\"",
        "DW_TAG_enumeration_type, name: \"Color\"",
        "!DIEnumerator(name: \"Red\", value: 0)",
        "!DIEnumerator(name: \"Green\", value: 5)",
        "!DIEnumerator(name: \"Blue\", value: 6)",
        "DW_TAG_array_type",
        "!DISubrange(count: 3, lowerBound: 1)",
        "name: \"STRING[10]\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"length\"",
        "!DIDerivedType(tag: DW_TAG_member, name: \"data\"",
        "identifier: \"stc:T:",
    ] {
        assert!(
            module.contains(expected),
            "missing `{expected}` in debug IR:\n{module}"
        );
    }

    // Persistent roots and program fields are declaration-only because their
    // storage is host-owned GOT memory, never a module global.
    for expected in [
        "!DIGlobalVariable(name: \"shared\", linkageName: \"stc_g__ROOT_n_SHARED\"",
        "!DIGlobalVariable(name: \"gpoint\", linkageName: \"stc_g__ROOT_n_GPOINT\"",
        "!DIGlobalVariable(name: \"garr\", linkageName: \"stc_g__ROOT_n_GARR\"",
        "!DIGlobalVariable(name: \"gstr\", linkageName: \"stc_g__ROOT_n_GSTR\"",
        "!DIGlobalVariable(name: \"Main.held\"",
        "!DIGlobalVariable(name: \"Main.FB\"",
        "!DIGlobalVariable(name: \"Main.count\"",
        "!DIGlobalVariable(name: \"Main.p\"",
        "!DIGlobalVariable(name: \"Main.arr\"",
        "!DIGlobalVariable(name: \"Main.str\"",
        "isDefinition: false",
        "!DIGlobalVariableExpression(var:",
        "!DIExpression()",
    ] {
        assert!(
            module.contains(expected),
            "missing `{expected}` in debug IR:\n{module}"
        );
    }

    // Unused declarations cannot be described without inventing a type ID;
    // they are omitted rather than emitted with bogus metadata.
    assert!(!module.contains("name: \"Small\""), "{module}");
}

#[test]
fn codeview_metadata_is_selected_for_windows_targets() {
    let paths = source_paths();
    let module = generate_ir(
        SOURCE,
        true,
        "x86_64-pc-windows-msvc",
        absolute_source_path(),
        Some(&paths),
    );
    for expected in [
        "!\"Debug Info Version\", i32 3",
        "!\"wchar_size\", i32 2",
        "!\"CodeView\", i32 1",
        "!DISubprogram(name: \"ADD\"",
        "!DILocalVariable(name: \"sum\"",
        "line: 42, type:",
        "DW_TAG_class_type, name: \"Shape\"",
        "identifier: \"stc:T:",
    ] {
        let expected = expected.replace("42", &line_of("FUNCTION Add").to_string());
        assert!(
            module.contains(&expected),
            "missing `{expected}` in CodeView-target IR:\n{module}"
        );
    }
    assert!(!module.contains("Dwarf Version"), "{module}");

    // CodeView is the native format for MSVC targets. Windows GNU targets use
    // DWARF, matching clang's target-specific default.
    let paths = source_paths();
    let gnu = generate_ir(
        SOURCE,
        true,
        "x86_64-pc-windows-gnu",
        absolute_source_path(),
        Some(&paths),
    );
    assert!(gnu.contains("!\"Dwarf Version\", i32 5"), "{gnu}");
    assert!(!gnu.contains("!\"CodeView\""), "{gnu}");
}

#[test]
fn debug_disabled_emits_no_debug_metadata_and_is_byte_identical() {
    let paths = source_paths();
    let first = generate_ir(
        SOURCE,
        false,
        "x86_64-unknown-linux-gnu",
        absolute_source_path(),
        Some(&paths),
    );
    let second = generate_ir(
        SOURCE,
        false,
        "x86_64-unknown-linux-gnu",
        "/another/host/package/src/main.st",
        Some(&paths),
    );
    assert_eq!(first, second, "debug-disabled IR must be deterministic");
    for marker in [
        "!llvm.dbg.cu",
        "!llvm.module.flags",
        "!DIFile",
        "!DISubprogram",
        "!DILocalVariable",
        "!DIGlobalVariable",
        "!dbg",
        "llvm.dbg.declare",
        "; debug metadata",
    ] {
        assert!(
            !first.contains(marker),
            "debug-disabled IR contains `{marker}`:\n{first}"
        );
    }
}

#[test]
fn debug_paths_are_normalized_and_deterministic() {
    let paths = vec!["src\\nested\\main.st".to_string()];
    let first = generate_ir(
        SOURCE,
        true,
        "x86_64-unknown-linux-gnu",
        "C:\\host-a\\src\\main.st",
        Some(&paths),
    );
    let second = generate_ir(
        SOURCE,
        true,
        "x86_64-unknown-linux-gnu",
        "C:\\host-b\\src\\main.st",
        Some(&paths),
    );
    assert_eq!(
        first, second,
        "debug metadata depends on an output host path"
    );
    assert!(
        first.contains("!DIFile(filename: \"src/nested/main.st\", directory: \".\")"),
        "{first}"
    );
    for marker in ["host-a", "host-b", "C:\\", "2026-"] {
        assert!(!first.contains(marker), "`{marker}` leaked into:\n{first}");
    }
}

fn tool_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tool_output(program: &str, args: &[&OsStr]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("cannot run {program}: {error}"));
    assert!(
        output.status.success(),
        "{program} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn contains(haystack: &str, needles: &[&str], context: &str) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "{context} inspection is missing `{needle}`:\n{haystack}"
        );
    }
}

fn compile_object(
    clang: &str,
    directory: &Path,
    module: &str,
    debug: bool,
    target_triple: &str,
) -> PathBuf {
    std::fs::create_dir_all(directory).expect("create object directory");
    let ir = directory.join("app.ll");
    let object = directory.join("app.o");
    std::fs::write(&ir, module).expect("write IR");
    let mut command = Command::new(clang);
    command
        .args(["-x", "ir"])
        .arg(&ir)
        .args(["-c", "-o"])
        .arg(&object);
    if target_triple.contains("windows") {
        // Match the driver: disable the COFF object wall-clock timestamp so
        // identical inputs produce identical objects.
        command.arg("-mno-incremental-linker-compatible");
    }
    if debug {
        command.args(["-g", "-fdebug-compilation-dir=."]);
        if target_triple.contains("windows") {
            command.args(["-gcodeview", "-gno-codeview-command-line"]);
            command.arg("-Xclang").arg("-object-file-name=app.o");
        } else {
            command.arg("-gdwarf-5");
        }
    }
    let output = command.output().expect("run clang");
    assert!(
        output.status.success(),
        "clang rejected the generated IR:\n{}\nIR:\n{module}",
        String::from_utf8_lossy(&output.stderr)
    );
    object
}

fn section_dump(object: &Path) -> Option<String> {
    if tool_available("llvm-readobj") {
        return Some(tool_output(
            "llvm-readobj",
            &[OsStr::new("--sections"), object.as_os_str()],
        ));
    }
    if tool_available("readelf") {
        return Some(tool_output(
            "readelf",
            &[OsStr::new("-S"), object.as_os_str()],
        ));
    }
    None
}

fn require_toolchain_environment() -> bool {
    std::env::var_os("STC_REQUIRE_CLANG").is_some()
}

#[test]
fn debug_objects_are_deterministic_and_inspectable() {
    let Some(clang) = common::clang() else {
        return;
    };
    let target_triple = stc_codegen::host_triple();
    let paths = source_paths();
    let module = generate_ir(
        SOURCE,
        true,
        &target_triple,
        absolute_source_path(),
        Some(&paths),
    );

    let root = std::env::temp_dir().join(format!(
        "stc-debug-objects-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let first_dir = root.join("first");
    let second_dir = root.join("second");
    let first = compile_object(clang, &first_dir, &module, true, &target_triple);
    let second = compile_object(clang, &second_dir, &module, true, &target_triple);
    let first_bytes = std::fs::read(&first).expect("read first object");
    let second_bytes = std::fs::read(&second).expect("read second object");
    assert_eq!(
        first_bytes, second_bytes,
        "debug objects must not depend on the output directory"
    );
    if target_triple.contains("windows") {
        assert_eq!(
            &first_bytes[4..8],
            &[0, 0, 0, 0],
            "COFF objects must not carry a wall-clock timestamp"
        );
    }
    for path in [&first_dir, &second_dir] {
        let bytes = path.to_string_lossy();
        assert!(
            !first_bytes
                .windows(bytes.len())
                .any(|window| window == bytes.as_bytes()),
            "object embeds the output directory `{bytes}`"
        );
    }

    let sections = section_dump(&first).unwrap_or_else(|| {
        if require_toolchain_environment() {
            panic!("clang is present but no object-section inspection tool is available");
        }
        String::new()
    });
    let mut inspected = false;
    if cfg!(windows) {
        assert!(
            sections.contains(".debug$S") && sections.contains(".debug$T"),
            "CodeView sections are missing:\n{sections}"
        );
        if tool_available("llvm-pdbutil") {
            let dump = tool_output(
                "llvm-pdbutil",
                &[OsStr::new("dump"), OsStr::new("-all"), first.as_os_str()],
            );
            contains(
                &dump,
                &[
                    "S_LPROC32_ID",
                    "ADD",
                    "S_LOCAL",
                    "sum",
                    "LF_STRUCTURE",
                    "Point",
                    "LF_CLASS",
                    "Shape",
                    "LF_ENUM",
                    "Color",
                    "main.st",
                    "line/column/addr entries",
                ],
                "llvm-pdbutil",
            );
            assert!(
                !dump.contains("host-only"),
                "host path leaked into CodeView:\n{dump}"
            );
            inspected = true;
        }
        if tool_available("llvm-readobj") {
            let dump = tool_output(
                "llvm-readobj",
                &[OsStr::new("--codeview"), first.as_os_str()],
            );
            contains(
                &dump,
                &[
                    "Struct (",
                    "Point",
                    "DataMember",
                    "Class (",
                    "Shape",
                    "Enum (",
                    "Color",
                    "FuncId (",
                    "ADD",
                    "StringData:",
                    "main.st",
                    "stc:T:",
                ],
                "llvm-readobj --codeview",
            );
            inspected = true;
        }
    } else {
        assert!(
            sections.contains(".debug_info") && sections.contains(".debug_line"),
            "DWARF sections are missing:\n{sections}"
        );
        let (tool, info) = if tool_available("readelf") {
            (
                "readelf",
                tool_output(
                    "readelf",
                    &[OsStr::new("--debug-dump=info"), first.as_os_str()],
                ),
            )
        } else if tool_available("llvm-dwarfdump") {
            (
                "llvm-dwarfdump",
                tool_output(
                    "llvm-dwarfdump",
                    &[OsStr::new("--debug-info"), first.as_os_str()],
                ),
            )
        } else if tool_available("objdump") {
            (
                "objdump",
                tool_output("objdump", &[OsStr::new("--dwarf=info"), first.as_os_str()]),
            )
        } else {
            ("", String::new())
        };
        if info.is_empty() {
            if require_toolchain_environment() {
                panic!("clang is present but no DWARF inspection tool is available");
            }
            eprintln!("note: no DWARF inspection tool is available; skipping object content");
        } else {
            contains(
                &info,
                &[
                    "DW_TAG_compile_unit",
                    "DW_TAG_subprogram",
                    "ADD",
                    "DW_TAG_formal_parameter",
                    "DW_TAG_variable",
                    "sum",
                    "DW_TAG_structure_type",
                    "Point",
                    "DW_TAG_member",
                    "DW_TAG_class_type",
                    "Shape",
                    "DW_TAG_enumeration_type",
                    "Color",
                    "DW_TAG_array_type",
                    "DW_AT_decl_file",
                    "DW_AT_decl_line",
                    "DW_AT_location",
                    "main.st",
                ],
                tool,
            );
            assert!(
                !info.contains("host-only"),
                "host path leaked into DWARF:\n{info}"
            );
            inspected = true;
        }
        if tool_available("readelf") {
            let lines = tool_output(
                "readelf",
                &[OsStr::new("--debug-dump=decodedline"), first.as_os_str()],
            );
            contains(&lines, &["main.st"], "readelf --debug-dump=decodedline");
            let add_body_line = line_of("  sum := a + b;");
            let main_body_line = line_of("  held.id := held.id + 1;");
            for line in [add_body_line, main_body_line] {
                let line = line.to_string();
                contains(&lines, &[line.as_str()], "readelf --debug-dump=decodedline");
            }
            inspected = true;
        }
    }
    if !inspected {
        if require_toolchain_environment() {
            panic!("clang is present but no platform debug inspection tool is available");
        }
        eprintln!("note: no platform debug inspection tool is available; skipping");
    }
}

#[test]
fn debug_disabled_objects_have_no_debug_sections() {
    let Some(clang) = common::clang() else {
        return;
    };
    let target_triple = stc_codegen::host_triple();
    let paths = source_paths();
    let module = generate_ir(
        SOURCE,
        false,
        &target_triple,
        absolute_source_path(),
        Some(&paths),
    );
    let root = std::env::temp_dir().join(format!(
        "stc-debug-disabled-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let object = compile_object(clang, &root, &module, false, &target_triple);
    let Some(sections) = section_dump(&object) else {
        eprintln!("note: no object-section inspection tool is available; skipping");
        return;
    };
    for marker in [".debug$S", ".debug$T", ".debug_info", ".debug_line"] {
        assert!(
            !sections.contains(marker),
            "debug-disabled object contains `{marker}`:\n{sections}"
        );
    }
}
