use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn runtime_artifact() -> PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let output = Command::new(cargo)
                .args(["build", "-p", "stc-runtime", "--locked"])
                .current_dir(workspace_root())
                .output()
                .expect("run cargo build -p stc-runtime");
            assert!(
                output.status.success(),
                "failed to build stc-runtime:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let exe = std::env::current_exe().expect("test executable");
            let profile = exe
                .parent()
                .and_then(Path::parent)
                .expect("target profile directory");
            let name = if cfg!(windows) {
                "stc_runtime.lib"
            } else {
                "libstc_runtime.a"
            };
            let artifact = profile.join(name);
            assert!(
                artifact.is_file(),
                "runtime artifact missing at {}",
                artifact.display()
            );
            artifact
        })
        .clone()
}

fn clang() -> Option<&'static str> {
    static CLANG: OnceLock<Option<&'static str>> = OnceLock::new();
    CLANG
        .get_or_init(|| {
            let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
            match Command::new(&candidate).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    Some(Box::leak(candidate.into_boxed_str()))
                }
                _ => {
                    if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                        panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
                    }
                    eprintln!("note: clang not found; skipping execution test");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn exe_path(directory: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        directory.join(format!("{name}.exe"))
    } else {
        directory.join(name)
    }
}

fn lib_path(directory: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        directory.join(format!("{name}.lib"))
    } else {
        directory.join(format!("{name}.a"))
    }
}

fn built_module(directory: &TestDir, name: &str) -> String {
    let extension = if cfg!(windows) { "dll" } else { "so" };
    let prefix = format!("{name}-");
    let suffix = format!(".{extension}");
    std::fs::read_dir(&directory.path)
        .expect("artifact directory")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|file| file.starts_with(&prefix) && file.ends_with(&suffix))
        .unwrap_or_else(|| panic!("module for `{name}` was not emitted"))
}

fn build_library(directory: &TestDir, source: &str, name: &str) {
    let source_path = directory.write_source(name, source);
    let runtime = runtime_artifact();
    let output = run_stc(&[
        "build",
        source_path.to_str().unwrap(),
        "--emit=lib",
        "--name",
        name,
        "--out-dir",
        directory.path.to_str().unwrap(),
        "--runtime",
        runtime.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "library build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = built_module(directory, name);
    assert!(
        directory.path.join(format!("{name}.stc-got.bin")).is_file(),
        "manifest missing"
    );
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("stc-exec-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write_source(&self, name: &str, source: &str) -> PathBuf {
        let path = self.path.join(format!("{name}.st"));
        std::fs::write(&path, source).expect("write source");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn run_stc(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("run stc binary")
}

fn build_executable(directory: &TestDir, source_name: &str, artifact: &str) {
    let source = directory.write_source(source_name, source_name_source(source_name));
    let runtime = runtime_artifact();
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=exe",
        "--name",
        artifact,
        "--out-dir",
        directory.path.to_str().unwrap(),
        "--runtime",
        runtime.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn source_name_source(name: &str) -> &'static str {
    match name {
        "noop" => NOOP,
        "divzero" => DIV_ZERO,
        "overflow" => OVERFLOW,
        "narrow" => NARROW,
        "powneg" => POW_NEGATIVE,
        "powoverflow" => POW_OVERFLOW,
        "subrange" => SUBRANGE,
        "bounds" => BOUNDS,
        other => panic!("unknown source {other}"),
    }
}

const NOOP: &str = r#"
{stc:export}{stc:no_mangle}
FUNCTION IsPos : BOOL
VAR_INPUT
  x : DINT;
END_VAR
  IsPos := x > 0;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION Half : LREAL
VAR_INPUT
  x : LREAL;
END_VAR
  Half := x / 2.0;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const DIV_ZERO: &str = r#"
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 10;
  x := x / 0;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const OVERFLOW: &str = r#"
PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := 2147483647;
  x := x + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const POW_NEGATIVE: &str = r#"
PROGRAM Main
VAR
  x : DINT;
  y : DINT;
END_VAR
  y := -1;
  x := 2 ** y;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const POW_OVERFLOW: &str = r#"
PROGRAM Main
VAR
  x : DINT;
  a : DINT;
  b : DINT;
END_VAR
  a := 2;
  b := 31;
  x := a ** b;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const SUBRANGE: &str = r#"
TYPE
  Small : INT(0..10);
END_TYPE
PROGRAM Main
VAR
  s : Small;
  x : DINT;
END_VAR
  x := 11;
  s := x;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const BOUNDS: &str = r#"
PROGRAM Main
VAR
  a : ARRAY[1..2] OF DINT;
  i : DINT;
END_VAR
  i := 3;
  a[i] := 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const NARROW: &str = r#"
PROGRAM Main
VAR
  x : DINT;
  y : SINT;
END_VAR
  x := 300;
  y := DINT_TO_SINT(x);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn executable_runs_a_no_op_cycle() {
    let directory = TestDir::new("noop");
    build_executable(&directory, "noop", "noop");
    for cycles in ["0", "3"] {
        let output = Command::new(exe_path(&directory.path, "noop"))
            .args(["--cycles", cycles])
            .output()
            .expect("run executable");
        assert_eq!(
            output.status.code(),
            Some(0),
            "--cycles {cycles}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for argument in ["--trace", "--cycles"] {
        let output = Command::new(exe_path(&directory.path, "noop"))
            .arg(argument)
            .output()
            .expect("run executable");
        assert_eq!(output.status.code(), Some(2), "{argument} was accepted");
    }
}

#[test]
fn traps_are_fatal_and_reported() {
    for (source, needle) in [
        ("divzero", "DIV_ZERO"),
        ("overflow", "OVERFLOW"),
        ("narrow", "RANGE"),
        ("powneg", "RANGE"),
        ("powoverflow", "OVERFLOW"),
        ("subrange", "RANGE"),
        ("bounds", "BOUNDS"),
    ] {
        let directory = TestDir::new(source);
        build_executable(&directory, source, source);
        let output = Command::new(exe_path(&directory.path, source))
            .args(["--cycles", "1"])
            .output()
            .expect("run executable");
        assert_eq!(output.status.code(), Some(70), "{source} did not trap");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{source}: {stderr}");
    }
}

const LIBRARY: &str = r#"
{stc:export}
FUNCTION TestAdd : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  TestAdd := a + b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION TestFact : DINT
VAR_INPUT
  n : DINT;
END_VAR
VAR
  r : DINT;
  i : DINT;
END_VAR
  r := 1;
  FOR i := n TO 1 BY -1 DO
    r := r * i;
  END_FOR;
  TestFact := r;
END_FUNCTION

{stc:export}
FUNCTION TestControl : DINT
VAR_INPUT
  seed : DINT;
END_VAR
VAR
  i : DINT;
  total : DINT;
  small : SINT;
END_VAR
  total := 0;
  FOR i := 1 TO seed DO
    CASE i OF
      1..3: total := total + 1;
      4: total := total + 10;
    ELSE
      total := total + 100;
    END_CASE;
  END_FOR;
  WHILE total < 100 DO
    total := total + 5;
  END_WHILE;
  REPEAT
    total := total - 1;
  UNTIL total = 100 END_REPEAT;
  small := DINT_TO_SINT(seed);
  IF small > 10 THEN
    total := total + 1;
  END_IF;
  TestControl := total;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION IsPos : BOOL
VAR_INPUT
  x : DINT;
END_VAR
  IsPos := x > 0;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION Half : LREAL
VAR_INPUT
  x : LREAL;
END_VAR
  Half := x / 2.0;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const IO_SOURCE: &str = r#"
PROGRAM Main
VAR
  start AT %IX0.0 : BOOL := TRUE;
  done AT %QX0.0 : BOOL;
  marker AT %MW0 : INT;
  count : DINT;
END_VAR
  count := count + 1;
  marker := 42;
  done := start;
  %QW1 := %IW1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn compile_harness(
    directory: &TestDir,
    harness: &str,
    library: &Path,
    module: &str,
    output_name: &str,
) -> PathBuf {
    let clang = clang().expect("clang required for harness tests");
    let harness_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/harness")
        .join(harness);
    let object = directory.path.join("harness.o");
    let compile = Command::new(clang)
        .arg(&harness_path)
        .args(["-c", "-o"])
        .arg(&object)
        .output()
        .expect("compile harness");
    assert!(
        compile.status.success(),
        "harness compile failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let exe = exe_path(&directory.path, output_name);
    let mut link = Command::new(clang);
    link.arg(&object).arg(library).args(["-o"]).arg(&exe);
    for argument in stc_driver::toolchain::system_link_args() {
        link.arg(argument);
    }
    let linked = link.output().expect("link harness");
    assert!(
        linked.status.success(),
        "harness link failed:\n{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let run = Command::new(&exe)
        .arg(module)
        .output()
        .expect("run harness");
    assert_eq!(
        run.status.code(),
        Some(0),
        "{harness} failed: stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    exe
}

#[test]
fn library_mode_exports_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("library");
    build_library(&directory, LIBRARY, "library");
    let library = lib_path(&directory.path, "library");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "library.c",
        &library,
        &built_module(&directory, "library"),
        "harness",
    );
}

#[test]
fn launcher_and_embedding_host_load_the_same_module() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("both-hosts");
    let source = directory.write_source("library", LIBRARY);
    let runtime = runtime_artifact();
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=exe,lib",
        "--name",
        "both",
        "--out-dir",
        directory.path.to_str().unwrap(),
        "--runtime",
        runtime.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "combined build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let module = built_module(&directory, "both");

    let launcher = Command::new(exe_path(&directory.path, "both"))
        .args(["--cycles", "2"])
        .output()
        .expect("run the launcher");
    assert_eq!(
        launcher.status.code(),
        Some(0),
        "launcher failed: {}",
        String::from_utf8_lossy(&launcher.stderr)
    );

    let library = lib_path(&directory.path, "both");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(&directory, "library.c", &library, &module, "both_harness");
}

#[test]
fn process_image_is_observable_through_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("io");
    build_library(&directory, IO_SOURCE, "io");
    let metadata =
        std::fs::read_to_string(directory.path.join("io.stc-meta.json")).expect("metadata file");
    assert!(
        metadata.contains("\"input_defaults_hex\": \"01000000\""),
        "{metadata}"
    );
    assert!(metadata.contains("\"got_id\":"), "{metadata}");
    let library = lib_path(&directory.path, "io");
    compile_harness(
        &directory,
        "io.c",
        &library,
        &built_module(&directory, "io"),
        "io_harness",
    );
}

#[test]
fn build_emits_deterministic_ir_and_metadata() {
    let directory = TestDir::new("determinism");
    let source = directory.write_source("noop", NOOP);
    for run in 0..2 {
        let out = directory.path.join(format!("run{run}"));
        std::fs::create_dir_all(&out).unwrap();
        let output = run_stc(&[
            "build",
            source.to_str().unwrap(),
            "--emit=ir,metadata",
            "--name",
            "noop",
            "--out-dir",
            out.to_str().unwrap(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let first = std::fs::read(directory.path.join("run0/noop.ll")).unwrap();
    let second = std::fs::read(directory.path.join("run1/noop.ll")).unwrap();
    assert_eq!(first, second);
    let first = std::fs::read(directory.path.join("run0/noop.stc-meta.json")).unwrap();
    let second = std::fs::read(directory.path.join("run1/noop.stc-meta.json")).unwrap();
    assert_eq!(first, second);
}

#[test]
fn unsupported_constructs_are_compiler_errors() {
    let directory = TestDir::new("unsupported");
    let source = directory.write_source(
        "unsupported",
        "CLASS A\nEND_CLASS\nCLASS B EXTENDS A\nEND_CLASS\nPROGRAM Main\nVAR a : A; b : B; END_VAR\n  a ?= b;\nEND_PROGRAM\n\
         CONFIGURATION Cfg\nRESOURCE R ON Cpu\n  PROGRAM inst : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
    );
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=ir",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("STC-E-CODEGEN-001"), "{stderr}");
}

#[test]
fn toolchain_errors_use_exit_two() {
    let directory = TestDir::new("toolchain");
    let source = directory.write_source("noop", NOOP);
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=obj",
        "--clang",
        "definitely-not-a-clang",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("STC-E-BUILD-001"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=exe",
        "--runtime",
        "definitely-not-a-runtime.lib",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("STC-E-BUILD-003"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const ADVANCED: &str = r#"
VAR_GLOBAL
  seed : DINT := 5;
  result : DINT;
  Calls : DINT;
  cycles : DINT;
END_VAR
VAR_GLOBAL RETAIN
  Saved : DINT;
END_VAR

FUNCTION Bump : BOOL
  Calls := Calls + 1;
  Bump := TRUE;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION ShortCircuit : DINT
  Calls := 0;
  IF FALSE AND Bump() THEN
  END_IF;
  IF TRUE OR Bump() THEN
  END_IF;
  ShortCircuit := Calls;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION TruncHalf : DINT
VAR_INPUT
  x : LREAL;
END_VAR
  TruncHalf := LREAL_TO_DINT(x / 2.0);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION SetValue : DINT
VAR_INPUT
  v : DINT;
END_VAR
  Saved := v;
  SetValue := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetValue : DINT
  GetValue := Saved;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetResult : DINT
  GetResult := result;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetCycles : DINT
  GetCycles := cycles;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PowInt : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  PowInt := a ** b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PowReal : LREAL
VAR_INPUT
  a : LREAL;
  b : LREAL;
END_VAR
  PowReal := a ** b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION ModTest : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  ModTest := a MOD b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION UDiv : UDINT
VAR_INPUT
  a : UDINT;
  b : UDINT;
END_VAR
  UDiv := a / b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION BitOps : WORD
VAR_INPUT
  a : WORD;
  b : WORD;
END_VAR
  BitOps := (a AND b) OR (a XOR b);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION TimeAdd : TIME
VAR_INPUT
  a : TIME;
  b : TIME;
END_VAR
  TimeAdd := a + b;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION TimeScale : TIME
VAR_INPUT
  t : TIME;
  f : DINT;
END_VAR
  TimeScale := (t * f) / 2;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION DateToNs : LDATE
VAR_INPUT
  d : DATE;
END_VAR
  DateToNs := DATE_TO_LDATE(d);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION LoopControl : DINT
VAR
  i : DINT;
  total : DINT;
END_VAR
  total := 0;
  FOR i := 1 TO 10 DO
    IF i = 3 THEN
      CONTINUE;
    END_IF;
    IF i = 6 THEN
      EXIT;
    END_IF;
    total := total + i;
  END_FOR;
  LoopControl := total;
END_FUNCTION

PROGRAM P
VAR_INPUT
  input : DINT;
END_VAR
VAR_OUTPUT
  output : DINT;
END_VAR
VAR
  configurable : DINT := 1;
END_VAR
  cycles := cycles + 1;
  output := input + configurable;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : P (input := seed, output => result);
END_RESOURCE
VAR_CONFIG
  inst.configurable : DINT := 5;
END_VAR
END_CONFIGURATION
"#;

#[test]
fn bindings_short_circuit_casts_and_retained_state() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("advanced");
    build_library(&directory, ADVANCED, "advanced");
    let metadata = std::fs::read_to_string(directory.path.join("advanced.stc-meta.json"))
        .expect("metadata file");
    assert!(metadata.contains("\"path\": \"Saved\""), "{metadata}");
    assert!(metadata.contains("\"size\": 4"), "{metadata}");
    let library = lib_path(&directory.path, "advanced");
    compile_harness(
        &directory,
        "advanced.c",
        &library,
        &built_module(&directory, "advanced"),
        "advanced_harness",
    );
}

#[test]
fn build_json_emits_codegen_errors_on_stdout() {
    let directory = TestDir::new("json");
    let source = directory.write_source(
        "unsupported",
        "CLASS A\nEND_CLASS\nCLASS B EXTENDS A\nEND_CLASS\nPROGRAM Main\nVAR a : A; b : B; END_VAR\n  a ?= b;\nEND_PROGRAM\n\
         CONFIGURATION Cfg\nRESOURCE R ON Cpu\n  PROGRAM inst : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
    );
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=ir",
        "--message-format=json",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"code\":\"STC-E-CODEGEN-001\""),
        "{stdout}"
    );
    assert!(stdout.contains("\"file\":"), "{stdout}");
    assert!(!directory.path.join("unsupported.ll").exists());
}

#[test]
fn build_stops_before_codegen_when_parsing_fails() {
    let directory = TestDir::new("parse_error");
    let source = directory.write_source("broken", "PROGRAM Main\n  x := ;\nEND_PROGRAM\n");
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=ir",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("STC-E-GRAM-105"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path.join("broken.ll").exists());
}

#[test]
fn cross_target_object_emission_is_supported() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("cross");
    let source = directory.write_source("noop", NOOP);
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=obj",
        "--target=aarch64-unknown-linux-gnu",
        "--name",
        "cross",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "cross-target build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path.join("cross.o").is_file());
    let ir = std::fs::read_to_string(directory.path.join("cross.ll")).expect("IR");
    assert!(
        ir.contains("target triple = \"aarch64-unknown-linux-gnu\""),
        "{ir}"
    );
    assert!(
        !ir.contains("target datalayout"),
        "datalayout must be omitted"
    );
    assert!(!directory.path.join("cross.exe").exists());
}

#[test]
fn runtime_is_discovered_beside_the_driver() {
    let _ = runtime_artifact();
    let directory = TestDir::new("discovery");
    let source = directory.write_source("noop", NOOP);
    let output = run_stc(&[
        "build",
        source.to_str().unwrap(),
        "--emit=exe",
        "--name",
        "discovery",
        "--out-dir",
        directory.path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "discovery build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const AGGREGATES: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

FUNCTION Make : Point
VAR_INPUT
  x : DINT;
  y : DINT;
END_VAR
  Make.x := x;
  Make.y := y;
END_FUNCTION

FUNCTION SumPoint : DINT
VAR_INPUT
  p : Point;
END_VAR
  SumPoint := p.x + p.y;
END_FUNCTION

FUNCTION Update : DINT
VAR_INPUT
  a : DINT;
END_VAR
VAR_OUTPUT
  o : DINT;
END_VAR
VAR_IN_OUT
  io : DINT;
END_VAR
  o := a + io;
  io := io + 1;
  Update := o;
END_FUNCTION

FUNCTION Mutate : DINT
VAR_INPUT
  p : Point;
END_VAR
  p.x := 99;
  Mutate := p.x;
END_FUNCTION

FUNCTION Bump : DINT
VAR_IN_OUT
  p : Point;
END_VAR
  p.x := p.x + 1;
  p.y := p.y + 1;
  Bump := p.x * 10 + p.y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION Run : DINT
VAR
  p : Point;
  q : Point;
  x : DINT;
  y : DINT;
  v : DINT;
END_VAR
  p := Make(1, 2);
  q := Make(x := 3, y := 4);
  v := SumPoint(p) + SumPoint(q);
  Update(a := 10, o => x, io := y);
  IF Mutate(p) <> 99 THEN v := v + 1000; END_IF;
  IF p.x <> 1 THEN v := v + 2000; END_IF;
  Run := v + x + y;
END_FUNCTION

VAR_GLOBAL
  gp : Point;
  gps : ARRAY[1..2] OF Point;
  gstr : STRING[10];
  bp : Point := (x := 1, y := 2);
  ip : Point := (x := 3, y := 4);
  ia : ARRAY[1..2] OF DINT := [5, 6];
  istr : STRING[8] := 'hi';
  im : ARRAY[1..2, 1..3] OF DINT;
  ic : Color;
  iss : Small;
  istr2 : STRING[10];
END_VAR

TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
END_TYPE

{stc:export}{stc:no_mangle}
FUNCTION Multi : DINT
VAR
  v : DINT;
END_VAR
  im[2, 3] := 7;
  v := im[2, 3];
  ic := Green;
  CASE ic OF
    Red: v := v + 1;
    Green: v := v + 2;
  ELSE
    v := v + 100;
  END_CASE;
  iss := 5;
  v := v + iss;
  istr2 := 'hello';
  IF istr2 = 'hello' THEN v := v + 10; END_IF;
  IF istr2 <> 'hell' THEN v := v + 20; END_IF;
  Multi := v;
END_FUNCTION

VAR_GLOBAL
  rx : DINT;
END_VAR

FUNCTION SumC : DINT
VAR_IN_OUT
  a : ARRAY[*] OF DINT;
END_VAR
  SumC := a[1] + a[2] + a[3];
END_FUNCTION

FUNCTION BumpC : DINT
VAR_IN_OUT
  a : ARRAY[*] OF DINT;
END_VAR
  a[1] := a[1] + 10;
  BumpC := a[1];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunConf : DINT
VAR
  arr : ARRAY[1..3] OF DINT;
  v : DINT;
END_VAR
  arr[1] := 1;
  arr[2] := 2;
  arr[3] := 3;
  v := SumC(arr);
  v := v + BumpC(arr);
  RunConf := v + arr[1];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RefTest : DINT
VAR
  p : REF_TO DINT;
  q : REF_TO DINT;
  n : REF_TO DINT;
  x : DINT;
END_VAR
  rx := 5;
  p := REF(rx);
  q := p;
  x := q^;
  q^ := 9;
  n := NULL;
  IF n = NULL THEN x := x + 1; END_IF;
  IF p = NULL THEN x := x + 100; END_IF;
  RefTest := x + rx;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION BumpInit : DINT
  BumpInit := Bump(bp);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION SumBp : DINT
  SumBp := bp.x + bp.y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION InitSum : DINT
  InitSum := ip.x * 1000 + ip.y * 100 + ia[1] * 10 + ia[2] + LEN(istr);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION SetPoint : DINT
VAR_INPUT
  x : DINT;
  y : DINT;
END_VAR
  gp.x := x;
  gp.y := y;
  SetPoint := gp.x * 10 + gp.y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetSum : DINT
  GetSum := gp.x + gp.y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION SetElem : DINT
VAR_INPUT
  i : DINT;
  v : DINT;
END_VAR
  gps[i].x := v;
  gps[i].y := v + 1;
  SetElem := gps[i].x * 100 + gps[i].y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION SetText : DINT
VAR
  n : DINT;
END_VAR
  gstr := 'hello';
  n := LEN(gstr);
  SetText := n * 10 + LEN_CODE_UNIT(gstr);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION CopyTest : DINT
VAR
  p : Point;
END_VAR
  p := gp;
  p.x := p.x + 1;
  CopyTest := p.x * 10 + p.y;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn aggregate_library_runs_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("aggregates");
    build_library(&directory, AGGREGATES, "aggregates");
    let library = lib_path(&directory.path, "aggregates");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "aggregates.c",
        &library,
        &built_module(&directory, "aggregates"),
        "aggregates_harness",
    );
}
