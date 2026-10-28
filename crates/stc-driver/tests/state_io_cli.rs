//! Phase-5 end-to-end tests: file-backed retained state and the v1 I/O
//! backends (`null`, `memory`, `file`).

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
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
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

fn runtime_artifact() -> PathBuf {
    common::runtime_static()
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stc-phase5-{}-{name}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create directory");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.path.join(relative)).expect("read file")
    }

    fn exists(&self, relative: &str) -> bool {
        self.path.join(relative).is_file()
    }

    fn run(&self, args: &[&str]) -> Output {
        let runtime = runtime_artifact();
        let mut all = args.to_vec();
        all.push("--runtime");
        let runtime_text = runtime.to_string_lossy().into_owned();
        all.push(&runtime_text);
        Command::new(stc())
            .args(&all)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const RETAIN_SOURCE: &str = r#"
PROGRAM Main
VAR RETAIN
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const RETAIN_LONG_SOURCE: &str = r#"
PROGRAM Main
VAR RETAIN
  counter : LINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn manifest(extra: &str) -> String {
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"app\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         {extra}"
    )
}

fn scaffold(directory: &TestDir, extra: &str) {
    directory.write("stc.toml", &manifest(extra));
    directory.write("src/main.st", RETAIN_SOURCE);
}

/// Returns the payload of a state file after validating its structure.
fn state_payload(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(&bytes[0..8], b"STCRET01", "state magic");
    assert_eq!(&bytes[8..12], &1_u32.to_le_bytes(), "format version");
    assert_eq!(&bytes[12..16], &2_u32.to_le_bytes(), "runtime ABI");
    let length = u64::from_le_bytes(bytes[32..40].try_into().expect("length"));
    assert_eq!(
        bytes.len() as u64,
        40 + length + 4,
        "header, payload, and CRC lengths agree"
    );
    bytes[40..40 + length as usize].to_vec()
}

fn dint(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .expect("DINT payload"),
    )
}

fn lint(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(
        payload[offset..offset + 8]
            .try_into()
            .expect("LINT payload"),
    )
}

#[test]
fn retained_state_round_trips_and_layout_changes_follow_the_policy() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("retain");
    scaffold(&directory, "\n[retain]\nfile = \"state/app.stcstate\"\n");
    let state = "state/app.stcstate";

    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let payload = state_payload(&directory.read(state));
    assert_eq!(payload.len(), 4, "one DINT leaf");
    assert_eq!(
        dint(&payload, 0),
        1,
        "first cold run starts at 0 and saves 1"
    );

    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(
        dint(&state_payload(&directory.read(state)), 0),
        2,
        "the warm run restores the retained counter"
    );

    // A layout change (DINT -> LINT) changes the layout hash.
    directory.write("src/main.st", RETAIN_LONG_SOURCE);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("STC-E-RUN-RETAIN-001"),
        "{}",
        text(&output)
    );
    assert_eq!(
        dint(&state_payload(&directory.read(state)), 0),
        2,
        "the rejected layout leaves the old file intact"
    );

    // on-mismatch = "reset" keeps running and overwrites the file.
    directory.write(
        "stc.toml",
        &manifest("\n[retain]\nfile = \"state/app.stcstate\"\non-mismatch = \"reset\"\n"),
    );
    let output = directory.run(&["run", "--cycles", "1", "-v"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let payload = state_payload(&directory.read(state));
    assert_eq!(payload.len(), 8, "the LINT layout has an 8-byte payload");
    assert_eq!(lint(&payload, 0), 1, "the reset run starts cold");
    assert!(
        text(&output).contains("STC-W-RUN-RETAIN-002"),
        "{}",
        text(&output)
    );

    // The default (metadata-relative) state file lives beside the module.
    let directory = TestDir::new("retain-default");
    scaffold(&directory, "");
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let payload = state_payload(&directory.read("target/stc/debug/app.stcstate"));
    assert_eq!(dint(&payload, 0), 1);
}

const RETAIN_AGGREGATE_SOURCE: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE

FUNCTION_BLOCK Counter
VAR RETAIN PUBLIC
  count : DINT;
END_VAR
  count := count + 1;
END_FUNCTION_BLOCK

VAR_GLOBAL
  plain : DINT;
END_VAR
VAR_GLOBAL NON_RETAIN
  transient : DINT;
END_VAR

PROGRAM Main
VAR RETAIN
  pt : Point;
  arr : ARRAY[1..2] OF DINT;
  text : STRING[8];
  fb : Counter;
  phase : DINT;
END_VAR
VAR
  out AT %QD0 : DINT;
END_VAR
  pt.x := pt.x + 1;
  arr[1] := arr[1] + 3;
  fb();
  phase := phase + 1;
  IF phase = 1 THEN
    text := 'first';
  ELSE
    text := 'second';
  END_IF;
  plain := plain + 1;
  transient := transient + 1;
  out := pt.x + arr[1] + fb.count;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

#[test]
fn aggregate_string_and_fb_values_round_trip() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("retain-aggregate");
    directory.write(
        "stc.toml",
        &manifest(
            "\n[io]\nbackend = \"file\"\nfile = \"agg.io\"\n\n\
             [retain]\nfile = \"agg.stcstate\"\n",
        ),
    );
    directory.write("src/main.st", RETAIN_AGGREGATE_SOURCE);

    let output = directory.run(&["run", "--cycles", "1", "-v"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(
        text(&output).contains("STC-W-RUN-RETAIN-001"),
        "the first run is cold: {}",
        text(&output)
    );
    assert_eq!(
        i32::from_le_bytes(directory.read("agg.io").try_into().expect("DINT")),
        5,
        "pt.x + arr[1] + fb.count after the cold run"
    );
    let payload = state_payload(&directory.read("agg.stcstate"));
    assert!(
        payload.windows(5).any(|window| window == b"first"),
        "the retained string is in the state payload"
    );
    let metadata =
        std::fs::read_to_string(directory.path.join("target/stc/debug/app.stc-meta.json"))
            .expect("metadata");
    for leaf in ["inst.arr", "inst.PT.x", "inst.text", "inst.fb.count"] {
        assert!(
            metadata.contains(&format!("\"path\": \"{leaf}\"")),
            "missing retained leaf {leaf}:\n{metadata}"
        );
    }
    for leaf in ["plain", "transient"] {
        assert!(
            !metadata.contains(&format!("\"path\": \"{leaf}\"")),
            "NON_RETAIN leaf {leaf} was emitted:\n{metadata}"
        );
    }

    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(
        i32::from_le_bytes(directory.read("agg.io").try_into().expect("DINT")),
        10,
        "aggregate, array, and FB state round-trips"
    );
    let payload = state_payload(&directory.read("agg.stcstate"));
    assert!(
        payload.windows(6).any(|window| window == b"second"),
        "the second run's string was saved"
    );
}

#[test]
fn corrupt_state_reports_the_documented_codes() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("retain-corrupt");
    scaffold(&directory, "\n[retain]\nfile = \"app.stcstate\"\n");
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));

    // A bad CRC is corruption.
    let mut bytes = directory.read("app.stcstate");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    directory.write_bytes("app.stcstate", &bytes);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("STC-E-RUN-RETAIN-002"),
        "{}",
        text(&output)
    );

    // A bad magic is corruption too.
    let mut bytes = directory.read("app.stcstate");
    bytes[0] = b'X';
    directory.write_bytes("app.stcstate", &bytes);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("STC-E-RUN-RETAIN-002"),
        "{}",
        text(&output)
    );

    // An unknown format version is a mismatch.
    let mut bytes = directory.read("app.stcstate");
    bytes[0] = b'S';
    bytes[8..12].copy_from_slice(&9_u32.to_le_bytes());
    directory.write_bytes("app.stcstate", &bytes);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("STC-E-RUN-RETAIN-001"),
        "{}",
        text(&output)
    );

    // reset policy recovers and overwrites the file.
    directory.write(
        "stc.toml",
        &manifest("\n[retain]\nfile = \"app.stcstate\"\non-mismatch = \"reset\"\n"),
    );
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(dint(&state_payload(&directory.read("app.stcstate")), 0), 1);
}

#[test]
fn state_file_and_retain_overrides() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("retain-overrides");
    scaffold(&directory, "\n[retain]\nfile = \"manifest.stcstate\"\n");

    // --state-file resolves against the current directory.
    let output = directory.run(&["run", "--cycles", "1", "--state-file", "cli.stcstate"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(dint(&state_payload(&directory.read("cli.stcstate")), 0), 1);
    assert!(!directory.exists("manifest.stcstate"));

    // --reset-retain ignores the existing value and overwrites on shutdown.
    let output = directory.run(&[
        "run",
        "--cycles",
        "1",
        "--state-file",
        "cli.stcstate",
        "--reset-retain",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(dint(&state_payload(&directory.read("cli.stcstate")), 0), 1);

    // --no-retain neither loads nor saves.
    let output = directory.run(&["run", "--cycles", "1", "--state-file", "cli.stcstate"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(dint(&state_payload(&directory.read("cli.stcstate")), 0), 2);
    let output = directory.run(&[
        "run",
        "--cycles",
        "1",
        "--state-file",
        "cli.stcstate",
        "--no-retain",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(
        dint(&state_payload(&directory.read("cli.stcstate")), 0),
        2,
        "--no-retain leaves the file untouched"
    );
    assert!(!directory.exists("target/stc/debug/app.stcstate"));

    // [retain] enabled = false disables load and save.
    directory.write(
        "stc.toml",
        &manifest("\n[retain]\nfile = \"manifest.stcstate\"\nenabled = false\n"),
    );
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(!directory.exists("manifest.stcstate"));
}

#[test]
fn failed_state_replace_reports_the_write_error() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("retain-write-failure");
    scaffold(&directory, "");
    // A directory at the target path makes the atomic replace fail; the run
    // still reports STC-E-RUN-RETAIN-003 and exits 1.
    std::fs::create_dir(directory.path.join("blocked.stcstate")).expect("block the state path");
    let output = directory.run(&["run", "--cycles", "1", "--state-file", "blocked.stcstate"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(
        text(&output).contains("STC-E-RUN-RETAIN-003"),
        "{}",
        text(&output)
    );
    assert!(
        directory.path.join("blocked.stcstate").is_dir(),
        "the previous file (directory) is intact"
    );
}

const IO_SOURCE: &str = r#"
PROGRAM Main
VAR
  start AT %IX0.0 : BOOL;
  done AT %QX0.0 : BOOL;
END_VAR
  done := start;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn io_manifest(extra: &str) -> String {
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"ioapp\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         [io]\n\
         {extra}"
    )
}

#[test]
fn file_io_backend_reads_once_and_writes_atomically() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("io-file");
    directory.write(
        "stc.toml",
        &io_manifest("backend = \"file\"\nfile = \"io.bin\"\n"),
    );
    directory.write("src/main.st", IO_SOURCE);

    directory.write_bytes("io.bin", &[0]);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(directory.read("io.bin"), vec![0]);

    directory.write_bytes("io.bin", &[1]);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(directory.read("io.bin"), vec![1]);

    // A missing input file is a cold start with defaults and a warning.
    std::fs::remove_file(directory.path.join("io.bin")).expect("remove input");
    let output = directory.run(&["run", "--cycles", "1", "-v"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(text(&output).contains("STC-W-IO-004"), "{}", text(&output));
    assert_eq!(directory.read("io.bin"), vec![0]);

    // --io-file overrides both paths.
    directory.write_bytes("other.bin", &[1]);
    let output = directory.run(&["run", "--cycles", "1", "--io-file", "other.bin"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(directory.read("other.bin"), vec![1]);
}

#[test]
fn null_backend_does_not_write_an_image_file() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("io-null");
    directory.write("stc.toml", &manifest(""));
    directory.write("src/main.st", IO_SOURCE);
    let output = directory.run(&["run", "--cycles", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(!directory.exists("io.bin"));
    assert!(!directory.exists("target/stc/debug/ioapp.stcio"));

    // An explicit memory backend is accepted but writes no file either.
    let output = directory.run(&["run", "--cycles", "1", "--io", "memory"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(!directory.exists("target/stc/debug/app.stcio"));
}
