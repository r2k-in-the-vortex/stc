//! C-harness sample for the committed exact vectors (M6 phase 9).
//!
//! The test generates a library whose exported wrappers perform the selected
//! exact operations, writes the executable vector cases to a text file, and
//! runs `tests/harness/exact_vectors.c` against the generated module. It also
//! checks that checked arithmetic traps through a real module cycle.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use serde_json::{Map, Value};

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
    common::runtime_static()
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
                    eprintln!("note: clang not found; skipping exact-vector harness test");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("stc-exact-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, text).expect("write test file");
        path
    }
}

fn run_stc(args: &[&str]) -> Output {
    Command::new(binary()).args(args).output().expect("run stc")
}

fn lib_path(directory: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        directory.join(format!("{name}.lib"))
    } else {
        directory.join(format!("{name}.a"))
    }
}

fn built_module(directory: &Path, name: &str) -> String {
    let extension = if cfg!(windows) { "dll" } else { "so" };
    let prefix = format!("{name}-");
    let suffix = format!(".{extension}");
    std::fs::read_dir(directory)
        .expect("artifact directory")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|file| file.starts_with(&prefix) && file.ends_with(&suffix))
        .unwrap_or_else(|| panic!("module for `{name}` was not emitted"))
}

fn build_library(directory: &TestDir, source: &str, name: &str) {
    let source_path = directory.write(&format!("{name}.st"), source);
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
}

fn run_harness(directory: &TestDir, name: &str, module: &str, argument: &str) -> Output {
    let clang = clang().expect("clang required");
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/harness/exact_vectors.c");
    let object = directory.path.join(format!("{name}.o"));
    let compile = Command::new(clang)
        .arg(&harness)
        .args(["-c", "-o"])
        .arg(&object)
        .output()
        .expect("compile harness");
    assert!(
        compile.status.success(),
        "harness compile failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let exe = if cfg!(windows) {
        directory.path.join(format!("{name}.exe"))
    } else {
        directory.path.join(name)
    };
    let mut link = Command::new(clang);
    link.arg(&object)
        .arg(lib_path(&directory.path, "exact"))
        .args(["-o"])
        .arg(&exe);
    for argument in stc_driver::toolchain::system_link_args() {
        link.arg(argument);
    }
    let linked = link.output().expect("link harness");
    assert!(
        linked.status.success(),
        "harness link failed:\n{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    Command::new(&exe)
        .arg(module)
        .arg(argument)
        .output()
        .expect("run harness")
}

fn vectors_dir() -> PathBuf {
    workspace_root().join("tests/vectors/exact")
}

fn load_vector(name: &str) -> Value {
    let path = vectors_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("vector JSON")
}

fn cases(vector: &Value) -> &[Value] {
    vector
        .get("cases")
        .and_then(Value::as_array)
        .expect("vector cases")
}

fn object(value: &Value) -> &Map<String, Value> {
    value.as_object().expect("case object")
}

fn field<'a>(case: &'a Map<String, Value>, key: &str) -> &'a Value {
    case.get(key)
        .unwrap_or_else(|| panic!("missing case field `{key}`"))
}

fn int_value(value: &Value) -> i128 {
    if let Some(number) = value.as_i64() {
        i128::from(number)
    } else if let Some(number) = value.as_u64() {
        i128::from(number)
    } else {
        panic!("expected an integer, found {value}")
    }
}

fn type_layout(name: &str) -> (bool, u8) {
    match name {
        "SINT" => (true, 8),
        "INT" => (true, 16),
        "DINT" => (true, 32),
        "LINT" => (true, 64),
        "USINT" | "BYTE" => (false, 8),
        "UINT" | "WORD" => (false, 16),
        "UDINT" | "DWORD" => (false, 32),
        "ULINT" | "LWORD" => (false, 64),
        other => panic!("unsupported harness type `{other}`"),
    }
}

fn op_suffix(op: &str) -> &'static str {
    match op {
        "ADD" => "Add",
        "SUB" => "Sub",
        "MUL" => "Mul",
        "SHL" => "Shl",
        "SHR" => "Shr",
        "ROL" => "Rol",
        "ROR" => "Ror",
        other => panic!("unsupported harness operation `{other}`"),
    }
}

fn suffix(name: &str) -> &'static str {
    match name {
        "SINT" => "Sint",
        "INT" => "Int",
        "DINT" => "Dint",
        "LINT" => "Lint",
        "USINT" => "Usint",
        "UINT" => "Uint",
        "UDINT" => "Udint",
        "ULINT" => "Ulint",
        "BYTE" => "Byte",
        "WORD" => "Word",
        "DWORD" => "Dword",
        "LWORD" => "Lword",
        other => panic!("unsupported harness type `{other}`"),
    }
}

fn widen(name: &str) -> String {
    if type_layout(name).0 {
        format!("{name}_TO_LINT")
    } else {
        format!("{name}_TO_LWORD")
    }
}

fn input_bits(name: &str, value: i128) -> u64 {
    if type_layout(name).0 {
        (value as i64) as u64
    } else {
        value as u64
    }
}

fn output_bits(name: &str, value: i128) -> u64 {
    input_bits(name, value)
}

fn literal(kind: &str, text: &str) -> String {
    assert!(
        !text.contains(['$', '\'', '"', '\\']),
        "vector string `{text}` needs ST escaping"
    );
    match kind {
        "SBCS" => format!("STRING#'{text}'"),
        "DBCS" => format!("WSTRING#\"{text}\""),
        _ => format!("USTRING#'{text}'"),
    }
}

const PROGRAM: &str = "\nPROGRAM Main\nVAR\n  x : DINT;\nEND_VAR\n  x := x;\nEND_PROGRAM\n\
                       \nCONFIGURATION C\nRESOURCE R ON Cpu\n  PROGRAM inst : Main;\n\
                       END_RESOURCE\nEND_CONFIGURATION\n";

fn wrapper(name: &str, body: &str, result: &str, params: &str) -> String {
    format!(
        "{{stc:export}}{{stc:no_mangle}}\nFUNCTION {name} : {result}\n\
         VAR_INPUT\n{params}END_VAR\n  {name} := {body};\nEND_FUNCTION\n\n"
    )
}

fn generate_source() -> (String, String) {
    let mut source = String::new();
    let mut case_file = String::new();
    let mut wrappers = 0usize;

    // Arithmetic: ADD, SUB, MUL over every integer type.
    for (function, op) in [("add", "ADD"), ("sub", "SUB"), ("mul", "MUL")] {
        let vector = load_vector(function);
        for ty in [
            "SINT", "INT", "DINT", "LINT", "USINT", "UINT", "UDINT", "ULINT",
        ] {
            let name = format!("Vec{}{}", op_suffix(op), suffix(ty));
            let result = if type_layout(ty).0 { "LINT" } else { "LWORD" };
            let params = format!("  a : {ty};\n  b : {ty};\n");
            source.push_str(&wrapper(
                &name,
                &format!("{}({op}(a, b))", widen(ty)),
                result,
                &params,
            ));
            for case in cases(&vector) {
                let case = object(case);
                if case.get("type").and_then(Value::as_str) != Some(ty) {
                    continue;
                }
                let result_value = field(case, "result");
                if result_value.is_string() {
                    continue; // Trap cases are covered by the Rust model tests.
                }
                let a = input_bits(ty, int_value(field(case, "a")));
                let b = input_bits(ty, int_value(field(case, "b")));
                let expected = output_bits(ty, int_value(result_value));
                case_file.push_str(&format!("B {name} {a} {b} {expected}\n"));
            }
            wrappers += 1;
        }
    }

    // Shifts and rotates over the bit-string types.
    for (function, op) in [
        ("shl", "SHL"),
        ("shr", "SHR"),
        ("rol", "ROL"),
        ("ror", "ROR"),
    ] {
        let vector = load_vector(function);
        for ty in ["BYTE", "WORD", "DWORD", "LWORD"] {
            let name = format!("Vec{}{}", op_suffix(op), suffix(ty));
            let params = format!("  a : {ty};\n  n : DINT;\n");
            source.push_str(&wrapper(
                &name,
                &format!("{}({op}(a, n))", widen(ty)),
                "LWORD",
                &params,
            ));
            for case in cases(&vector) {
                let case = object(case);
                if case.get("type").and_then(Value::as_str) != Some(ty) {
                    continue;
                }
                let a = input_bits(ty, int_value(field(case, "a")));
                let n = (int_value(field(case, "n")) as i64) as u64;
                let expected = output_bits(ty, int_value(field(case, "result")));
                case_file.push_str(&format!("B {name} {a} {n} {expected}\n"));
            }
            wrappers += 1;
        }
    }

    // Selection over the integer/bit select types.
    let vector = load_vector("sel");
    for ty in ["DINT", "LINT", "UDINT", "ULINT", "DWORD", "LWORD"] {
        let name = format!("VecSel{}", suffix(ty));
        let result = if type_layout(ty).0 { "LINT" } else { "LWORD" };
        let params = format!("  g : BOOL;\n  a : {ty};\n  b : {ty};\n");
        source.push_str(&wrapper(
            &name,
            &format!("{}(SEL(g, a, b))", widen(ty)),
            result,
            &params,
        ));
        for case in cases(&vector) {
            let case = object(case);
            if case.get("type").and_then(Value::as_str) != Some(ty) {
                continue;
            }
            let g = u64::from(bool_value(field(case, "g")));
            let a = input_bits(ty, int_value(field(case, "a")));
            let b = input_bits(ty, int_value(field(case, "b")));
            let expected = output_bits(ty, int_value(field(case, "result")));
            case_file.push_str(&format!("T {name} {g} {a} {b} {expected}\n"));
        }
        wrappers += 1;
    }

    // String lengths with the literal embedded in the generated source.
    let mut string_index = 0usize;
    for (function, op) in [("len", "LEN"), ("len_code_unit", "LEN_CODE_UNIT")] {
        let vector = load_vector(function);
        for case in cases(&vector) {
            let case = object(case);
            let kind = field(case, "kind").as_str().expect("kind");
            let text = field(case, "s").as_str().expect("string");
            let name = format!("VecStr{string_index}");
            string_index += 1;
            source.push_str(&format!(
                "{{stc:export}}{{stc:no_mangle}}\nFUNCTION {name} : LINT\n  \
                 {name} := DINT_TO_LINT({op}({}));\nEND_FUNCTION\n\n",
                literal(kind, text)
            ));
            let expected = u64::try_from(int_value(field(case, "result"))).expect("length");
            case_file.push_str(&format!("S {name} {expected}\n"));
            wrappers += 1;
        }
    }

    source.push_str(PROGRAM);
    assert!(wrappers > 40, "expected a broad wrapper set");
    (source, case_file)
}

fn bool_value(value: &Value) -> bool {
    value.as_bool().expect("boolean")
}

#[test]
fn exact_vectors_run_through_generated_code() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("vectors");
    let (source, case_text) = generate_source();
    build_library(&directory, &source, "exact");
    let case_file = directory.write("cases.txt", &case_text);
    let module = built_module(&directory.path, "exact");
    let output = run_harness(
        &directory,
        "exact_harness",
        &module,
        case_file.to_str().unwrap(),
    );
    assert!(
        output.status.success(),
        "exact-vector harness failed (status {:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checked_arithmetic_traps_through_a_module_cycle() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("trap");
    let source = "PROGRAM Main\nVAR\n  x : DINT := 2147483647;\nEND_VAR\n  x := x + 1;\n\
                  END_PROGRAM\n\nCONFIGURATION C\nRESOURCE R ON Cpu\n  \
                  PROGRAM inst : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n";
    build_library(&directory, source, "exact");
    let module = built_module(&directory.path, "exact");
    let output = run_harness(&directory, "trap_harness", &module, "--trap");
    assert!(
        output.status.success(),
        "overflow did not trap through a module cycle (status {:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
