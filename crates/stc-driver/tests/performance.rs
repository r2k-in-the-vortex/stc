//! M7 performance pass: build and workflow timings (D-193).
//!
//! The measurement is ignored by default; wall-clock numbers are
//! informational and never gate the suite. It records `check`, IR, module,
//! release-module, `run`, and `test` costs for the committed hello example,
//! plus the per-cycle wall time of a short run. Runtime microbenchmarks
//! (switch latency, debug overhead, typed read) remain in their dedicated
//! ignored tests.
//!
//! ```text
//! cargo test -p stc-driver --test performance -- --ignored --nocapture
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn clang() -> Option<&'static str> {
    let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
    match Command::new(&candidate).arg("--version").output() {
        Ok(output) if output.status.success() => Some(Box::leak(candidate.into_boxed_str())),
        _ => {
            if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
            }
            eprintln!("note: clang not found; skipping performance measurements");
            None
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-performance-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.path.clone(), |path, part| path.join(part))
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        if entry.file_name().to_str() == Some("target") {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type().expect("entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn run(directory: &Path, args: &[&str]) -> Output {
    let output = Command::new(stc())
        .args(args)
        .arg("--color=never")
        .current_dir(directory)
        .output()
        .expect("run stc");
    assert!(
        output.status.success(),
        "`stc {}` failed:\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn measure(runs: usize, mut operation: impl FnMut()) -> Duration {
    operation();
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        operation();
        best = best.min(start.elapsed());
    }
    best
}

fn report(name: &str, best: Duration, runs: usize) {
    println!(
        "{name:<28} {:>10.3} ms  (best of {runs})",
        best.as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "informational wall-clock measurement; run explicitly"]
fn workflow_timings() {
    if clang().is_none() {
        return;
    }
    let source = workspace_root().join("examples/hello");
    let directory = TestDir::new("workflow");
    copy_tree(&source, &directory.path("hello"));
    let manifest = directory.path("hello/stc.toml");
    let manifest = manifest.to_string_lossy().into_owned();
    let runtime = common::runtime_static();
    let runtime = runtime.to_string_lossy().into_owned();
    let out = |name: &str| directory.path(name).to_string_lossy().into_owned();

    println!("stc performance (host measurement, informational)");
    println!("  version      {}", env!("CARGO_PKG_VERSION"));
    println!("  host         {}", stc_host());
    println!();

    let runs = 10;
    let check = measure(runs, || {
        run(
            &directory.path,
            &["check", "--locked", "--manifest-path", &manifest],
        );
    });
    report("stc check", check, runs);

    let ir = measure(5, || {
        run(
            &directory.path,
            &[
                "build",
                "--locked",
                "--emit=ir",
                "--out-dir",
                &out("ir"),
                "--manifest-path",
                &manifest,
            ],
        );
    });
    report("stc build --emit=ir", ir, 5);

    let module = measure(5, || {
        run(
            &directory.path,
            &[
                "build",
                "--locked",
                "--emit=module",
                "--out-dir",
                &out("debug"),
                "--manifest-path",
                &manifest,
            ],
        );
    });
    report("stc build (debug module)", module, 5);

    let release = measure(5, || {
        run(
            &directory.path,
            &[
                "build",
                "--locked",
                "--release",
                "--emit=module",
                "--out-dir",
                &out("release"),
                "--manifest-path",
                &manifest,
            ],
        );
    });
    report("stc build --release", release, 5);

    let short = {
        let cycles = 100_u64.to_string();
        measure(5, || {
            run(
                &directory.path,
                &[
                    "run",
                    "--locked",
                    "--cycles",
                    &cycles,
                    "--runtime",
                    &runtime,
                    "--out-dir",
                    &out("run"),
                    "--manifest-path",
                    &manifest,
                ],
            );
        })
    };
    let long = {
        let cycles = 1_100_u64.to_string();
        measure(5, || {
            run(
                &directory.path,
                &[
                    "run",
                    "--locked",
                    "--cycles",
                    &cycles,
                    "--runtime",
                    &runtime,
                    "--out-dir",
                    &out("run"),
                    "--manifest-path",
                    &manifest,
                ],
            );
        })
    };
    report("stc run (100 cycles)", short, 5);
    report("stc run (1100 cycles)", long, 5);
    let marginal = (long.saturating_sub(short)).as_secs_f64() / 1_000.0;
    println!(
        "{:<28} {:>10.3} us/cycle (marginal, build/startup removed)",
        "stc run cycle cost",
        marginal * 1_000_000.0
    );

    let test = measure(3, || {
        run(
            &directory.path,
            &[
                "test",
                "--locked",
                "--runtime",
                &runtime,
                "--manifest-path",
                &manifest,
            ],
        );
    });
    report("stc test", test, 3);
}

fn stc_host() -> String {
    let output = Command::new("rustc").args(["-vV"]).output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        })
        .unwrap_or_else(|| "unknown host".into())
}
