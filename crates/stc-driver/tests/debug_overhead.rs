//! M6.5 phase-9 debug overhead measurement.
//!
//! Records the per-cycle cost of the same generated program built through the
//! real driver pipeline in its three documented forms (Debug and Monitoring
//! §17; CLI and Manifest §1.7):
//!
//! - `release`: optimised (`-O2`), stripped, and hook-free;
//! - `debug hook-free`: debug information (`-O0 -g`) with no checkpoint table
//!   and no `debug_check` import — the documented test form (§17.7);
//! - `debug hooks`: debug information with the checkpoint table, service 13
//!   calls, and the `frame-pointer="all"`/`disable-tail-calls="true"`
//!   attributes the front end forces on hinted functions.
//!
//! It also measures a typed in-process read and, when the platform supports
//! it, the same hook-bearing build under an active observe session so the
//! session check can be separated from the no-session no-op.
//!
//! The measurement is ignored by default because wall-clock numbers are noisy
//! on shared runners and must never gate normal tests. Run it explicitly:
//!
//! ```text
//! cargo test -p stc-driver --test debug_overhead -- --ignored --nocapture
//! ```
//!
//! Every number printed here is measured, not expected. The test asserts only
//! non-timing preconditions (the builds have the documented hook/identity
//! shape, they load, and they cycle).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use stc_got::ImportService;
use stc_runtime::{
    Candidate, DebugClient, DebugServerConfig, Host, LoadMode, LoadOutcome, ManifestStore,
    TransportKind,
};

mod common;

/// Independent timed runs per variant; the best run is reported and the
/// median is printed for noise context. Shared runners can migrate the thread
/// to a different core mid-run, so best-of is the primary figure.
const REPEATS: usize = 15;
/// Untimed cycles before each measured variant.
const WARMUP_CYCLES: u64 = 2_000;
/// Cycles per timed run.
const CYCLES: u64 = 50_000;
/// Timed typed reads per run.
const READS: u64 = 20_000;

/// The representative production program. Every checkpoint site in `Bump` and
/// the `Main` body executes exactly once per cycle: there are no loops, the
/// `IF` guard is always true for the reachable range, and the arithmetic is
/// bounded so the program cannot trap during a long measurement run.
const SOURCE: &str = "\
VAR_GLOBAL
  Acc : DINT;
END_VAR

FUNCTION Bump : DINT
VAR_INPUT
  x : DINT;
  bias : DINT;
END_VAR
VAR
  y : DINT;
END_VAR
  y := x + bias;
  IF y >= 0 THEN
    y := y MOD 1000;
  END_IF;
  Bump := y;
END_FUNCTION

PROGRAM Main
VAR
  ticks : DINT;
  total : DINT;
END_VAR
  ticks := (ticks + 1) MOD 1000;
  total := Bump(total, ticks);
  Acc := (Acc + total) MOD 1000000;
END_PROGRAM
";

/// The hook-free debug form is the documented test form (§17.7), which is
/// compiled together with `[test].sources`. A test PROGRAM is selected through
/// the synthetic `STC_TEST` configuration; the body is identical to
/// [`SOURCE`] apart from the POU name.
const TEST_SOURCE: &str = "\
VAR_GLOBAL
  Acc : DINT;
END_VAR

FUNCTION Bump : DINT
VAR_INPUT
  x : DINT;
  bias : DINT;
END_VAR
VAR
  y : DINT;
END_VAR
  y := x + bias;
  IF y >= 0 THEN
    y := y MOD 1000;
  END_IF;
  Bump := y;
END_FUNCTION

{stc:test}
PROGRAM Bench
VAR
  ticks : DINT;
  total : DINT;
END_VAR
  ticks := (ticks + 1) MOD 1000;
  total := Bump(total, ticks);
  Acc := (Acc + total) MOD 1000000;
END_PROGRAM
";

/// A production POU for the hook-free test project. It is never selected or
/// run; it exists so the package has a production source set.
const TEST_PROJECT_STUB: &str = "\
PROGRAM Idle
VAR
  idle : DINT;
END_VAR
  idle := (idle + 1) MOD 1000;
END_PROGRAM
";

fn stc() -> &'static str {
    env!("CARGO_BIN_EXE_stc")
}

fn clang() -> Option<String> {
    let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
    match Command::new(&candidate).arg("--version").output() {
        Ok(output) if output.status.success() => Some(candidate),
        _ => {
            if std::env::var_os("STC_REQUIRE_CLANG").is_some() {
                panic!("clang is required (STC_REQUIRE_CLANG=1) but was not found");
            }
            eprintln!("note: clang not found; skipping the debug overhead measurement");
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
            "stc-debug-overhead-{}-{name}-{unique}",
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

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
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
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn manifest(name: &str, test_sources: bool) -> String {
    let test = if test_sources {
        "[test]\n\
         sources = [\"tests/**/*.st\"]\n\
         \n\
         [build]\n"
    } else {
        "[build]\n"
    };
    format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         language = \"stc-2013-oop-utf8-v1\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         {test}"
    )
}

fn production_project(name: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("stc.toml", &manifest("app", false));
    project.write("src/main.st", SOURCE);
    project
}

fn hookfree_project(name: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("stc.toml", &manifest("bench", true));
    project.write("src/other.st", TEST_PROJECT_STUB);
    project.write("tests/bench.st", TEST_SOURCE);
    project
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read `{}`: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse `{}`: {error}\n{text}", path.display()))
}

/// One built artifact set: module, manifest, metadata sidecar, IR, and the
/// optional stopped-frame sidecar.
struct Built {
    module: PathBuf,
    manifest: PathBuf,
    ir: PathBuf,
    frames: PathBuf,
    metadata_json: Value,
}

impl Built {
    /// Reads the metadata-recorded module file from an output directory.
    fn read(directory: &Path, stem: &str) -> Self {
        let metadata = directory.join(format!("{stem}.stc-meta.json"));
        let metadata_json = json(&metadata);
        let module = directory.join(
            metadata_json["module"]["file"]
                .as_str()
                .unwrap_or_else(|| panic!("`{}` records no module file", metadata.display())),
        );
        Self {
            module,
            manifest: directory.join(format!("{stem}.stc-got.bin")),
            ir: directory.join(format!("{stem}.ll")),
            frames: directory.join(format!("{stem}.stc-frames.json")),
            metadata_json,
        }
    }

    fn checkpoints(&self) -> usize {
        self.metadata_json["debug"]["checkpoints"]
            .as_array()
            .map_or(0, Vec::len)
    }

    fn build_id(&self) -> &str {
        self.metadata_json["build_id"]
            .as_str()
            .expect("metadata build id")
    }

    fn manifest_digest(&self) -> &str {
        self.metadata_json["manifest"]["digest"]
            .as_str()
            .expect("metadata manifest digest")
    }

    fn ir_text(&self) -> String {
        std::fs::read_to_string(&self.ir).unwrap_or_default()
    }
}

fn build_release(project: &TestDir) -> Built {
    let output = project.run(&["build", "--release", "--emit", "ir,module,metadata"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    Built::read(&project.path("target/stc/release"), "app")
}

fn build_debug(project: &TestDir) -> Built {
    let output = project.run(&["build", "--emit", "ir,module,metadata"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    Built::read(&project.path("target/stc/debug"), "app")
}

fn build_hookfree(project: &TestDir) -> Built {
    // `stc test --no-run` links the test host, so it needs the static runtime
    // archive. Build it into the shared test-runtime directory like the other
    // driver suites instead of relying on an artifact beside the driver, which
    // a clean CI target directory does not contain.
    let runtime = common::runtime_static();
    let runtime = runtime.to_string_lossy().into_owned();
    let output = project.run(&["test", "--no-run", "--runtime", &runtime]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    Built::read(&project.path("target/stc/test/bench-Bench"), "bench-Bench")
}

fn has_service_13(built: &Built) -> bool {
    let candidate = Candidate::open(&host_platform(), &built.module, &built.manifest)
        .unwrap_or_else(|error| panic!("candidate `{}`: {error}", built.module.display()));
    candidate.descriptor().slots.iter().any(|slot| {
        slot.service == ImportService::DebugCheck.id() && slot.kind == stc_got::SlotKind::Import
    })
}

fn host_platform() -> stc_runtime::HostPlatform {
    stc_runtime::host_platform()
}

fn install(host: &mut Host, built: &Built) -> u64 {
    let candidate = Candidate::open(&host_platform(), &built.module, &built.manifest)
        .unwrap_or_else(|error| panic!("candidate `{}`: {error}", built.module.display()));
    match host.load(candidate, LoadMode::Auto) {
        Ok(LoadOutcome::ColdInstalled { generation }) => generation,
        other => panic!(
            "expected a cold install for `{}`: {other:?}",
            built.module.display()
        ),
    }
}

/// Runs `CYCLES` timed cycles in `REPEATS` independent runs and returns the
/// best and median nanoseconds per cycle.
fn cycle_times(built: &Built) -> (f64, f64) {
    let mut host = Host::new(host_platform(), ManifestStore::memory());
    let _generation = install(&mut host, built);
    let mut now = 1_000_000_i64;
    for _ in 0..WARMUP_CYCLES {
        host.cycle_at(now).expect("warm-up cycle");
        now = now.saturating_add(1_000_000);
    }
    let mut runs = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        let start = Instant::now();
        for _ in 0..CYCLES {
            host.cycle_at(now).expect("measured cycle");
            now = now.saturating_add(1_000_000);
        }
        runs.push(start.elapsed().as_secs_f64() * 1e9 / CYCLES as f64);
    }
    host.shutdown().expect("shutdown");
    runs.sort_by(f64::total_cmp);
    (runs[0], runs[runs.len() / 2])
}

/// Starts an observe-only session over the loopback socket transport, runs the
/// same cycle measurement, and tears the session down.
fn cycle_times_with_session(project: &TestDir, built: &Built) -> (f64, f64) {
    let endpoint = project.path("overhead.endpoint.json");
    let mut host = Host::new(host_platform(), ManifestStore::memory());
    let _generation = install(&mut host, built);
    let mut config = DebugServerConfig::new(endpoint, TransportKind::Socket);
    config.session_timeout = Duration::from_secs(120);
    host.start_debug_server(&config).expect("debug server");
    let info = host.debug_server_info().expect("server info");
    let mut client = DebugClient::connect_file(&info.endpoint_file).expect("connect");
    pumped(&mut host, || client.start_session(&["observe"])).expect("session start");

    let mut now = 1_000_000_i64;
    for _ in 0..WARMUP_CYCLES {
        host.cycle_at(now).expect("warm-up cycle");
        now = now.saturating_add(1_000_000);
    }
    let mut runs = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        let start = Instant::now();
        for _ in 0..CYCLES {
            host.cycle_at(now).expect("measured cycle");
            now = now.saturating_add(1_000_000);
        }
        runs.push(start.elapsed().as_secs_f64() * 1e9 / CYCLES as f64);
    }
    let _ = pumped(&mut host, || {
        client.call_ok("session.stop", stc_got::json::Json::object(Vec::new()))
    });
    host.stop_debug_server();
    host.shutdown().expect("shutdown");
    runs.sort_by(f64::total_cmp);
    (runs[0], runs[runs.len() / 2])
}

fn pumped<T: Send>(host: &mut Host, call: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let handle = scope.spawn(call);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "debug call did not finish");
            host.debug_pump_at(0);
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.join().expect("debug call thread")
    })
}

/// Times typed reads of the `Acc` global through the host API, with no
/// protocol transport in the path.
fn typed_read_times(built: &Built) -> (f64, f64) {
    let mut host = Host::new(host_platform(), ManifestStore::memory());
    let _generation = install(&mut host, built);
    let request = || stc_runtime::DebugReadRequest::new(stc_runtime::DebugTarget::path("Acc"));
    // Warm up and prove the target resolves; a failure is a real regression,
    // not timing noise.
    let first = host.debug_read(request()).expect("typed read of `Acc`");
    assert!(
        !first.type_key.is_empty(),
        "the representative program exposes a typed `Acc`: {first:?}"
    );
    for _ in 0..1_000 {
        host.debug_read(request()).expect("warm-up read");
    }
    let mut runs = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        let start = Instant::now();
        for _ in 0..READS {
            host.debug_read(request()).expect("measured read");
        }
        runs.push(start.elapsed().as_secs_f64() * 1e9 / READS as f64);
    }
    host.shutdown().expect("shutdown");
    runs.sort_by(f64::total_cmp);
    (runs[0], runs[runs.len() / 2])
}

fn cpu_description() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("model name") {
                return value.trim_start_matches([':', ' ']).trim().to_owned();
            }
        }
    }
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".into())
}

fn tool_version(program: &str, arguments: &[&str]) -> String {
    match Command::new(program).args(arguments).output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines().next().unwrap_or("unknown").trim().to_owned()
        }
        _ => format!("{program} unavailable"),
    }
}

fn report(label: &str, (best, median): (f64, f64)) {
    eprintln!("{label:<28} best {best:>10.2} ns   median {median:>10.2} ns");
}

#[test]
#[ignore = "performance measurement; run explicitly with --ignored --nocapture"]
fn debug_overhead_is_measured_and_release_artifacts_are_clean() {
    let Some(clang) = clang() else {
        return;
    };
    let production = production_project("forms");
    let hookfree_project = hookfree_project("hookfree");

    let release = build_release(&production);
    let debug = build_debug(&production);
    let hookfree = build_hookfree(&hookfree_project);

    // Non-timing preconditions: each form has the documented artifact shape.
    assert_eq!(debug.metadata_json["debug"]["hooks"], true);
    assert!(debug.checkpoints() > 0, "the debug form emits checkpoints");
    assert!(has_service_13(&debug), "the debug form imports service 13");
    assert!(
        debug.frames.is_file(),
        "the debug form writes a frame sidecar"
    );
    assert!(
        !has_service_13(&release),
        "the release form imports no service 13"
    );
    assert!(
        release.metadata_json.get("debug").is_none(),
        "the release metadata sidecar has no debug object"
    );
    assert!(
        !release.frames.exists(),
        "the release form writes no frame sidecar"
    );
    assert!(
        !release.ir_text().contains("!llvm.dbg") && !release.ir_text().contains("!dbg"),
        "release IR carries no debug metadata"
    );
    assert!(
        debug.ir_text().contains("\"frame-pointer\"=\"all\""),
        "hook-bearing debug functions force frame pointers"
    );
    assert!(
        !release.ir_text().contains("\"frame-pointer\"=\"all\""),
        "the release form does not force frame-pointer attributes"
    );
    assert_eq!(hookfree.metadata_json["debug"]["hooks"], false);
    assert_eq!(
        hookfree.checkpoints(),
        0,
        "the hook-free form has no checkpoint table"
    );
    assert!(
        matches!(
            hookfree.metadata_json["debug"]["info"].as_str(),
            Some("dwarf" | "codeview")
        ),
        "the hook-free debug form emits DWARF or CodeView: {}",
        hookfree.metadata_json["debug"]["info"]
    );
    assert!(
        !has_service_13(&hookfree),
        "the hook-free form imports no service 13"
    );
    assert!(
        !hookfree.frames.exists(),
        "the hook-free form writes no frame sidecar"
    );
    assert_ne!(
        debug.build_id(),
        release.build_id(),
        "debug and release build IDs differ"
    );
    assert_ne!(
        debug.manifest_digest(),
        release.manifest_digest(),
        "debug and release manifest digests differ"
    );

    eprintln!("debug overhead measurement");
    eprintln!(
        "host: os={} arch={} family={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::FAMILY
    );
    eprintln!("cpu: {}", cpu_description());
    eprintln!("rustc: {}", tool_version("rustc", &["-V"]));
    eprintln!("clang: {}", tool_version(&clang, &["--version"]));
    eprintln!(
        "method: same generator source; {CYCLES} cycles/run, best and median of {REPEATS} runs, \
         {WARMUP_CYCLES} warm-up cycles; host.cycle_at on a real clang-built module"
    );

    let release_times = cycle_times(&release);
    let hookfree_times = cycle_times(&hookfree);
    let debug_times = cycle_times(&debug);
    report("release (-O2, stripped)", release_times);
    report("debug hook-free (-O0 -g)", hookfree_times);
    report("debug hooks (-O0 -g)", debug_times);

    let hooks = debug.checkpoints() as f64;
    let hook_delta = debug_times.0 - hookfree_times.0;
    eprintln!(
        "checkpoints/cycle: {hooks:.0} (all sites in the executed functions run once per cycle)"
    );
    eprintln!(
        "hook cost: {:.2} ns/hook (best debug hooks - best debug hook-free, divided by {hooks:.0})",
        hook_delta / hooks
    );
    eprintln!(
        "relative: debug hooks / release = {:.2}x; debug hook-free / release = {:.2}x; \
         hook share of the debug cycle = {:.1}%",
        debug_times.0 / release_times.0,
        hookfree_times.0 / release_times.0,
        100.0 * hook_delta / debug_times.0
    );

    let session_times = cycle_times_with_session(&production, &debug);
    report("debug hooks + session", session_times);
    eprintln!(
        "active observe session over no session: {:.2} ns/cycle ({:.2} ns/hook), including \
         the per-cycle server pump",
        session_times.0 - debug_times.0,
        (session_times.0 - debug_times.0) / hooks
    );

    let reads = typed_read_times(&debug);
    report("typed read (host API)", reads);
    eprintln!(
        "typed read: one resolved `Acc` DINT copy per call through the in-process API, including \
         manifest target resolution and value decoding"
    );

    eprintln!(
        "note: the hook-free debug form is the documented test form (§17.7); at -O0 clang keeps \
         frame pointers on x86-64 anyway, but the forced `frame-pointer=\"all\"` attribute applies \
         only to the hook-bearing debug form."
    );
}
