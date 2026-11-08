//! `stc run` integration tests (M6 phase 4): project run defaults, CLI
//! overrides, host status output, and exit codes.

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
        let path = std::env::temp_dir().join(format!(
            "stc-run-phase4-{}-{name}-{unique}",
            std::process::id()
        ));
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

    fn run(&self, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run stc")
    }

    fn run_with_runtime(&self, args: &[&str]) -> Output {
        let runtime = runtime_artifact();
        let mut all = args.to_vec();
        all.push("--runtime");
        let runtime_text = runtime.to_string_lossy().into_owned();
        all.push(&runtime_text);
        self.run(&all)
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

const SOURCE: &str = r#"
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
PROGRAM Slow
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
PROGRAM Startup
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
CONFIGURATION C
RESOURCE R ON Cpu
  TASK Fast (PRIORITY := 1);
  TASK Timed (INTERVAL := T#5s);
  TASK Once (SINGLE := TRUE);
  PROGRAM fast WITH Fast : Main;
  PROGRAM slow WITH Timed : Slow;
  PROGRAM once WITH Once : Startup;
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
    directory.write("src/main.st", SOURCE);
}

#[test]
fn run_uses_manifest_defaults_and_prints_host_status() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("defaults");
    scaffold(&directory, "\n[run]\ncycles = 2\ncycle-time = \"1ms\"\n");

    let output = directory.run_with_runtime(&["run", "-v"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let text = text(&output);
    assert!(text.contains("stc: status: cycles 2"), "{text}");
    assert!(text.contains("stc: status: task "), "{text}");
    assert!(text.contains("due="), "{text}");
    assert!(
        text.contains("#ONCE due=0 completed=1"),
        "the SINGLE task completed exactly once:\n{text}"
    );
    assert!(
        text.contains("#TIMED due=0"),
        "the interval task did not run before its deadline:\n{text}"
    );
    assert!(
        directory.path.join("target/stc/debug/app.exe").is_file()
            || directory.path.join("target/stc/debug/app").is_file(),
        "the launcher was emitted"
    );
    assert!(
        directory
            .path
            .join("target/stc/debug/app.stc-meta.json")
            .is_file()
    );
}

#[test]
fn run_cycles_override_the_manifest_default() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("override");
    scaffold(&directory, "\n[run]\ncycles = 2\n");

    let output = directory.run_with_runtime(&["run", "--cycles", "1", "--cycle-time", "1ms", "-v"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(
        text(&output).contains("stc: status: cycles 1"),
        "{}",
        text(&output)
    );
}

#[test]
fn run_returns_exit_code_4_for_a_trap() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("trap");
    directory.write("stc.toml", &manifest("\n[run]\ncycles = 1\n"));
    directory.write(
        "src/main.st",
        "PROGRAM Main\nVAR x : DINT; END_VAR\n  x := 1 / 0;\nEND_PROGRAM\n",
    );
    let output = directory.run_with_runtime(&["run"]);
    assert_eq!(output.status.code(), Some(4), "{}", text(&output));
    assert!(text(&output).contains("DIV_ZERO"), "{}", text(&output));
}

#[test]
fn run_option_validation_uses_exit_code_2() {
    let directory = TestDir::new("usage");
    scaffold(&directory, "");
    for args in [
        vec!["run", "--cycles", "nope"],
        vec!["run", "--cycle-time", "soon"],
        vec!["run", "--io", "disk"],
        vec!["run", "--emit", "module"],
        vec!["build", "--cycles", "2"],
    ] {
        let output = directory.run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", text(&output));
    }
}

fn assert_host_rt_help(text: &str) {
    #[cfg(target_os = "linux")]
    {
        assert!(
            text.contains("`--rt` sets the cycle thread to `SCHED_FIFO` (default priority 90)"),
            "{text}"
        );
        assert!(
            text.contains("uses absolute monotonic sleeps; `--rt-cpu` pins it to one CPU"),
            "{text}"
        );
    }
    #[cfg(windows)]
    {
        assert!(
            text.contains("`--rt` raises the cycle thread to `THREAD_PRIORITY_TIME_CRITICAL`"),
            "{text}"
        );
        assert!(
            text.contains("registers it with MMCSS when available"),
            "{text}"
        );
        assert!(
            text.contains("`--rt-cpu`/`--rt-priority` are Linux-only"),
            "{text}"
        );
    }
    #[cfg(target_os = "macos")]
    {
        assert!(
            text.contains("`--rt` applies a Mach `THREAD_TIME_CONSTRAINT_POLICY`"),
            "{text}"
        );
        assert!(
            text.contains("falling back to an interactive QoS class"),
            "{text}"
        );
        assert!(
            text.contains("`--rt-cpu`/`--rt-priority` are Linux-only"),
            "{text}"
        );
    }
}

#[test]
fn launcher_help_names_the_host_rt_facility() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("launcher-help");
    scaffold(&directory, "\n[run]\ncycles = 1\n");
    let build = directory.run_with_runtime(&["build", "--emit", "exe,module,metadata"]);
    assert_eq!(build.status.code(), Some(0), "{}", text(&build));
    let launcher = directory.path.join(if cfg!(windows) {
        "target/stc/debug/app.exe"
    } else {
        "target/stc/debug/app"
    });
    let output = Command::new(&launcher)
        .arg("--help")
        .output()
        .expect("run the launcher");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("stc runtime launcher"), "{text}");
    assert_host_rt_help(&text);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn rt_run_emits_metrics_on_windows_and_macos() {
    if clang().is_none() {
        return;
    }
    let directory = TestDir::new("rt-metrics");
    scaffold(&directory, "\n[run]\ncycles = 3\ncycle-time = \"1ms\"\n");
    let metrics = directory.path.join("rt.jsonl");
    let output = directory.run_with_runtime(&[
        "run",
        "--rt",
        "--cycle-time",
        "1ms",
        "--cycles",
        "3",
        "--rt-metrics",
        metrics.to_str().expect("metrics path"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    let contents = std::fs::read_to_string(&metrics).expect("read metrics");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 5, "header, three cycles, summary: {contents}");
    let header = lines[0];
    assert!(header.contains("\"record\":\"header\""), "{header}");
    assert!(
        header.contains("\"protocol\":\"stc-rt-metrics/1\""),
        "{header}"
    );
    assert!(
        header.contains("\"sched\":\"mmcss\"")
            || header.contains("\"sched\":\"time_critical\"")
            || header.contains("\"sched\":\"time_constraint\"")
            || header.contains("\"sched\":\"qos\""),
        "the header names the applied facility: {header}"
    );
    let cycles = lines
        .iter()
        .filter(|line| line.contains("\"record\":\"cycle\""))
        .count();
    assert_eq!(cycles, 3, "{contents}");
    let summary = lines.last().expect("summary");
    assert!(summary.contains("\"record\":\"summary\""), "{summary}");
    assert!(summary.contains("\"cycles\":3"), "{summary}");
}
