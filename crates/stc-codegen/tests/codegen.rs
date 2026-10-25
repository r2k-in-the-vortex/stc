mod common;

use common::{clean_hir, generate, options, verify_with_clang};
use stc_codegen::Mode;

const OOP_HIDDEN: &str = r#"
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
  sa : ARRAY[1..2] OF Shape;
END_VAR

PROGRAM Main
VAR RETAIN
  held : Shape;
END_VAR
VAR
  fb : FB;
  s : Shape;
END_VAR
  held.id := held.id + 1;
  s.id := held.id;
  shared.id := s.id;
  fb(inc := 1);
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn descriptors_and_hidden_roots_emit_and_verify_with_clang() {
    let output = generate(OOP_HIDDEN, Mode::Executable);
    let module = &output.module;
    let manifest = &output.got.manifest;

    // Interface prototypes are function records with trap-stub bodies.
    let stub = manifest
        .functions
        .iter()
        .find(|function| function.key == "_ROOT::ISHAPE.AREA@()->DINT")
        .expect("interface prototype record");
    assert_eq!(stub.function_kind, stc_got::FunctionKind::Method);
    assert_eq!(stub.vtable_slot, Some(0));
    assert!(
        module.contains("define internal i32 @stc_m_ISHAPE_AREA_"),
        "{module}"
    );
    assert!(
        module.contains("call void %t") && module.contains("(i32 10, i32"),
        "the trap stub raises STC-TRAP_UNIMPLEMENTED:\n{module}"
    );

    // One TYPEINFO for each class/FB and a VTABLE for the class.
    for key in [
        "_ROOT::SHAPE#DESC:TYPEINFO",
        "_ROOT::SHAPE#DESC:VTABLE",
        "_ROOT::FB#DESC:TYPEINFO",
    ] {
        assert!(
            manifest
                .descriptors
                .iter()
                .any(|descriptor| descriptor.key == key),
            "missing {key}: {:#?}",
            manifest.descriptors
        );
    }
    assert!(
        !manifest
            .descriptors
            .iter()
            .any(|descriptor| descriptor.key.contains("ISHAPE")),
        "interface types have no descriptor"
    );

    // Hidden roots: handles for classes, records for classes and FBs.
    for key in [
        "_ROOT::SHARED#HANDLE",
        "_ROOT::SHARED#OBJREC",
        "_ROOT::SA#HANDLE",
        "_ROOT::SA.E0#OBJREC",
        "_ROOT::SA.E1#OBJREC",
        "_ROOT::MAIN.HELD#C/R/INST/HANDLE",
        "_ROOT::MAIN.HELD#C/R/INST/OBJREC",
        "_ROOT::MAIN.FB#C/R/INST/OBJREC",
    ] {
        assert!(
            manifest.roots.iter().any(|root| root.key == key),
            "missing {key}: {:#?}",
            manifest
                .roots
                .iter()
                .map(|root| &root.key)
                .collect::<Vec<_>>()
        );
    }
    assert!(
        !manifest
            .roots
            .iter()
            .any(|root| root.key == "_ROOT::MAIN.FB#C/R/INST/HANDLE"),
        "FB declarations get no handle root"
    );

    // FB bodies and methods take `(handle, root table, slots)`.
    assert!(
        module.contains("define internal void @stc_fb_body_FB(ptr %this, ptr %table, ptr %slots)"),
        "{}",
        module
    );
    assert!(
        module
            .matches("(ptr %this, ptr %table, ptr %slots)")
            .count()
            >= 2,
        "object bodies use the handled ABI:
{module}"
    );

    // Prepare fills fresh records and handles from the slot table.
    let prepare = module
        .split("@stc_module_prepare")
        .nth(1)
        .expect("prepare body");
    assert!(
        prepare.contains("getelementptr inbounds ptr, ptr"),
        "record entries are pointer-sized GEP stores:\n{prepare}"
    );

    // The owned instance's field roots stay retained; hidden roots never
    // appear in the payload or its metadata.
    let leaves = &output.metadata.retain.leaves;
    assert_eq!(leaves.len(), 1, "{leaves:#?}");
    assert_eq!(leaves[0].path.to_ascii_uppercase(), "INST.HELD.ID");
    assert_eq!(output.metadata.retain.size, 4);
    assert!(
        leaves
            .iter()
            .all(|leaf| !leaf.path.contains("HANDLE") && !leaf.path.contains("OBJREC"))
    );
    for key in [
        "_ROOT::MAIN.HELD#C/R/INST/HANDLE",
        "_ROOT::MAIN.HELD#C/R/INST/OBJREC",
    ] {
        let root = manifest
            .roots
            .iter()
            .find(|root| root.key == key)
            .unwrap_or_else(|| panic!("missing {key}"));
        assert_eq!(root.retention, stc_got::Retention::None);
    }
    let retained = manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::MAIN.HELD.ID#C/R/INST")
        .expect("owned instance field root");
    assert_eq!(retained.retention, stc_got::Retention::Retain);

    verify_with_clang("oop_hidden", module);
}

#[test]
fn class_methods_on_persistent_receivers_emit_and_verify() {
    let source = r#"
CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS

VAR_GLOBAL
  g : Shape;
END_VAR

PROGRAM Main
VAR
  m : Shape;
  x : DINT;
END_VAR
  g.id := 3;
  m.id := 4;
  x := g.Area() + m.Area();
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    // Both receivers pass the owned instance's field roots.
    assert!(
        output
            .module
            .contains("define internal void @stc_prog_MAIN_body"),
        "{}",
        output.module
    );
    assert!(
        output
            .module
            .contains("define internal i32 @stc_m_SHAPE_AREA_"),
        "{}",
        output.module
    );
    verify_with_clang("class_methods", &output.module);
}

#[test]
fn persistent_class_assignment_copies_the_handle() {
    let source = r#"
CLASS Shape
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS
VAR_GLOBAL
  a : Shape;
  b : Shape;
END_VAR
PROGRAM Main
VAR
  x : DINT;
END_VAR
  a := b;
  a.id := 3;
  x := a.id + b.id;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    // The assignment and the field write go through the referenced record.
    assert!(
        output
            .module
            .contains("define internal void @stc_prog_MAIN_body"),
        "{}",
        output.module
    );
    verify_with_clang("class_handle_assignment", &output.module);
}

#[test]
fn recursive_class_ownership_is_rejected_at_the_declaration() {
    let source = r#"
CLASS Node
VAR PUBLIC
  next : Node;
END_VAR
END_CLASS
PROGRAM Main
VAR
  root : Node;
END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("recursive owned instances have no finite storage");
    assert!(
        errors.iter().any(|error| {
            error.code() == "STC-E-CODEGEN-001"
                && error.message().contains("recursively owns")
                && error.span().is_some_and(|span| span.start != span.end)
        }),
        "{errors:#?}"
    );
}

#[test]
fn object_record_fills_are_guarded_by_the_fresh_mask() {
    let output = generate(OOP_HIDDEN, Mode::Executable);
    let prepare = output
        .module
        .split("@stc_module_prepare")
        .nth(1)
        .expect("prepare body");
    // Every record/handle fill is guarded by the cold-or-fresh phi.
    assert!(prepare.contains("phi i1 [ true,"), "{prepare}");
    assert!(prepare.contains("and i8"), "{prepare}");

    let second = generate(OOP_HIDDEN, Mode::Executable);
    assert_eq!(output.module, second.module);
    assert_eq!(
        output.metadata.to_json().render(),
        second.metadata.to_json().render()
    );
    assert_eq!(output.got.manifest_binary, second.got.manifest_binary);
    assert_eq!(output.got.descriptor_bytes, second.got.descriptor_bytes);
}

const SCALARS: &str = r#"
FUNCTION Fact : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
END_VAR
  r := 1;
  WHILE n > 1 DO
    r := r * n;
    n := n - 1;
  END_WHILE;
  Fact := r;
END_FUNCTION

PROGRAM Main
VAR
  count : DINT := 0;
  total : DINT;
  flag : BOOL;
END_VAR
  count := count + 1;
  total := Fact(5);
  IF total = 120 THEN
    flag := TRUE;
  ELSE
    flag := FALSE;
  END_IF;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const LIBRARY: &str = r#"
{stc:export}
FUNCTION TestAdd : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  TestAdd := a + b;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := TestAdd(x, 1);
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const IO_AND_RETAIN: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 3;
END_VAR

PROGRAM Main
VAR
  input AT %IX1.2 : BOOL {stc:io {"device":"rack","channel":3,"direction":"input"}};
  output AT %QW1 : WORD;
  plain : DINT;
END_VAR
VAR RETAIN
  Saved : DINT := 5;
END_VAR
  plain := Counter;
  output := 16#1234;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn emits_scalar_program_and_verifies_with_clang() {
    let output = generate(SCALARS, Mode::Executable);
    let module = &output.module;
    assert!(
        module.contains("define dso_local dllexport i32 @stc_module_cycle("),
        "{module}"
    );
    assert!(
        module.contains("define dso_local dllexport i32 @stc_module_prepare("),
        "{module}"
    );
    assert!(
        module.contains("define dso_local dllexport i32 @stc_module_attach("),
        "{module}"
    );
    assert!(!module.contains("@stc_pi_INST"), "{module}");
    assert!(module.contains("llvm.smul.with.overflow.i32"), "{module}");
    assert!(
        module.contains("define internal i32 @stc_fn_FACT_"),
        "{module}"
    );
    assert!(
        module.contains("call i32 %t"),
        "user calls are indirect:
{module}"
    );
    assert_eq!(output.metadata.programs.len(), 1);
    assert_eq!(output.metadata.programs[0].name, "inst");
    verify_with_clang("scalars", module);
}

#[test]
fn library_mode_exports_wrappers_and_metadata() {
    let output = generate(LIBRARY, Mode::Library);
    let module = &output.module;
    assert!(!module.contains("@main("), "{module}");
    assert!(
        module.contains("define dso_local dllexport i32 @stc_module_cycle("),
        "{module}"
    );
    assert!(
        module.contains("define dso_local dllexport i32 @stc_module_shutdown("),
        "{module}"
    );
    assert!(
        module.contains("define dso_local dllexport i32 @stc_export_TESTADD("),
        "{module}"
    );
    assert!(
        module.contains("define dso_local dllexport ptr @stc_metadata_json()"),
        "{module}"
    );
    assert_eq!(output.metadata.exports.len(), 1);
    assert_eq!(output.metadata.exports[0].symbol, "stc_export_TESTADD");
    assert!(!output.metadata.exports[0].no_mangle);
    verify_with_clang("library", module);
}

#[test]
fn emits_process_image_and_retain_layout() {
    let output = generate(IO_AND_RETAIN, Mode::Executable);
    let io = &output.metadata.io;
    assert_eq!(io.inputs_bytes, 2);
    assert_eq!(io.outputs_bytes, 4);
    assert_eq!(io.input_defaults_hex, "0000");
    assert_eq!(io.output_defaults_hex, "00000000");
    let input = io
        .bindings
        .iter()
        .find(|binding| binding.variable == "inst.input")
        .expect("input binding");
    assert_eq!(input.area, 'I');
    assert_eq!(input.offset_bits, 10);
    assert_eq!(input.address, "IX1.2");
    assert_eq!(input.ty, "BOOL");
    let hint = input.hint.as_ref().expect("io hint");
    assert_eq!(hint.device.as_deref(), Some("rack"));
    assert_eq!(hint.channel, Some(3));
    assert_eq!(hint.direction, Some("input"));
    let output_binding = io
        .bindings
        .iter()
        .find(|binding| binding.variable == "inst.output")
        .expect("output binding");
    assert_eq!(output_binding.offset_bits, 16);

    let retain = &output.metadata.retain;
    assert_eq!(retain.size, 8);
    let paths: Vec<_> = retain
        .leaves
        .iter()
        .map(|leaf| leaf.path.as_str())
        .collect();
    assert_eq!(paths, ["Counter", "inst.Saved"]);
    assert_eq!(retain.leaves[0].offset, 0);
    assert_eq!(retain.leaves[1].offset, 4);
    assert_ne!(retain.layout_hash, 0);
    verify_with_clang("io_retain", &output.module);
}

#[test]
fn metadata_is_deterministic() {
    let first = generate(IO_AND_RETAIN, Mode::Executable);
    let second = generate(IO_AND_RETAIN, Mode::Executable);
    assert_eq!(first.module, second.module);
    assert_eq!(
        first.metadata.to_json().render(),
        second.metadata.to_json().render()
    );
    assert_eq!(first.spans, second.spans);
}

#[test]
fn m5_metadata_is_deterministic() {
    let first = generate(AGGREGATES, Mode::Executable);
    let second = generate(AGGREGATES, Mode::Executable);
    assert_eq!(first.module, second.module);
    assert_eq!(
        first.metadata.to_json().render(),
        second.metadata.to_json().render()
    );
    assert_eq!(first.got.manifest_json, second.got.manifest_json);
    assert_eq!(first.got.manifest_binary, second.got.manifest_binary);
    assert_eq!(first.got.slots, second.got.slots);
}

#[test]
fn assignment_attempts_are_rejected_once_with_a_span() {
    let source = r#"
CLASS A
END_CLASS
CLASS B EXTENDS A
END_CLASS
PROGRAM Main
VAR
  a : A;
  b : B;
END_VAR
  a ?= b;
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("assignment attempts are deferred to phase 4");
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0].code(), "STC-E-CODEGEN-001");
    assert!(
        errors[0].message().contains("assignment attempts"),
        "{errors:#?}"
    );
    assert!(errors[0].span().is_some_and(|span| span.start != span.end));
}

#[test]
fn unsupported_interface_dispatch_reports_at_the_call_site() {
    let source = r#"
INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE

FUNCTION_BLOCK Use
VAR_INPUT
  i : IShape;
END_VAR
VAR
  x : DINT;
END_VAR
  x := i.Area();
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  u : Use;
END_VAR
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("interface dispatch is M5 phase 4b");
    assert!(
        errors.iter().any(|error| {
            error.code() == "STC-E-CODEGEN-001"
                && error.message().contains("interface dispatch")
                && error.span().is_some_and(|span| span.start != span.end)
        }),
        "{errors:#?}"
    );
}

#[test]
fn deep_binary_chain_does_not_overflow() {
    let mut source = String::from("PROGRAM Main\nVAR\n  x : DINT;\nEND_VAR\n  x := 1");
    for _ in 0..20_000 {
        source.push_str(" + 1");
    }
    source.push_str(";\nEND_PROGRAM\nCONFIGURATION Cfg\nRESOURCE R ON Cpu\n  PROGRAM inst : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n");
    let output = generate(&source, Mode::Executable);
    assert!(output.module.len() < 80_000_000);
    assert!(
        output
            .module
            .contains("define internal void @stc_prog_MAIN_body")
    );
}

#[test]
fn overloads_get_distinct_stable_symbols() {
    let source = r#"
FUNCTION F : DINT
VAR_INPUT a : DINT; END_VAR
  F := a;
END_FUNCTION
FUNCTION F : DINT
VAR_INPUT a : INT; END_VAR
  F := a;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
  x := F(1);
  x := F(2);
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let first = generate(source, Mode::Executable);
    let second = generate(source, Mode::Executable);
    assert_eq!(first.module, second.module);
    let symbols: Vec<&str> = first
        .module
        .lines()
        .filter_map(|line| line.strip_prefix("define internal i32 @stc_fn_F_"))
        .map(|rest| rest.split('(').next().unwrap_or(""))
        .collect();
    assert_eq!(symbols.len(), 2, "{}", first.module);
    assert_ne!(symbols[0], symbols[1]);
}

#[test]
fn golden_module_snapshot() {
    let output = generate(LIBRARY, Mode::Library);
    let path = common::snapshot_path("library");
    if std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &output.module).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot; run with STC_UPDATE_SNAPSHOTS=1"));
    assert_eq!(expected, output.module);
}

#[test]
fn implicit_single_program_is_instanced_as_default() {
    let source = r#"
PROGRAM Only
VAR x : DINT; END_VAR
  x := 1;
END_PROGRAM
"#;
    let output = generate(source, Mode::Executable);
    assert!(output.metadata.configuration.is_none());
    assert_eq!(output.metadata.programs.len(), 1);
    assert_eq!(output.metadata.programs[0].name, "_default");
    assert!(!output.module.contains("@stc_pi_"), "{}", output.module);
    assert!(
        output
            .module
            .contains("define internal void @stc_prog_ONLY_body("),
        "{}",
        output.module
    );
    verify_with_clang("implicit", &output.module);
}

#[test]
fn ambiguous_execution_plans_are_rejected() {
    let two_programs = r#"
PROGRAM A
VAR x : DINT; END_VAR
END_PROGRAM
PROGRAM B
VAR y : DINT; END_VAR
END_PROGRAM
"#;
    let hir = clean_hir(two_programs);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("two programs need a configuration");
    assert_eq!(errors[0].code(), "STC-E-RUN-002");

    let hir = clean_hir("");
    let errors_empty = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("an empty crate has no runnable program");
    assert_eq!(errors_empty[0].code(), "STC-E-RUN-002");

    let two_configs = r#"
PROGRAM A
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION C1
RESOURCE R ON Cpu
  PROGRAM inst : A;
END_RESOURCE
END_CONFIGURATION
CONFIGURATION C2
RESOURCE R ON Cpu
  PROGRAM inst : A;
END_RESOURCE
END_CONFIGURATION
"#;
    let hir = clean_hir(two_configs);
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("two configurations need a manifest");
    assert_eq!(errors[0].code(), "STC-E-RUN-002");
    assert!(errors[0].message().contains("C1"), "{errors:#?}");
}

#[test]
fn single_tasks_run_through_a_first_cycle_guard() {
    let source = r#"
PROGRAM A
VAR x : DINT; END_VAR
  x := x + 1;
END_PROGRAM
PROGRAM B
VAR y : DINT; END_VAR
  y := y + 1;
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  TASK Startup (SINGLE := TRUE, PRIORITY := 1);
  PROGRAM a WITH Startup : A;
  PROGRAM b WITH Startup : B;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(
        !output.module.contains("@stc_single_R_STARTUP"),
        "task guards are data roots:
{}",
        output.module
    );
    let cycle = output
        .module
        .split("define dso_local dllexport i32 @stc_module_cycle(")
        .nth(1)
        .expect("cycle entry");
    assert!(cycle.contains("store i8 %t"), "{cycle}");
    assert_eq!(output.metadata.tasks.len(), 1);
    assert_eq!(output.metadata.tasks[0].single, Some(true));
    assert_eq!(output.metadata.tasks[0].priority, Some(1));
    verify_with_clang("single_task", &output.module);
}

#[test]
fn multiple_tasks_per_resource_are_rejected() {
    let source = r#"
PROGRAM A
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  TASK T1 (INTERVAL := T#1ms);
  TASK T2 (INTERVAL := T#2ms);
  PROGRAM inst WITH T1 : A;
END_RESOURCE
END_CONFIGURATION
"#;
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("M4 supports one task per resource");
    assert_eq!(errors[0].code(), "STC-E-RUN-001");
}

#[test]
fn direct_variables_and_bit_access_are_lowered() {
    let source = r#"
PROGRAM Main
VAR
  word_in AT %IW0 : WORD;
  flag : BOOL;
  b : BYTE;
END_VAR
  %QW0 := word_in;
  b := %IB1;
  flag := b.0;
  b.3 := flag;
  %QB2 := b;
  flag := word_in.0;
  %QX0.1 := flag;
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert_eq!(output.metadata.io.inputs_bytes, 2);
    assert_eq!(output.metadata.io.outputs_bytes, 3);
    assert_eq!(output.metadata.io.markers_bytes, 0);
    assert!(!output.module.contains("@stc_io_base"));
    assert!(output.module.contains("@stc_module_slots"));
    assert!(output.module.contains("load i16, ptr"), "{}", output.module);
    assert!(output.module.contains("load i8, ptr"), "{}", output.module);
    verify_with_clang("direct", &output.module);
}

#[test]
fn default_arguments_are_materialized_at_the_call_site() {
    let source = r#"
FUNCTION Scale : DINT
VAR_INPUT
  value : DINT;
  factor : INT := 2;
END_VAR
  Scale := value * factor;
END_FUNCTION
PROGRAM Main
VAR x : DINT; END_VAR
  x := Scale(4);
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(
        output.module.contains("i16 2,"),
        "default argument missing:
{}",
        output.module
    );
    verify_with_clang("defaults", &output.module);
}

#[test]
fn void_functions_and_unreachable_statements_are_valid_ir() {
    let source = r#"
FUNCTION Touch
VAR_INPUT
  x : DINT;
END_VAR
END_FUNCTION

FUNCTION First : DINT
  First := 1;
  RETURN;
  First := 2;
END_FUNCTION

PROGRAM Main
VAR x : DINT; END_VAR
  Touch(1);
  x := First();
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(
        output
            .module
            .contains("define internal void @stc_fn_TOUCH_"),
        "{}",
        output.module
    );
    assert!(output.module.contains("call void %t"), "{}", output.module);
    verify_with_clang("void_functions", &output.module);
}

#[test]
fn each_program_instance_gets_its_own_global() {
    let source = r#"
PROGRAM P
VAR x : DINT; END_VAR
  x := x + 1;
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM a : P;
  PROGRAM b : P;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(!output.module.contains("@stc_pi_A"), "{}", output.module);
    assert!(!output.module.contains("@stc_pi_B"), "{}", output.module);
    assert_eq!(
        output
            .module
            .lines()
            .filter(|line| line.contains("define internal void @stc_prog_P_body("))
            .count(),
        1,
        "the program body is compiled once"
    );
    let cycle = output
        .module
        .split("define dso_local dllexport i32 @stc_module_cycle(")
        .nth(1)
        .unwrap();
    assert_eq!(cycle.matches("alloca [1 x ptr]").count(), 2, "{cycle}");
    verify_with_clang("instances", &output.module);
}

#[test]
fn character_types_use_their_declared_widths() {
    let source = r#"
PROGRAM Main
VAR
  c : CHAR;
  w : WCHAR;
  u : UCHAR;
  flag : BOOL;
END_VAR
  c := CHAR#'A';
  w := CHAR_TO_WCHAR(c);
  u := WCHAR_TO_UCHAR(w);
  flag := c = CHAR#'A';
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R on Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(output.module.contains("store i8"), "{}", output.module);
    assert!(output.module.contains("store i16"), "{}", output.module);
    assert!(output.module.contains("store i32"), "{}", output.module);
    verify_with_clang("chars", &output.module);
}

#[test]
fn var_config_at_overrides_are_rejected() {
    let source = r#"
PROGRAM P
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : P;
END_RESOURCE
VAR_CONFIG
  inst.x AT %MD0 : DINT := 1;
END_VAR
END_CONFIGURATION
"#;
    let hir = clean_hir(source);
    let sources = vec!["<test>".to_string()];
    let errors = stc_codegen::generate(&hir, &options("test", Mode::Executable, &sources))
        .expect_err("AT overrides are M5");
    assert_eq!(errors[0].code(), "STC-E-CODEGEN-001");
    assert!(errors[0].message().contains("AT"), "{errors:#?}");
}

#[test]
fn namespaces_participate_in_mangled_names() {
    let source = r#"
NAMESPACE Alpha
FUNCTION F : DINT
  F := 1;
END_FUNCTION
END_NAMESPACE

NAMESPACE Beta
FUNCTION F : DINT
  F := 2;
END_FUNCTION
END_NAMESPACE

PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    assert!(
        output.module.contains("@stc_fn_ALPHA_n_F_"),
        "{}",
        output.module
    );
    assert!(
        output.module.contains("@stc_fn_BETA_n_F_"),
        "{}",
        output.module
    );
    verify_with_clang("namespaces", &output.module);
}

const GOT_SURFACE: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 3;
END_VAR
VAR_GLOBAL
  Flag AT %MX0.0 : BOOL;
END_VAR

FUNCTION Bump : DINT
VAR_INPUT
  value : DINT;
END_VAR
  Counter := Counter + value;
  Bump := Counter;
END_FUNCTION

PROGRAM Main
VAR
  input AT %IX0.0 : BOOL;
  output AT %QX0.0 : BOOL;
  ticks : DINT;
END_VAR
VAR RETAIN
  saved : DINT;
END_VAR
  ticks := ticks + 1;
  saved := Bump(ticks);
  output := input;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  TASK Fast (SINGLE := TRUE);
  PROGRAM inst WITH Fast : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn generated_module_has_no_direct_persistent_or_user_references() {
    let output = generate(GOT_SURFACE, Mode::Executable);
    let module = &output.module;
    assert!(!module.contains("@stc_g_"), "{module}");
    assert!(!module.contains("@stc_pi_"), "{module}");
    assert!(!module.contains("@stc_single_"), "{module}");
    assert!(!module.contains("@stc_io_base"), "{module}");
    assert!(module.contains("define dso_local dllexport i32 @stc_module_attach("));
    assert!(module.contains("define dso_local dllexport i32 @stc_module_prepare("));
    assert!(module.contains("define dso_local dllexport i32 @stc_module_cycle("));
    assert!(module.contains("define dso_local dllexport i32 @stc_module_shutdown("));
    assert!(module.contains("define dso_local dllexport void @stc_module_detach("));
    for line in module.lines() {
        assert!(
            !line.trim_start().starts_with("call ") || !line.contains("@stc_fn_"),
            "direct user call in:\n{module}"
        );
    }

    let table = output.got.function_table();
    let descriptor = &output.got.descriptor.functions;
    assert_eq!(table.len(), descriptor.len(), "function table length");
    for (index, (got_id, symbol)) in table.iter().enumerate() {
        assert_eq!(*got_id, descriptor[index].got_id, "descriptor order");
        assert!(
            module.contains(&format!("ptr @{symbol}")),
            "function table names `{symbol}`"
        );
    }
    verify_with_clang("got_surface", module);
}

const AGGREGATES: &str = r#"
TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE
PROGRAM Main
VAR
  c : Color;
  s : Small;
  p : Point;
  q : Point;
  arr : ARRAY[1..3] OF DINT;
  ps : ARRAY[1..2] OF Point;
  str : STRING[10];
  i : DINT;
  v : DINT;
END_VAR
  c := Green;
  s := 5;
  p.x := 3;
  p.y := 4;
  q := p;
  arr[1] := q.x + q.y;
  FOR i := 1 TO 2 DO
    ps[i].x := i * 10;
    ps[i].y := i * 20;
  END_FOR;
  str := CONCAT('ab', 'cd');
  v := LEN(str);
  v := v + arr[1] + ps[2].y + s;
END_PROGRAM
CONFIGURATION Cfg
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn aggregate_program_emits_and_verifies_with_clang() {
    let output = generate(AGGREGATES, Mode::Executable);
    let module = &output.module;
    assert!(module.contains("load { i32, i32 }"), "{module}");
    assert!(module.contains("store { i32, i32 }"), "{module}");
    assert!(module.contains("llvm.memcpy.p0.p0.i64"), "{module}");
    assert!(module.contains("icmp sgt i64"), "{module}");
    assert!(
        output
            .got
            .manifest
            .types
            .iter()
            .any(|ty| ty.key.ends_with("::POINT")),
        "{:#?}",
        output.got.manifest.types
    );
    assert!(
        output
            .got
            .manifest
            .roots
            .iter()
            .any(|root| root.key.contains(".P.X")),
        "{:#?}",
        output.got.manifest.roots
    );
    assert!(
        output
            .got
            .manifest
            .roots
            .iter()
            .any(|root| root.key.contains(".PS.E1.Y")),
        "{:#?}",
        output.got.manifest.roots
    );
    verify_with_clang("aggregates", module);
}

#[test]
fn virtual_dispatch_reads_typeinfo_and_the_class_vtable() {
    let source = r#"
CLASS Base
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
END_CLASS

CLASS Derived EXTENDS Base
VAR PUBLIC
  extra : DINT;
END_VAR
METHOD PUBLIC OVERRIDE M : DINT
  M := SUPER.M() + extra;
END_METHOD
END_CLASS

{stc:export}{stc:no_mangle}
FUNCTION Dispatch : DINT
VAR
  d : Derived;
  b : Base;
END_VAR
  d.id := 1;
  d.extra := 2;
  b := d;
  Dispatch := b.M();
END_FUNCTION

PROGRAM Main
VAR x : DINT; END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    // The dispatch loads TYPEINFO (record word 0), the class VTABLE at
    // TYPEINFO+64, and slot 0 at VTABLE+48 before calling the cell's code.
    assert!(
        output.module.contains("getelementptr i64, ptr %t") && output.module.contains(", i64 8"),
        "{}",
        output.module
    );
    assert!(
        output.module.contains(", i64 6"),
        "the class VTABLE entry GEP is missing:\n{}",
        output.module
    );
    assert!(
        output.module.matches("call i32 %").count() >= 2,
        "virtual and SUPER calls are missing:\n{}",
        output.module
    );
    verify_with_clang("virtual_dispatch", &output.module);
}

#[test]
fn class_null_dispatch_traps_at_runtime() {
    let source = r#"
CLASS Obj
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := x;
END_METHOD
END_CLASS

{stc:export}{stc:no_mangle}
FUNCTION Boom : DINT
VAR
  c : Obj;
END_VAR
  c := NULL;
  Boom := c.M();
END_FUNCTION

PROGRAM Main
VAR x : DINT; END_VAR
  x := Boom();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;
    let output = generate(source, Mode::Executable);
    verify_with_clang("class_null_dispatch", &output.module);
}
