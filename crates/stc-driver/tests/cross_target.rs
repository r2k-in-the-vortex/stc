//! Cross-target IR/object smoke matrix (M7 phase 2, D-195).
//!
//! `stc` emits target-independent textual IR unless `--target` is given. This
//! suite compiles the emitted IR with `clang --target=<triple>` for the
//! documented matrix and inspects the object header, so an architecture,
//! endianness, or pointer-width regression is visible without a sysroot.
//! Executable and embedding-library emission remain host-only and must fail
//! with a diagnostic rather than produce a partial artifact.

use std::path::{Path, PathBuf};
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
                    eprintln!("note: clang not found; skipping cross-target tests");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

/// One supported smoke triple and the object header it must produce.
#[derive(Clone, Copy)]
enum Header {
    Elf { class: u8, data: u8, machine: u16 },
    MachO { cpu: u32 },
    Coff { machine: u16 },
}

const ELL: u8 = 1; // ELF little-endian
const ELB: u8 = 2; // ELF big-endian

const MATRIX: [(&str, Header); 9] = [
    (
        "x86_64-unknown-linux-gnu",
        Header::Elf {
            class: 2,
            data: ELL,
            machine: 0x3e,
        },
    ),
    (
        "aarch64-unknown-linux-gnu",
        Header::Elf {
            class: 2,
            data: ELL,
            machine: 0xb7,
        },
    ),
    (
        "aarch64_be-unknown-linux-gnu",
        Header::Elf {
            class: 2,
            data: ELB,
            machine: 0xb7,
        },
    ),
    ("x86_64-apple-darwin", Header::MachO { cpu: 0x0100_0007 }),
    ("aarch64-apple-darwin", Header::MachO { cpu: 0x0100_000c }),
    ("x86_64-pc-windows-msvc", Header::Coff { machine: 0x8664 }),
    ("aarch64-pc-windows-msvc", Header::Coff { machine: 0xaa64 }),
    (
        "riscv64-unknown-elf",
        Header::Elf {
            class: 2,
            data: ELL,
            machine: 0xf3,
        },
    ),
    (
        "thumbv7em-none-eabihf",
        Header::Elf {
            class: 1,
            data: ELL,
            machine: 0x28,
        },
    ),
];

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stc-cross-{}-{name}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write test file");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A source that exercises scalars, an array, a UTF-8 string, an enum, an FB,
/// and a configuration. The IR is target-independent, so exercising these
/// constructs here is a layout/emission smoke test, not an execution claim.
const SOURCE: &str = r#"
TYPE Color : (Red, Green, Blue); END_TYPE

FUNCTION AddTwo : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  AddTwo := a + b;
END_FUNCTION

FUNCTION_BLOCK Counter
VAR_INPUT
  increment : DINT := 1;
END_VAR
VAR_OUTPUT
  value : DINT;
END_VAR
  value := value + increment;
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  counter : Counter;
  sum : DINT;
  i : DINT;
  values : ARRAY [0..3] OF DINT;
  text : USTRING[16];
  color : Color;
END_VAR
  counter(increment := 1, value => sum);
  FOR i := 0 TO 3 DO
    values[i] := AddTwo(i, 1);
  END_FOR;
  text := 'hello';
  IF sum > 0 THEN
    color := Green;
  END_IF;
END_PROGRAM

CONFIGURATION C
RESOURCE R ON Cpu
  PROGRAM inst : Main;
END_RESOURCE
END_CONFIGURATION
"#;

fn run(args: &[&str], directory: &TestDir) -> Output {
    Command::new(stc())
        .args(args)
        .arg("--color=never")
        .current_dir(&directory.path)
        .output()
        .expect("run stc")
}

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_header(object: &Path) -> Vec<u8> {
    let bytes = std::fs::read(object).expect("read object");
    assert!(bytes.len() >= 20, "object is too short: {}", bytes.len());
    bytes[..20].to_vec()
}

fn check_header(object: &Path, expected: Header) {
    let header = read_header(object);
    match expected {
        Header::Elf {
            class,
            data,
            machine,
        } => {
            assert_eq!(&header[..4], b"\x7fELF", "{}: not ELF", object.display());
            assert_eq!(header[4], class, "{}: ELF class", object.display());
            assert_eq!(header[5], data, "{}: ELF data encoding", object.display());
            let encoded = if data == ELL {
                u16::from_le_bytes([header[18], header[19]])
            } else {
                u16::from_be_bytes([header[18], header[19]])
            };
            assert_eq!(encoded, machine, "{}: ELF machine", object.display());
        }
        Header::MachO { cpu } => {
            assert_eq!(
                &header[..4],
                b"\xcf\xfa\xed\xfe",
                "{}: not 64-bit little-endian Mach-O",
                object.display()
            );
            assert_eq!(
                u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
                cpu,
                "{}: Mach-O cpu type",
                object.display()
            );
        }
        Header::Coff { machine } => {
            assert_eq!(
                u16::from_le_bytes([header[0], header[1]]),
                machine,
                "{}: COFF machine",
                object.display()
            );
        }
    }
}

#[test]
fn release_ir_is_target_independent_and_compiles_for_the_matrix() {
    let Some(clang) = clang() else {
        return;
    };
    let directory = TestDir::new("ir");
    directory.write("main.st", SOURCE);
    let output = run(
        &[
            "build",
            "main.st",
            "--release",
            "--emit=ir",
            "--name",
            "cross",
            "--out-dir",
            "ir",
        ],
        &directory,
    );
    assert_success(&output, "release IR build");
    let ir = std::fs::read_to_string(directory.path("ir/cross.ll")).expect("read IR");
    assert!(
        !ir.contains("target datalayout"),
        "IR must not embed a target datalayout"
    );
    assert!(
        !ir.contains("target triple"),
        "IR without --target must not embed a target triple"
    );
    for (triple, header) in MATRIX {
        let object = directory.path(&format!("objects/{triple}.o"));
        std::fs::create_dir_all(object.parent().unwrap()).expect("create object directory");
        let output = Command::new(clang)
            .arg(format!("--target={triple}"))
            .args(["-x", "ir", "-c"])
            .arg(directory.path("ir/cross.ll"))
            .arg("-o")
            .arg(&object)
            .output()
            .expect("run clang");
        assert!(
            output.status.success(),
            "clang --target={triple} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(object.metadata().expect("object metadata").len() > 0);
        check_header(&object, header);
    }
}

#[test]
fn driver_emits_objects_for_the_matrix() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("driver");
    directory.write("main.st", SOURCE);
    for (triple, header) in MATRIX {
        let out = format!("out/{triple}");
        let output = run(
            &[
                "build",
                "main.st",
                "--emit=obj",
                "--target",
                triple,
                "--name",
                "cross",
                "--out-dir",
                &out,
            ],
            &directory,
        );
        assert_success(&output, &format!("driver object build for {triple}"));
        let ir = std::fs::read_to_string(directory.path(&format!("{out}/cross.ll")))
            .expect("read target IR");
        assert!(
            ir.contains(&format!("target triple = \"{triple}\"")),
            "{triple}: IR does not carry the requested target triple"
        );
        let object = directory.path(&format!("{out}/cross.o"));
        check_header(&object, header);
    }
}

/// Cross-target `exe`/`lib` emission is rejected with `STC-E-BUILD-004`.
#[test]
fn cross_target_executable_and_library_are_rejected() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("reject");
    directory.write("main.st", SOURCE);
    // The requested target must be foreign to the host: an AArch64 Linux
    // target is the native target on an AArch64 Linux host, where linking
    // legitimately succeeds. Keep the same expectations either way.
    let foreign = if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "aarch64-unknown-linux-gnu"
    };
    for (emit, message) in [
        ("exe", "cross-target executables are not supported"),
        ("lib", "cross-target embedding libraries are not supported"),
    ] {
        let out = format!("out/{emit}");
        let output = run(
            &[
                "build",
                "main.st",
                &format!("--emit={emit}"),
                "--target",
                foreign,
                "--name",
                "cross",
                "--out-dir",
                &out,
            ],
            &directory,
        );
        assert!(
            !output.status.success(),
            "{emit}: cross-target build unexpectedly succeeded"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(message),
            "{emit}: unexpected diagnostic:\n{stderr}"
        );
        let names: &[&str] = if emit == "exe" {
            if cfg!(windows) {
                &["cross.exe"]
            } else {
                &["cross"]
            }
        } else if cfg!(windows) {
            &["cross.lib", "cross.dll"]
        } else if cfg!(target_os = "macos") {
            &["libcross.a", "libcross.dylib"]
        } else {
            &["libcross.a", "libcross.so"]
        };
        for name in names {
            assert!(
                !directory.path(&format!("{out}/{name}")).exists(),
                "{emit}: partial artifact `{name}` was left behind"
            );
        }
    }
}
