mod common;

use common::{clean_hir, generate, options, verify_with_clang};
use stc_codegen::Mode;

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
fn unsupported_declarations_report_once_with_a_span() {
    let source = r#"
TYPE Color : (Red, Green);
END_TYPE
CLASS C
END_CLASS
PROGRAM Main
VAR x : DINT; END_VAR
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
        .expect_err("M5 declarations must be rejected");
    assert!(
        errors
            .iter()
            .all(|error| error.code() == "STC-E-CODEGEN-001"),
        "{errors:#?}"
    );
    let messages = errors
        .iter()
        .map(|error| error.message().to_string())
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| message.contains("ENUM")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("CLASS")),
        "{messages:?}"
    );
    assert!(errors.iter().all(|error| {
        error
            .span()
            .is_some_and(|span| span.start != 0 || span.end != 0)
    }));
}

#[test]
fn unsupported_standard_call_reports_at_the_call_site() {
    let source = r#"
PROGRAM Main
VAR
  x : DINT;
  y : DINT;
END_VAR
  y := ABS(x);
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
        .expect_err("standard functions are M5");
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0].code(), "STC-E-CODEGEN-001");
    assert!(errors[0].message().contains("ABS"), "{errors:#?}");
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
