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
        "strrange" => STRRANGE,
        "conf2bounds" => CONF2BOUNDS,
        "ptrtrap" => PTR_NULL_TRAP,
        "objtrap" => OBJECT_NULL_TRAP,
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

const STRRANGE: &str = r#"
PROGRAM Main
VAR
  s : STRING[4];
END_VAR
  s := 'abcd';
  s := INSERT(s, 'x', 1);
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const CONF2BOUNDS: &str = r#"
FUNCTION Sum2 : DINT
VAR_IN_OUT
  a : ARRAY[*, *] OF DINT;
END_VAR
  Sum2 := a[3, 1];
END_FUNCTION

PROGRAM Main
VAR
  m : ARRAY[1..2, 1..3] OF DINT;
  x : DINT;
END_VAR
  x := Sum2(m);
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
        ("strrange", "RANGE"),
        ("conf2bounds", "BOUNDS"),
        ("ptrtrap", "NULL_REF"),
        ("objtrap", "NULL_REF"),
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

const OBJECT_NULL_TRAP: &str = r#"
CLASS Obj
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := x;
END_METHOD
END_CLASS

FUNCTION Boom : DINT
VAR
  c : Obj;
END_VAR
  c := NULL;
  Boom := c.M();
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := Boom();
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const PTR_NULL_TRAP: &str = r#"
FUNCTION Boom : DINT
VAR
  p : POINTER TO DINT;
END_VAR
  p := NULL;
  Boom := p^;
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
  x := Boom();
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
        "TYPE\n  Point : STRUCT\n    x : DINT;\n    y : DINT;\n  END_STRUCT;\nEND_TYPE\n\
         VAR_GLOBAL\n  g : Point;\nEND_VAR\n\
         PROGRAM Main\nVAR r : REF_TO Point; END_VAR\n  r := REF(g);\nEND_PROGRAM\n\
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
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR PUBLIC
  cv : DINT;
  label : STRING[8];
END_VAR
  cv := cv + inc;
  label := 'fb';
  q := cv;
END_FUNCTION_BLOCK

VAR_GLOBAL
  seed : DINT := 5;
  result : DINT;
  Calls : DINT;
  cycles : DINT;
END_VAR
VAR_GLOBAL RETAIN
  Saved : DINT;
  SavedPoint : Point;
  SavedPoints : ARRAY[1..2] OF Point;
  SavedText : STRING[8];
  SavedFb : Counter;
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
FUNCTION SetAgg : DINT
  SavedPoint.x := 11;
  SavedPoint.y := 22;
  SavedPoints[1].x := 1;
  SavedPoints[1].y := 2;
  SavedPoints[2].x := 3;
  SavedPoints[2].y := 4;
  SavedText := 'agg';
  SetAgg := 0;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetAgg : DINT
VAR
  total : DINT;
END_VAR
  total := SavedPoint.x + SavedPoint.y;
  total := total + SavedPoints[1].x + SavedPoints[1].y;
  total := total + SavedPoints[2].x + SavedPoints[2].y;
  total := total + LEN(SavedText);
  GetAgg := total;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION ClearAgg : DINT
  SavedPoint.x := 0;
  SavedPoint.y := 0;
  SavedPoints[1].x := 0;
  SavedPoints[1].y := 0;
  SavedPoints[2].x := 0;
  SavedPoints[2].y := 0;
  SavedText := '';
  ClearAgg := 0;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION BumpFb : DINT
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  q : DINT;
END_VAR
  SavedFb(inc := inc, q => q);
  BumpFb := q;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION GetFb : DINT
  GetFb := SavedFb.cv + LEN(SavedFb.label);
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
VAR RETAIN
  sp : Point;
  st : STRING[8];
END_VAR
  cycles := cycles + 1;
  sp.x := sp.x + 1;
  sp.y := sp.y + 1;
  st := 'p';
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
    for path in [
        "\"path\": \"Saved\"",
        "\"path\": \"SavedPoint.x\"",
        "\"path\": \"SavedPoints[1].y\"",
        "\"path\": \"SavedText\"",
        "\"path\": \"SavedFb.CV\"",
        "\"path\": \"SavedFb.label\"",
        "\"path\": \"inst.sp.x\"",
        "\"path\": \"inst.st\"",
    ] {
        assert!(metadata.contains(path), "{path} missing: {metadata}");
    }
    assert!(metadata.contains("\"size\": 84"), "{metadata}");
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
        "TYPE\n  Point : STRUCT\n    x : DINT;\n    y : DINT;\n  END_STRUCT;\nEND_TYPE\n\
         VAR_GLOBAL\n  g : Point;\nEND_VAR\n\
         PROGRAM Main\nVAR r : REF_TO Point; END_VAR\n  r := REF(g);\nEND_PROGRAM\n\
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

CLASS Base
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := x + 1;
END_METHOD
END_CLASS

CLASS Derived EXTENDS Base
METHOD PUBLIC OVERRIDE M : DINT
  M := SUPER.M() + 10;
END_METHOD
METHOD PUBLIC N : DINT
  N := M() + THIS.x;
END_METHOD
END_CLASS

FUNCTION_BLOCK Counter
VAR_INPUT
  inc : DINT;
END_VAR
VAR_OUTPUT
  q : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK

{stc:export}{stc:no_mangle}
FUNCTION MathTest : DINT
  MathTest := ABS(-5) + TRUNC(SQRT(9.0)) + TRUNC(EXPT(2.0, 3.0));
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION Tick : DINT
VAR
  q : DINT;
END_VAR
  gc(inc := 1, q => q);
  Tick := q;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunOop : DINT
VAR
  c : Counter;
  d : Derived;
  d2 : Derived;
  v : DINT;
END_VAR
  c(inc := 2, q => v);
  c(inc := 3, q => v);
  d.x := 4;
  v := v + d.M() + d.N();
  d2 := d;
  v := v + d2.M();
  RunOop := v;
END_FUNCTION

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
  gc : Counter;
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

{stc:export}{stc:no_mangle}
FUNCTION StringOps : DINT
VAR
  s : STRING[20];
  r : DINT;
END_VAR
  s := 'abcdef';
  r := 0;
  IF LEFT(s, 3) = 'abc' THEN r := r + 1; END_IF;
  IF RIGHT(s, 2) = 'ef' THEN r := r + 10; END_IF;
  IF MID(s, 3, 2) = 'bcd' THEN r := r + 100; END_IF;
  IF INSERT(s, 'XY', 3) = 'abXYcdef' THEN r := r + 1000; END_IF;
  IF DELETE(s, 2, 3) = 'abef' THEN r := r + 10000; END_IF;
  IF REPLACE(s, 'XY', 2, 3) = 'abXYef' THEN r := r + 100000; END_IF;
  IF FIND(s, 'cd') = 3 THEN r := r + 1000000; END_IF;
  IF FIND(s, 'zz') = 0 THEN r := r + 10000000; END_IF;
  IF FIND(s, '') = 1 THEN r := r + 100000000; END_IF;
  IF LEFT(s, 0) = '' THEN r := r + 1000000000; END_IF;
  StringOps := r;
END_FUNCTION

FUNCTION Sum2 : DINT
VAR_IN_OUT
  a : ARRAY[*, *] OF DINT;
END_VAR
VAR
  i : DINT;
  j : DINT;
END_VAR
  Sum2 := 0;
  FOR i := 1 TO 2 DO
    FOR j := 1 TO 3 DO
      Sum2 := Sum2 + a[i, j];
    END_FOR;
  END_FOR;
END_FUNCTION

FUNCTION Fill2 : DINT
VAR_IN_OUT
  a : ARRAY[*, *] OF DINT;
END_VAR
  a[2, 3] := 99;
  Fill2 := a[2, 3];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunConf2 : DINT
VAR
  m : ARRAY[1..2, 1..3] OF DINT;
  i : DINT;
  j : DINT;
END_VAR
  FOR i := 1 TO 2 DO
    FOR j := 1 TO 3 DO
      m[i, j] := i * 10 + j;
    END_FOR;
  END_FOR;
  RunConf2 := Sum2(m) + Fill2(m) + m[2, 3];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RefAgg : DINT
VAR
  p : Point;
  q : Point;
  r : REF_TO Point;
END_VAR
  p := Make(1, 2);
  r := REF(p);
  r^ := Make(3, 4);
  q := r^;
  RefAgg := p.x * 1000 + q.y;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION WideOps : DINT
VAR
  w : WSTRING[10];
  r : DINT;
END_VAR
  w := WSTRING#'wide';
  r := 0;
  IF LEFT(w, 2) = WSTRING#'wi' THEN r := r + 1; END_IF;
  IF MID(w, 2, 3) = WSTRING#'de' THEN r := r + 10; END_IF;
  IF FIND(w, WSTRING#'de') = 3 THEN r := r + 100; END_IF;
  WideOps := r;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION UnicodeOps : DINT
VAR
  u : USTRING[20];
  r : DINT;
END_VAR
  u := USTRING#'aé😀z';
  r := 0;
  IF LEN(u) = 4 THEN r := r + 1; END_IF;
  IF LEN_CODE_UNIT(u) = 8 THEN r := r + 10; END_IF;
  IF LEFT(u, 2) = USTRING#'aé' THEN r := r + 100; END_IF;
  IF MID(u, 1, 3) = USTRING#'😀' THEN r := r + 1000; END_IF;
  IF RIGHT(u, 2) = USTRING#'😀z' THEN r := r + 10000; END_IF;
  IF FIND(u, USTRING#'😀') = 3 THEN r := r + 100000; END_IF;
  UnicodeOps := r;
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

const STANDARD_FBS: &str = r#"
VAR_GLOBAL
  ton1 : TON;
  tof1 : TOF;
  tp1 : TP;
  rt1 : R_TRIG;
  ft1 : F_TRIG;
  cu1 : CTU;
  cd1 : CTD;
  cud1 : CTUD;
  sr1 : SR;
  rs1 : RS;
END_VAR

{stc:export}{stc:no_mangle}
FUNCTION FbStep : BOOL
VAR_INPUT
  which : DINT;
  inp : BOOL;
  aux : BOOL;
END_VAR
  FbStep := FALSE;
  CASE which OF
    0:
      ton1(IN := inp, PT := T#100ns);
      FbStep := ton1.Q;
    1:
      tof1(IN := inp, PT := T#100ns);
      FbStep := tof1.Q;
    2:
      tp1(IN := inp, PT := T#100ns);
      FbStep := tp1.Q;
    3:
      rt1(CLK := inp);
      FbStep := rt1.Q;
    4:
      ft1(CLK := inp);
      FbStep := ft1.Q;
    5:
      cu1(CU := inp, R := aux, PV := 3);
      FbStep := cu1.Q;
    6:
      cd1(CD := inp, LD := aux, PV := 3);
      FbStep := cd1.Q;
    7:
      cud1(CU := inp, CD := FALSE, R := aux, LD := FALSE, PV := 3);
      FbStep := cud1.QU;
    8:
      sr1(SET1 := inp, RESET := aux);
      FbStep := sr1.Q;
    9:
      rs1(SET := inp, RESET1 := aux);
      FbStep := rs1.Q;
    10:
      cud1(CU := FALSE, CD := inp, R := aux, LD := FALSE, PV := 3);
      FbStep := cud1.QD;
  ELSE
    FbStep := FALSE;
  END_CASE;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION FbET : TIME
VAR_INPUT
  which : DINT;
END_VAR
  CASE which OF
    0: FbET := ton1.ET;
    1: FbET := tof1.ET;
    2: FbET := tp1.ET;
  ELSE
    FbET := T#0ns;
  END_CASE;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION FbCV : INT
VAR_INPUT
  which : DINT;
END_VAR
  CASE which OF
    5: FbCV := cu1.CV;
    6: FbCV := cd1.CV;
    7: FbCV := cud1.CV;
    10: FbCV := cud1.CV;
  ELSE
    FbCV := 0;
  END_CASE;
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
fn pointer_operations_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("pointers");
    build_library(&directory, POINTERS, "pointers");
    let library = lib_path(&directory.path, "pointers");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "pointers.c",
        &library,
        &built_module(&directory, "pointers"),
        "pointers_harness",
    );
}

const POINTERS: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
  Node : STRUCT
    value : DINT;
    next : POINTER TO Node;
  END_STRUCT;
END_TYPE

VAR_GLOBAL
  g : DINT;
  gpt : Point;
  g2 : ARRAY[1..2, 1..3] OF DINT;
END_VAR

FUNCTION ReadP : DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  ReadP := p^;
END_FUNCTION

FUNCTION BumpP : DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  p^ := p^ + 1;
  BumpP := p^;
END_FUNCTION

FUNCTION GetNext : DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
VAR_OUTPUT
  q : POINTER TO DINT;
END_VAR
  q := p + 1;
  GetNext := q^;
END_FUNCTION

FUNCTION BumpPoint : DINT
VAR_IN_OUT
  a : Point;
END_VAR
  a.x := a.x + 1;
  BumpPoint := a.x;
END_FUNCTION

FUNCTION NextP : POINTER TO DINT
VAR_INPUT
  p : POINTER TO DINT;
END_VAR
  NextP := p + 1;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrGlobal : DINT
VAR
  p : POINTER TO DINT;
END_VAR
  g := 5;
  p := ADR(g);
  p^ := 7;
  PtrGlobal := g + p^;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrIndex : DINT
VAR
  arr : ARRAY[1..4] OF DINT;
  p : POINTER TO DINT;
  i : DINT;
END_VAR
  p := ADR(arr);
  FOR i := 0 TO 3 DO
    p[i] := i + 1;
  END_FOR;
  PtrIndex := arr[1] * 10 + arr[4];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrArith : DINT
VAR
  arr : ARRAY[1..4] OF DINT;
  p : POINTER TO DINT;
  q : POINTER TO DINT;
  n : DINT;
END_VAR
  arr[1] := 1;
  arr[2] := 2;
  arr[3] := 9;
  arr[4] := 4;
  p := ADR(arr);
  q := 2 + p;
  n := q - p;
  PtrArith := n * 10 + q^;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrParam : DINT
VAR
  arr : ARRAY[1..2] OF DINT;
  p : POINTER TO DINT;
  q : POINTER TO DINT;
  v : DINT;
END_VAR
  arr[1] := 3;
  arr[2] := 4;
  p := ADR(arr);
  v := ReadP(p);
  v := v + BumpP(p);
  v := v + GetNext(p, q => q);
  v := v + q^;
  PtrParam := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrResult : DINT
VAR
  arr : ARRAY[1..2] OF DINT;
  p : POINTER TO DINT;
END_VAR
  arr[1] := 5;
  arr[2] := 6;
  p := NextP(ADR(arr));
  PtrResult := p^;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrConv : DINT
VAR
  p : POINTER TO DINT;
  r : REF_TO DINT;
  rp : REF_TO DINT;
END_VAR
  r := REF(g);
  p := POINTER TO DINT(r);
  p^ := 11;
  rp := REF_TO DINT(p);
  rp^ := 12;
  PtrConv := g;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrSize : DINT
VAR
  p : POINTER TO DINT;
END_VAR
  PtrSize := SIZEOF(DINT) + SIZEOF(p) + SIZEOF(p^)
    + SIZEOF(STRING[10]) + SIZEOF(ARRAY[1..3] OF INT);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrString : DINT
VAR
  s : STRING[10];
  ps : POINTER TO STRING[10];
  r : DINT;
END_VAR
  s := 'abc';
  ps := ADR(s);
  r := 0;
  IF ps^[1] = CHAR#'a' THEN r := r + 10; END_IF;
  IF ps^ = 'abc' THEN r := r + 1; END_IF;
  PtrString := r + LEN(ps^);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrArray2 : DINT
VAR
  p : POINTER TO ARRAY[1..3] OF DINT;
END_VAR
  p := ADR(g2);
  p[1][3] := 7;
  PtrArray2 := g2[2, 3];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrField : DINT
VAR
  pt : Point;
  p : POINTER TO Point;
  n : Node;
  other : Node;
END_VAR
  p := ADR(pt);
  p^.x := 5;
  p^.y := 6;
  other.value := 9;
  n.next := ADR(other);
  PtrField := pt.x * 10 + pt.y + n.next^.value;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrPtr : DINT
VAR
  p : POINTER TO DINT;
  pp : POINTER TO POINTER TO DINT;
END_VAR
  p := ADR(g);
  pp := ADR(p);
  pp^^ := 8;
  PtrPtr := g;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrDirect : DINT
VAR
  p : POINTER TO DWORD;
END_VAR
  p := ADR(%MD10);
  p^ := 42;
  IF %MD10 = DWORD#42 THEN
    PtrDirect := 42;
  ELSE
    PtrDirect := 0;
  END_IF;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrArrayOfPtrs : DINT
VAR
  buf : ARRAY[1..3] OF DINT;
  ptrs : ARRAY[1..2] OF POINTER TO DINT;
  p : POINTER TO DINT;
END_VAR
  ptrs[1] := ADR(buf[2]);
  p := ptrs[1];
  p^ := 9;
  PtrArrayOfPtrs := buf[2];
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrNull : DINT
VAR
  p : POINTER TO DINT;
  r : REF_TO DINT;
  rp : REF_TO DINT;
  n : DINT;
END_VAR
  p := NULL;
  n := 0;
  IF p = NULL THEN n := 1; ELSE n := 2; END_IF;
  IF p <> NULL THEN n := n + 10; END_IF;
  r := REF_TO DINT(p);
  IF r = NULL THEN n := n + 100; END_IF;
  rp := r;
  p := POINTER TO DINT(rp);
  IF p = NULL THEN n := n + 1000; END_IF;
  PtrNull := n;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrInOut : DINT
VAR
  pt : Point;
  p : POINTER TO Point;
  v : DINT;
END_VAR
  pt.x := 1;
  pt.y := 2;
  p := ADR(pt);
  v := BumpPoint(p^);
  PtrInOut := v * 10 + pt.x;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION PtrLeaf : DINT
VAR
  p : POINTER TO DINT;
END_VAR
  p := ADR(gpt.y);
  p^ := 7;
  PtrLeaf := gpt.y;
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
fn standard_function_blocks_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("stdlib-fbs");
    build_library(&directory, STANDARD_FBS, "std_fbs");
    let library = lib_path(&directory.path, "std_fbs");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "stdlib.c",
        &library,
        &built_module(&directory, "std_fbs"),
        "stdlib_harness",
    );
}

const OOP_OBJECTS: &str = r#"
CLASS Inner
VAR PUBLIC
  x : DINT;
END_VAR
METHOD PUBLIC Bump : DINT
  x := x + 1;
  Bump := x;
END_METHOD
END_CLASS

CLASS Base
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
METHOD PUBLIC Mix : DINT
VAR_INPUT
  o : Base;
END_VAR
  Mix := o.M() + id;
END_METHOD
METHOD PUBLIC Self : Base
  Self := THIS;
END_METHOD
METHOD PUBLIC Give : DINT
VAR_OUTPUT
  o : Base;
END_VAR
  o := gD;
  Give := 3;
END_METHOD
END_CLASS

CLASS ABSTRACT Shape
METHOD PUBLIC ABSTRACT Area : DINT
END_METHOD
END_CLASS

CLASS Square EXTENDS Shape
VAR PUBLIC
  side : DINT;
END_VAR
METHOD PUBLIC OVERRIDE Area : DINT
  Area := side * side;
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

FUNCTION_BLOCK FB
VAR_INPUT PUBLIC
  inc : DINT;
END_VAR
VAR_OUTPUT PUBLIC
  q : DINT;
END_VAR
VAR PUBLIC
  cv : DINT;
END_VAR
METHOD PUBLIC GetCv : DINT
  GetCv := cv;
END_METHOD
  cv := cv + inc;
  q := cv;
END_FUNCTION_BLOCK








FUNCTION_BLOCK BaseFb
VAR_INPUT PUBLIC
  delta : DINT;
END_VAR
VAR_OUTPUT PUBLIC
  val : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + delta;
  val := cv;
END_FUNCTION_BLOCK

FUNCTION_BLOCK DerivedFb EXTENDS BaseFb
VAR_INPUT PUBLIC
  mult : DINT;
END_VAR
  SUPER();
  val := val * mult;
END_FUNCTION_BLOCK

CLASS Holder
VAR PUBLIC
  inner : Inner;
  fb : FB;
  base : Base;
END_VAR
METHOD PUBLIC Sum : DINT
  fb(inc := 1);
  Sum := inner.Bump() + fb.q + base.M();
END_METHOD
END_CLASS

CLASS Pool
VAR PUBLIC
  items : ARRAY[1..3] OF Base;
END_VAR
METHOD PUBLIC Total : DINT
VAR
  i : DINT;
  acc : DINT;
END_VAR
  FOR i := 1 TO 3 DO
    acc := acc + items[i].M();
  END_FOR;
  Total := acc;
END_METHOD
END_CLASS

CLASS Rec
VAR PUBLIC
  sink : DINT;
END_VAR
METHOD PUBLIC Sum : DINT
VAR_INPUT
  n : DINT;
END_VAR
  IF n <= 0 THEN
    Sum := 0;
  ELSE
    Sum := n + THIS.Sum(n - 1);
  END_IF;
END_METHOD
END_CLASS

TYPE
  Bundle : STRUCT
    obj : Base;
    fb : FB;
  END_STRUCT;
END_TYPE

VAR_GLOBAL
  gD : Derived;
  gh : Holder;
  gb : Bundle;
  gRun : DINT;
END_VAR

FUNCTION UseBase : DINT
VAR_INPUT
  b : Base;
END_VAR
  UseBase := b.M();
END_FUNCTION

FUNCTION Rebind : DINT
VAR_OUTPUT
  o : Base;
END_VAR
  o := gD;
  Rebind := 0;
END_FUNCTION

FUNCTION SwapHandle : DINT
VAR_IN_OUT
  o : Base;
END_VAR
  o := gD;
  SwapHandle := o.id;
END_FUNCTION

FUNCTION RebindIn : DINT
VAR_INPUT
  b : Base;
END_VAR
  b := NULL;
  RebindIn := 1;
  IF b = NULL THEN
    RebindIn := 2;
  END_IF;
END_FUNCTION

FUNCTION Pick : Base
VAR_INPUT
  useGlobal : BOOL;
END_VAR
  IF useGlobal THEN
    Pick := gD;
  ELSE
    Pick := NULL;
  END_IF;
END_FUNCTION

FUNCTION StepFb : DINT
VAR_IN_OUT
  f : FB;
END_VAR
  f(inc := 2);
  StepFb := f.q;
END_FUNCTION

FUNCTION UseFb : DINT
VAR_INPUT
  f : FB;
END_VAR
  f(inc := 5);
  UseFb := f.q;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunBasic : DINT
VAR
  d : Derived;
  b : Base;
  v : DINT;
END_VAR
  d.id := 1;
  d.extra := 5;
  b := d;
  v := b.M();
  b.id := 2;
  v := v + d.M();
  b.id := 40;
  v := v + UseBase(d);
  gD.id := 9;
  Rebind(o => b);
  v := v + b.M();
  RunBasic := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunCopy : DINT
VAR
  a : Derived;
  b : Derived;
  c : Derived;
  v : DINT;
END_VAR
  a.id := 1;
  a.extra := 2;
  b.id := 10;
  b.extra := 20;
  c := b;
  b := a;
  v := c.M();
  v := v + b.M();
  c.id := 7;
  v := v + c.M();
  RunCopy := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunArray : DINT
VAR
  items : ARRAY[1..3] OF Derived;
  b : Base;
  i : DINT;
  v : DINT;
END_VAR
  FOR i := 1 TO 3 DO
    items[i].id := i;
    items[i].extra := i * 10;
  END_FOR;
  FOR i := 1 TO 3 DO
    b := items[i];
    v := v + b.M();
  END_FOR;
  items[2].id := 100;
  v := v + items[2].M();
  RunArray := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunStruct : DINT
VAR
  s : Bundle;
  d : Derived;
  v : DINT;
END_VAR
  d.id := 2;
  d.extra := 8;
  s.obj := d;
  v := s.obj.M();
  s.fb(inc := 3);
  v := v + s.fb.q;
  s.obj.id := 4;
  v := v + d.M();
  RunStruct := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunGlobalStruct : DINT
VAR
  v : DINT;
END_VAR
  gD.id := 6;
  gD.extra := 1;
  gb.obj := gD;
  v := gb.obj.M();
  gb.fb(inc := 4);
  v := v + gb.fb.q;
  RunGlobalStruct := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunAbstract : DINT
VAR
  s : Square;
  b : Shape;
END_VAR
  s.side := 5;
  b := s;
  RunAbstract := b.Area();
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunFbSuper : DINT
VAR
  d : DerivedFb;
  v : DINT;
END_VAR
  d.delta := 2;
  d.mult := 3;
  d();
  v := d.val;
  d();
  v := d.val;
  RunFbSuper := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunPool : DINT
VAR
  p : Pool;
  d : Derived;
END_VAR
  d.id := 1;
  d.extra := 2;
  p.items[1] := d;
  p.items[2].id := 4;
  p.items[3].id := 5;
  RunPool := p.Total();
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunNested : DINT
VAR
  h : Holder;
  v : DINT;
END_VAR
  h.base.id := 9;
  v := h.Sum();
  v := v + h.inner.Bump();
  v := v + h.base.M();
  RunNested := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunGlobal : DINT
  gh.base.id := 7;
  RunGlobal := gh.Sum() + gh.inner.Bump() + gh.base.M() + gh.fb.GetCv();
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunRec : DINT
VAR
  r : Rec;
END_VAR
  RunRec := r.Sum(10);
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunFb : DINT
VAR
  f : FB;
  g : FB;
  v : DINT;
END_VAR
  v := UseFb(f);
  v := v + f.q;
  v := v + StepFb(f);
  v := v + StepFb(f);
  g := f;
  v := v + StepFb(g);
  v := v + g.q;
  v := v + g.inc;
  v := v + f.cv;
  v := v + f.GetCv();
  RunFb := v;
END_FUNCTION

FUNCTION ZeroFb : DINT
VAR_OUTPUT
  o : FB;
END_VAR
  o.cv := 0;
  o.q := 0;
  o.inc := 0;
  ZeroFb := 7;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunFbOut : DINT
VAR
  f : FB;
  v : DINT;
END_VAR
  v := StepFb(f);
  v := v + ZeroFb(o => f);
  v := v + f.GetCv() * 10 + f.q + f.cv;
  RunFbOut := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunParam : DINT
VAR
  d : Derived;
  b : Base;
  v : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  v := UseBase(d);
  v := v + RebindIn(d);
  v := v + d.M();
  b := d;
  v := v + UseBase(b);
  v := v + d.Mix(b);
  gD.id := 5;
  v := v + SwapHandle(b);
  v := v + b.id;
  v := v + d.Give(o => b);
  v := v + b.id;
  v := v + UseBase(d.Self());
  RunParam := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunNull : DINT
VAR
  b : Base;
  v : DINT;
END_VAR
  b := Pick(FALSE);
  IF b = NULL THEN
    v := 3;
  END_IF;
  gD.id := 30;
  b := Pick(TRUE);
  IF b <> NULL THEN
    v := v + b.M();
  END_IF;
  RunNull := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION CheckMain : DINT
  CheckMain := gRun;
END_FUNCTION

PROGRAM Main
VAR
  d : Derived;
  b : Base;
  f : FB;
  s : Bundle;
END_VAR
  d.id := 4;
  d.extra := 6;
  b := d;
  f(inc := 2);
  s.obj := d;
  gRun := b.M() + f.q + s.obj.M();
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn object_records_and_virtual_dispatch_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("oop");
    build_library(&directory, OOP_OBJECTS, "oop");
    let library = lib_path(&directory.path, "oop");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "oop.c",
        &library,
        &built_module(&directory, "oop"),
        "oop_harness",
    );
}

const INTERFACE_OBJECTS: &str = r#"
INTERFACE IBase
METHOD PUBLIC Base : DINT
END_METHOD
METHOD PUBLIC Scale : DINT
VAR_INPUT
  factor : DINT;
END_VAR
END_METHOD
METHOD PUBLIC Fill : DINT
VAR_OUTPUT
  o : DINT;
END_VAR
END_METHOD
END_INTERFACE

INTERFACE IDerived EXTENDS IBase
METHOD PUBLIC More : DINT
END_METHOD
END_INTERFACE

CLASS BaseImpl IMPLEMENTS IBase
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := id;
END_METHOD
METHOD PUBLIC Scale : DINT
VAR_INPUT
  factor : DINT;
END_VAR
  Scale := id * factor;
END_METHOD
METHOD PUBLIC Fill : DINT
VAR_OUTPUT
  o : DINT;
END_VAR
  o := id + 1;
  Fill := id;
END_METHOD
METHOD PUBLIC AsInterface : IBase
  AsInterface := THIS;
END_METHOD
END_CLASS

CLASS DerivedImpl EXTENDS BaseImpl IMPLEMENTS IDerived
VAR PUBLIC
  extra : DINT;
END_VAR
METHOD PUBLIC OVERRIDE Base : DINT
  Base := SUPER.Base() * 10;
END_METHOD
METHOD PUBLIC More : DINT
  More := extra;
END_METHOD
END_CLASS

FUNCTION_BLOCK FbImpl IMPLEMENTS IBase
VAR PUBLIC
  cv : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := cv;
END_METHOD
METHOD PUBLIC Scale : DINT
VAR_INPUT
  factor : DINT;
END_VAR
  Scale := cv * factor;
END_METHOD
METHOD PUBLIC Fill : DINT
VAR_OUTPUT
  o : DINT;
END_VAR
  o := cv + 1;
  Fill := cv;
END_METHOD
END_FUNCTION_BLOCK

CLASS Registry
VAR PUBLIC
  slot : IBase;
  slots : ARRAY[1..2] OF IBase;
END_VAR
METHOD PUBLIC Sum : DINT
  Sum := slot.Base() + slots[1].Base() + slots[2].Base();
END_METHOD
METHOD PUBLIC SumIndexed : DINT
VAR
  i : DINT;
  acc : DINT;
END_VAR
  FOR i := 1 TO 2 DO
    acc := acc + slots[i].Base();
  END_FOR;
  SumIndexed := acc;
END_METHOD
END_CLASS

TYPE
  Holder : STRUCT
    a : IBase;
    b : IBase;
  END_STRUCT;
END_TYPE

VAR_GLOBAL
  gD : DerivedImpl;
  gB : BaseImpl;
  gI : IBase;
  gReg : Registry;
  gH : Holder;
  gArr : ARRAY[1..2] OF IBase;
  gRun : DINT;
END_VAR

FUNCTION UseBase : DINT
VAR_INPUT
  i : IBase;
END_VAR
  UseBase := i.Base();
END_FUNCTION

FUNCTION SumAll : DINT
VAR_INPUT
  items : ARRAY[1..2] OF IBase;
END_VAR
  SumAll := items[1].Base() + items[2].Base();
END_FUNCTION

FUNCTION Pick : IBase
VAR_INPUT
  useDerived : BOOL;
END_VAR
  IF useDerived THEN
    Pick := gD;
  ELSE
    Pick := gB;
  END_IF;
END_FUNCTION

FUNCTION Identity : IBase
VAR_INPUT
  i : IBase;
END_VAR
  Identity := i;
END_FUNCTION

FUNCTION Promote : IBase
VAR_INPUT
  b : BaseImpl;
END_VAR
  Promote := b;
END_FUNCTION

FUNCTION IsNull : BOOL
VAR_INPUT
  i : IBase;
END_VAR
  IsNull := i = NULL;
END_FUNCTION

FUNCTION Swap : DINT
VAR_IN_OUT
  i : IBase;
END_VAR
  i := gB;
  Swap := i.Base();
END_FUNCTION

FUNCTION Clear : DINT
VAR_OUTPUT
  o : IBase;
END_VAR
  o := gD;
  Clear := 1;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunDispatch : DINT
VAR
  d : DerivedImpl;
  b : BaseImpl;
  i : IBase;
  base : IBase;
  derived : IDerived;
  x : DINT;
  v : DINT;
END_VAR
  d.id := 1;
  d.extra := 2;
  b.id := 5;
  i := d;
  v := i.Base();
  i := b;
  v := v + i.Base();
  v := v + UseBase(d);
  v := v + UseBase(b);
  gD.id := 2;
  gB.id := 4;
  v := v + Pick(TRUE).Base();
  v := v + Pick(FALSE).Base();
  derived := d;
  base := derived;
  v := v + base.Base();
  v := v + derived.More();
  v := v + derived.Scale(2);
  v := v + Identity(d).Base();
  v := v + b.AsInterface().Base();
  v := v + Promote(b).Base();
  v := v + Swap(i);
  v := v + Clear(o => i);
  v := v + i.Base();
  v := v + i.Scale(3);
  v := v + i.Fill(o => x);
  v := v + x;
  RunDispatch := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunNull : DINT
VAR
  i : IBase;
  b : BaseImpl;
  n : DINT;
END_VAR
  i := NULL;
  n := 0;
  IF i = NULL THEN
    n := n + 1;
  END_IF;
  IF i <> NULL THEN
    n := n + 10;
  END_IF;
  b := NULL;
  i := b;
  IF i = NULL THEN
    n := n + 100;
  END_IF;
  IF IsNull(NULL) THEN
    n := n + 1000;
  END_IF;
  IF IsNull(gD) THEN
    n := n + 10;
  END_IF;
  RunNull := n;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunArrays : DINT
VAR
  arr : ARRAY[1..2] OF IBase;
  d : DerivedImpl;
  b : BaseImpl;
  h : Holder;
  v : DINT;
END_VAR
  d.id := 3;
  d.extra := 1;
  b.id := 7;
  arr[1] := d;
  arr[2] := b;
  v := arr[1].Base() + arr[2].Base();
  v := v + SumAll(arr);
  h.a := d;
  h.b := b;
  v := v + h.a.Base() + h.b.Base();
  gD.id := 2;
  gI := gD;
  v := v + gI.Base();
  RunArrays := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunFields : DINT
VAR
  r : Registry;
  d : DerivedImpl;
  b : BaseImpl;
END_VAR
  d.id := 8;
  d.extra := 1;
  b.id := 9;
  r.slot := d;
  r.slots[1] := d;
  r.slots[2] := b;
  RunFields := r.Sum() + r.SumIndexed();
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunIndex : DINT
VAR
  i : DINT;
  v : DINT;
END_VAR
  gD.id := 3;
  FOR i := 1 TO 2 DO
    gArr[i] := gD;
  END_FOR;
  FOR i := 1 TO 2 DO
    v := v + gArr[i].Base();
  END_FOR;
  RunIndex := v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunFb : DINT
VAR
  f : FbImpl;
  i : IBase;
  v : DINT;
END_VAR
  f.cv := 9;
  i := f;
  RunFb := i.Base() + UseBase(f) + i.Scale(2) + i.Fill(o => v);
  RunFb := RunFb + v;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION CheckMain : DINT
  CheckMain := gRun;
END_FUNCTION

PROGRAM Main
VAR
  v : DINT;
  held : IBase;
END_VAR
  gD.id := 6;
  held := gD;
  gReg.slot := gD;
  gH.a := gD;
  gArr[1] := gD;
  gI := gD;
  v := held.Base() + gI.Base() + gReg.slot.Base() + gH.a.Base() + gArr[1].Base();
  gRun := v;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn interface_values_and_dispatch_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("interfaces");
    build_library(&directory, INTERFACE_OBJECTS, "interfaces");
    let library = lib_path(&directory.path, "interfaces");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "interfaces.c",
        &library,
        &built_module(&directory, "interfaces"),
        "interfaces_harness",
    );
}

const OBJECT_ATTEMPTS: &str = r#"
INTERFACE IBase
METHOD Base : DINT
END_METHOD
END_INTERFACE

INTERFACE IDerived EXTENDS IBase
METHOD More : DINT
END_METHOD
END_INTERFACE

CLASS BaseImpl IMPLEMENTS IBase
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := id;
END_METHOD
END_CLASS

CLASS DerivedImpl EXTENDS BaseImpl IMPLEMENTS IDerived
VAR PUBLIC
  extra : DINT;
END_VAR
METHOD PUBLIC OVERRIDE Base : DINT
  Base := SUPER.Base() + extra;
END_METHOD
METHOD PUBLIC More : DINT
  More := extra;
END_METHOD
END_CLASS

CLASS OtherImpl IMPLEMENTS IBase
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := id;
END_METHOD
END_CLASS

FUNCTION_BLOCK FbImpl IMPLEMENTS IBase
VAR PUBLIC
  cv : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := cv;
END_METHOD
END_FUNCTION_BLOCK

{stc:export}{stc:no_mangle}
FUNCTION RunAttempts : DINT
VAR
  d : DerivedImpl;
  b : BaseImpl;
  o : OtherImpl;
  f : FbImpl;
  a : BaseImpl;
  x : DerivedImpl;
  i : IBase;
  di : IDerived;
  n : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  b.id := 2;
  o.id := 9;
  f.cv := 5;
  n := 0;

  a := NULL;
  a ?= d;
  IF a <> NULL THEN n := n + a.Base(); END_IF;

  x := NULL;
  x ?= b;
  IF x = NULL THEN n := n + 100; END_IF;

  x ?= d;
  IF x <> NULL THEN n := n + x.More(); END_IF;

  i := o;
  x := NULL;
  x ?= i;
  IF x = NULL THEN n := n + 1000; END_IF;

  i := NULL;
  i ?= d;
  IF i <> NULL THEN n := n + i.Base(); END_IF;

  i := NULL;
  i ?= f;
  IF i <> NULL THEN n := n + i.Base(); END_IF;

  di := NULL;
  di ?= d;
  i := NULL;
  i ?= di;
  IF i <> NULL THEN n := n + i.Base(); END_IF;

  di := NULL;
  i := o;
  di ?= i;
  IF di = NULL THEN n := n + 10000; END_IF;

  x := NULL;
  x ?= di;
  IF x = NULL THEN n := n + 100000; END_IF;

  di ?= d;
  x := NULL;
  x ?= di;
  IF x <> NULL THEN n := n + x.More(); END_IF;

  a := d;
  a ?= NULL;
  IF a = NULL THEN n := n + 1000000; END_IF;

  i := NULL;
  i ?= i;
  IF i = NULL THEN n := n + 10000000; END_IF;

  RunAttempts := n;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunObjectRefs : DINT
VAR
  d : DerivedImpl;
  x : DerivedImpl;
  f : FbImpl;
  f2 : FbImpl;
  r : REF_TO DerivedImpl;
  rb : REF_TO DerivedImpl;
  rf : REF_TO FbImpl;
  n : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  f.cv := 5;
  f2.cv := 6;
  n := 0;

  r := REF(d);
  n := n + r^.Base();
  r^.id := 11;
  r^.extra := 12;
  n := n + d.Base();

  x.id := 21;
  x.extra := 22;
  r^ := x;
  n := n + d.Base();
  n := n + x.Base();
  x.id := 31;
  n := n + d.id;
  n := n + x.Base();

  rb := REF(d);
  n := n + rb^.id;
  d := NULL;
  rb := REF(d);
  IF rb = NULL THEN n := n + 1000; END_IF;

  rf := REF(f);
  n := n + rf^.Base();
  rf^ := f2;
  n := n + f.Base();
  rf^.cv := 8;
  n := n + f.cv;

  RunObjectRefs := n;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunRefInterface : DINT
VAR
  d : DerivedImpl;
  o : OtherImpl;
  r : REF_TO DerivedImpl;
  ro : REF_TO OtherImpl;
  i : IBase;
  di : IDerived;
  n : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  o.id := 9;
  n := 0;

  r := REF(d);
  i := NULL;
  i := r;
  IF i <> NULL THEN n := n + i.Base(); END_IF;

  di := NULL;
  di := r;
  IF di <> NULL THEN n := n + di.More(); END_IF;

  r := NULL;
  i := d;
  i := r;
  IF i = NULL THEN n := n + 100; END_IF;

  ro := REF(o);
  di := d;
  di := ro;
  IF di = NULL THEN n := n + 1000; END_IF;

  r := REF(d);
  i := NULL;
  i ?= r;
  IF i <> NULL THEN n := n + i.Base(); END_IF;

  ro := REF(o);
  di := d;
  di ?= ro;
  IF di = NULL THEN n := n + 100000; END_IF;

  RunRefInterface := n;
END_FUNCTION

{stc:export}{stc:no_mangle}
FUNCTION RunRefAttempts : DINT
VAR
  d : DerivedImpl;
  b : BaseImpl;
  r : REF_TO DerivedImpl;
  s : REF_TO BaseImpl;
  sb : REF_TO BaseImpl;
  n : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  b.id := 8;
  n := 0;

  r := REF(d);
  s := NULL;
  s ?= r;
  IF s <> NULL THEN n := n + s^.id; END_IF;
  n := n + s^.Base();

  r := NULL;
  r ?= s;
  IF r <> NULL THEN n := n + r^.More(); END_IF;

  sb := REF(b);
  r := NULL;
  r ?= sb;
  IF r = NULL THEN n := n + 100; END_IF;

  s := NULL;
  r ?= s;
  IF r = NULL THEN n := n + 1000; END_IF;

  RunRefAttempts := n;
END_FUNCTION

PROGRAM Main
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn assignment_attempts_and_object_references_run_under_a_c_harness() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("object_attempts");
    build_library(&directory, OBJECT_ATTEMPTS, "object_attempts");
    let library = lib_path(&directory.path, "object_attempts");
    assert!(library.is_file(), "{}", library.display());
    compile_harness(
        &directory,
        "object_attempts.c",
        &library,
        &built_module(&directory, "object_attempts"),
        "object_attempts_harness",
    );
}
