//! M6.5 phase-6 driver frame-sidecar tests: build-time DWARF/CodeView
//! extraction, build-identity pairing, deterministic artifacts, omission for
//! release and unpaired builds, and a real stopped session with typed locals
//! against driver-produced artifacts.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use stc_got::json::Json;
use stc_runtime::{
    Candidate, DebugClient, DebugServerConfig, Host, HostPlatform, LoadMode, LoadOutcome,
    ManifestStore, PROFILE, TransportKind,
};

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
                    eprintln!("note: clang not found; skipping frame sidecar checks");
                    None
                }
            }
        })
        .as_ref()
        .copied()
}

/// Reports whether the backend reader the driver uses for this target exists.
fn extraction_tool_available() -> bool {
    let tool = if cfg!(windows) {
        "llvm-readobj"
    } else {
        "readelf"
    };
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-debug-frames-{}-{name}-{unique}",
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

    fn run(&self, directory: &str, args: &[&str]) -> Output {
        Command::new(stc())
            .args(args)
            .current_dir(self.path(directory))
            .output()
            .expect("run stc")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn manifest(name: &str, extra: &str) -> String {
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
         {extra}"
    )
}

const PROGRAM: &str = "\
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

PROGRAM Main
VAR
  n : DINT;
  total : DINT;
END_VAR
  n := Add(1, 2);
  IF n > 0 THEN
    total := total + n;
  END_IF;
  WHILE total < 10 DO
    total := total + 1;
  END_WHILE;
END_PROGRAM
";

fn scaffold(name: &str, extra: &str) -> TestDir {
    let project = TestDir::new(name);
    project.write("app/stc.toml", &manifest("app", extra));
    project.write("app/src/main.st", PROGRAM);
    project
}

fn read_metadata(project: &TestDir, relative: &str) -> Value {
    let text = std::fs::read_to_string(project.path(relative))
        .unwrap_or_else(|error| panic!("read metadata `{relative}`: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse metadata `{relative}`: {error}\n{text}"))
}

fn metadata_text(project: &TestDir, relative: &str) -> String {
    std::fs::read_to_string(project.path(relative))
        .unwrap_or_else(|error| panic!("read metadata `{relative}`: {error}"))
}

fn host_platform() -> HostPlatform {
    HostPlatform::new(
        PROFILE,
        stc_codegen::resolve_target(Some(&stc_codegen::host_triple())),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

// ---------------------------------------------------------------------------
// Sidecar validation
// ---------------------------------------------------------------------------

fn sidecar_functions(frames: &Value) -> &Vec<Value> {
    frames["functions"].as_array().expect("functions array")
}

fn validate_function(
    function: &Value,
    manifest_functions: &BTreeSet<String>,
    checkpoint_count: u64,
) -> usize {
    let got = function["function"].as_str().expect("function got id");
    assert!(
        got.starts_with("0x") && got.len() == 34,
        "function identity `{got}`"
    );
    assert!(
        manifest_functions.contains(got),
        "function {got} is a manifest function"
    );
    let symbol = function["symbol"].as_str().expect("symbol");
    assert!(!symbol.is_empty(), "a function symbol is required");
    assert!(
        symbol.chars().all(|character| character.is_ascii_graphic()),
        "a symbol is printable ASCII: {symbol:?}"
    );
    assert_eq!(function["cfa"], "fp+16");
    assert_eq!(function["saved_fp"], -16);
    assert_eq!(function["return_address"], -8);
    assert_eq!(function["low_pc"], 0);
    let high_pc = function["high_pc"].as_u64().expect("high_pc");
    assert!(
        (1..=64 * 1024 * 1024).contains(&high_pc),
        "high_pc {high_pc}"
    );
    let frame_size = function["frame_size"].as_u64().expect("frame_size");
    assert!(
        (16..=64 * 1024 * 1024).contains(&frame_size),
        "frame_size {frame_size}"
    );
    let locals = function["locals"].as_array().expect("locals array");
    let mut previous = String::new();
    for local in locals {
        let name = local["name"].as_str().expect("local name");
        assert!(!name.is_empty(), "a local name is required");
        assert!(
            name.chars().all(|character| character.is_ascii_uppercase()
                || character.is_ascii_digit()
                || character == '_'),
            "a local name is normalized uppercase: {name:?}"
        );
        assert!(
            previous.is_empty() || name > previous.as_str(),
            "locals are sorted by name in {symbol}: {previous} then {name}"
        );
        previous = name.to_owned();
        let size = local["size"].as_u64().expect("local size");
        assert!(size > 0, "local {name} has a positive size");
        assert!(
            local["type"].as_str().is_some_and(|ty| !ty.is_empty()),
            "local {name} has a canonical type"
        );
        let location = local["location"].as_str().expect("local location");
        let offset = location
            .strip_prefix("fp-")
            .unwrap_or_else(|| panic!("local {name} has an fp-relative location: {location}"))
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("local {name} offset: {location}"));
        assert!(offset >= size, "local {name} fits before the frame pointer");
        assert!(
            offset <= frame_size,
            "local {name} is inside the {frame_size}-byte frame"
        );
        let live = &local["live"];
        let first = live["first"].as_u64().expect("live.first");
        let last = live["last"].as_u64().expect("live.last");
        assert!(first >= 1, "local {name} live range starts at 1");
        assert!(last >= first, "local {name} live range is ordered");
        assert!(
            last <= checkpoint_count,
            "local {name} live range ends within the checkpoint table"
        );
    }
    locals.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn debug_build_writes_a_deterministic_paired_frame_sidecar() {
    let Some(_) = clang() else {
        return;
    };
    let project = scaffold("frames", "[build]\n");
    for out in ["out1", "out2"] {
        let build = project.run("app", &["build", "--emit", "module", "--out-dir", out]);
        assert_eq!(code(&build), 0, "{}", stderr(&build));
    }

    let metadata1 = read_metadata(&project, "app/out1/app.stc-meta.json");
    let metadata2 = read_metadata(&project, "app/out2/app.stc-meta.json");
    assert_eq!(
        metadata1["build_id"], metadata2["build_id"],
        "build identities are deterministic"
    );
    assert_eq!(metadata1["debug"]["format"], 1);
    assert_eq!(metadata1["debug"]["hooks"], true);
    let checkpoints = metadata1["debug"]["checkpoints"]
        .as_array()
        .expect("checkpoint table");
    assert!(!checkpoints.is_empty(), "a debug build carries checkpoints");
    let expected_info = if metadata1["target"]
        .as_str()
        .is_some_and(|target| target.contains("windows"))
    {
        "codeview"
    } else {
        "dwarf"
    };
    assert_eq!(metadata1["debug"]["info"], expected_info);

    let frames1 = project.path("app/out1/app.stc-frames.json");
    let frames2 = project.path("app/out2/app.stc-frames.json");
    if !frames1.exists() {
        assert!(
            !extraction_tool_available(),
            "an extraction tool is available but the frame sidecar was not written"
        );
        eprintln!("note: no DWARF/CodeView reader; skipping frame table assertions");
        return;
    }
    assert!(
        frames2.exists(),
        "every paired debug build writes the frame sidecar"
    );
    let contents = std::fs::read_to_string(&frames1).expect("read frame sidecar");
    let second = std::fs::read_to_string(&frames2).expect("read second frame sidecar");
    assert_eq!(contents, second, "frame sidecars are byte-deterministic");
    assert!(
        !contents.contains('/') && !contents.contains('\\'),
        "the sidecar contains no paths"
    );

    let frames: Value = serde_json::from_str(&contents).expect("frame sidecar JSON");
    assert_eq!(frames["format"], 1);
    assert_eq!(frames["build_id"], metadata1["build_id"]);
    assert_eq!(frames["pointer_width"], 8);
    assert_eq!(frames["target"], metadata1["target"]);

    let manifest_text = std::fs::read_to_string(project.path("app/out1/app.stc-got.json"))
        .expect("read manifest JSON");
    assert!(
        !manifest_text.contains("stc-frames"),
        "the manifest does not reference the additive frame sidecar"
    );
    assert!(
        !metadata_text(&project, "app/out1/app.stc-meta.json").contains("stc-frames"),
        "the metadata sidecar does not reference the additive frame sidecar"
    );
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifest JSON");
    let manifest_functions: BTreeSet<String> = manifest["body"]["functions"]
        .as_array()
        .expect("manifest functions")
        .iter()
        .filter_map(|function| function["got_id"].as_str().map(str::to_owned))
        .collect();

    let functions = sidecar_functions(&frames);
    assert!(!functions.is_empty(), "the sidecar describes functions");
    let mut previous = String::new();
    let mut local_total = 0usize;
    for function in functions {
        let got = function["function"].as_str().expect("function got id");
        assert!(
            previous.is_empty() || got > previous.as_str(),
            "functions are sorted and unique: {previous} then {got}"
        );
        previous = got.to_owned();
        local_total += validate_function(function, &manifest_functions, checkpoints.len() as u64);
    }
    assert!(
        local_total > 0,
        "at least one function carries described locals"
    );
}

#[test]
fn release_build_omits_the_frame_sidecar() {
    let Some(_) = clang() else {
        return;
    };
    let project = scaffold("frames-release", "[build]\n");
    let build = project.run(
        "app",
        &["build", "--release", "--emit", "module", "--out-dir", "out"],
    );
    assert_eq!(code(&build), 0, "{}", stderr(&build));
    assert!(
        !project.path("app/out/app.stc-frames.json").exists(),
        "release builds never carry stopped-frame tables"
    );
    let metadata = read_metadata(&project, "app/out/app.stc-meta.json");
    assert!(
        metadata.get("debug").is_none(),
        "release metadata carries no debug object: {metadata}"
    );
    assert!(
        !metadata_text(&project, "app/out/app.stc-meta.json").contains("stc-frames"),
        "release metadata is unchanged"
    );
}

#[test]
fn frame_sidecar_requires_a_paired_module_artifact() {
    let project = scaffold("frames-unpaired", "[build]\n");
    let build = project.run(
        "app",
        &["build", "--emit", "ir,metadata", "--out-dir", "out"],
    );
    assert_eq!(code(&build), 0, "{}", stderr(&build));
    assert!(
        !project.path("app/out/app.stc-frames.json").exists(),
        "a debug build without a module artifact writes no frame sidecar"
    );
    let metadata = read_metadata(&project, "app/out/app.stc-meta.json");
    assert_eq!(metadata["debug"]["hooks"], true);
    assert!(
        !metadata["debug"]["checkpoints"]
            .as_array()
            .expect("checkpoints")
            .is_empty(),
        "the same build does carry checkpoints"
    );
}

// ---------------------------------------------------------------------------
// Protocol helpers for the driver-built module
// ---------------------------------------------------------------------------

fn obj<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    )
}

fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
    match value.as_object().and_then(|object| object.get(key)) {
        Some(field) => field,
        None => panic!("field `{key}` in {}", value.render_compact()),
    }
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    field(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("string `{key}` in {}", value.render_compact()))
}

fn number(value: &Json, key: &str) -> u64 {
    field(value, key)
        .as_number()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_else(|| panic!("number `{key}` in {}", value.render_compact()))
}

fn boolean(value: &Json, key: &str) -> bool {
    field(value, key)
        .as_bool()
        .unwrap_or_else(|| panic!("bool `{key}` in {}", value.render_compact()))
}

fn stop_json(status: &Json) -> Option<&Json> {
    let stop = field(status, "stop");
    stop.as_object().map(|_| stop)
}

fn next_event_named(client: &mut DebugClient, expected: &str) -> Json {
    loop {
        let event = client.next_event().expect("event");
        let kind = event
            .as_object()
            .and_then(|object| object.get("kind"))
            .and_then(Json::as_str);
        assert_eq!(kind, Some("event"));
        match text(&event, "event") {
            "watch" => {}
            name if name == expected => return field(&event, "data").clone(),
            name => panic!(
                "expected a `{expected}` event, got `{name}`: {}",
                event.render_compact()
            ),
        }
    }
}

fn wait_stopped(client: &mut DebugClient, timeout: Duration) -> Json {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = client
            .call_ok("breakpoint.list", obj([]))
            .expect("breakpoint.list");
        let status = client
            .call_ok("session.status", obj([]))
            .expect("session.status");
        if let Some(stop) = stop_json(&status) {
            return stop.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no stop within {timeout:?}: {}",
            status.render_compact()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn driver_sidecar_supports_a_real_stop_with_typed_locals() {
    let Some(_) = clang() else {
        return;
    };
    let project = scaffold("frames-runtime", "[build]\n");
    let build = project.run("app", &["build", "--emit", "module", "--out-dir", "out"]);
    assert_eq!(code(&build), 0, "{}", stderr(&build));
    let frames_path = project.path("app/out/app.stc-frames.json");
    if !frames_path.exists() {
        assert!(
            !extraction_tool_available(),
            "an extraction tool is available but the real frame sidecar is missing"
        );
        eprintln!("note: no DWARF/CodeView reader; skipping the stopped-session check");
        return;
    }
    let metadata = read_metadata(&project, "app/out/app.stc-meta.json");
    let manifest_text = std::fs::read_to_string(project.path("app/out/app.stc-got.json"))
        .expect("read manifest JSON");
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifest JSON");
    let manifest_functions = manifest["body"]["functions"]
        .as_array()
        .expect("manifest functions");
    let find_function = |suffix: &str| {
        manifest_functions
            .iter()
            .find(|function| {
                function["key"]
                    .as_str()
                    .is_some_and(|key| key.split('@').next().unwrap_or_default().ends_with(suffix))
            })
            .unwrap_or_else(|| panic!("manifest function ending with `{suffix}`"))
            .clone()
    };
    let add = find_function("::ADD");
    let main_body = find_function("::MAIN#BODY");
    let add_got = add["got_id"].as_str().expect("Add got id").to_owned();
    let main_got = main_body["got_id"]
        .as_str()
        .expect("Main got id")
        .to_owned();

    let checkpoints = metadata["debug"]["checkpoints"]
        .as_array()
        .expect("checkpoint table");
    let checkpoint = checkpoints
        .iter()
        .find(|checkpoint| {
            checkpoint["function"].as_str() == Some(add_got.as_str())
                && checkpoint["line"].as_u64() == Some(6)
        })
        .or_else(|| {
            checkpoints
                .iter()
                .find(|checkpoint| checkpoint["function"].as_str() == Some(add_got.as_str()))
        })
        .expect("Add checkpoint");
    let checkpoint_id = checkpoint["id"].as_u64().expect("checkpoint id");
    let line = checkpoint["line"].as_u64().expect("checkpoint line");

    let frames: Value =
        serde_json::from_str(&std::fs::read_to_string(&frames_path).expect("read frame sidecar"))
            .expect("frame sidecar JSON");
    let add_frame = sidecar_functions(&frames)
        .iter()
        .find(|function| function["function"].as_str() == Some(add_got.as_str()))
        .expect("Add frame description");
    let add_symbol = add_frame["symbol"].as_str().expect("Add symbol").to_owned();

    let platform = host_platform();
    let module_file = metadata["module"]["file"].as_str().expect("module file");
    let module = project.path(&format!("app/out/{module_file}"));
    let manifest_path = project.path("app/out/app.stc-got.bin");
    let candidate = Candidate::open(&platform, &module, &manifest_path)
        .unwrap_or_else(|error| panic!("candidate: {error}"));
    let mut host = Host::new(platform, ManifestStore::memory());
    let LoadOutcome::ColdInstalled { generation } =
        host.load(candidate, LoadMode::Auto).expect("cold install")
    else {
        panic!("expected a cold install");
    };
    let endpoint_dir = TestDir::new("frames-runtime-endpoint");
    let endpoint = endpoint_dir.path("debug.endpoint.json");
    let mut config = DebugServerConfig::new(endpoint.clone(), TransportKind::Socket);
    config.session_timeout = Duration::from_secs(30);
    host.start_debug_server(&config).expect("debug server");
    let mut client = DebugClient::connect_file(&endpoint).expect("client");

    let started = client
        .start_session(&["observe", "control"])
        .expect("session.start");
    assert!(
        boolean(field(&started, "capabilities"), "stepping"),
        "the driver sidecar must validate against the real module: {}",
        started.render_compact()
    );

    let response = client
        .call(
            "breakpoint.set",
            obj([(
                "target",
                obj([
                    ("kind", Json::string("checkpoint")),
                    ("checkpoint_id", Json::unsigned(checkpoint_id)),
                ]),
            )]),
        )
        .expect("breakpoint.set");
    assert_eq!(text(&response, "status"), "ok");
    assert!(boolean(field(&response, "result"), "bound"));

    let cycle = std::thread::scope(|scope| {
        let host_ref = &mut host;
        let handle = scope.spawn(move || host_ref.cycle_at(1_000_000));
        let stopped = wait_stopped(&mut client, Duration::from_secs(15));
        assert_eq!(text(&stopped, "reason"), "breakpoint");
        assert_eq!(number(&stopped, "checkpoint_id"), checkpoint_id);
        let stop_id = number(&stopped, "stop_id");
        let event = next_event_named(&mut client, "stopped");
        assert_eq!(number(&event, "stop_id"), stop_id);

        let response = client
            .call("stack.list", obj([("stop_id", Json::unsigned(stop_id))]))
            .expect("stack.list");
        assert_eq!(text(&response, "status"), "ok");
        let stack = field(&response, "result");
        let frames = field(stack, "frames").as_array().expect("frames");
        assert!(frames.len() >= 2, "expected a Main caller: {stack:?}");
        assert_eq!(text(&frames[0], "function"), add_got);
        assert_eq!(text(&frames[0], "symbol"), add_symbol);
        assert!(!boolean(&frames[0], "optimized_out"));
        let source = field(&frames[0], "source");
        assert_eq!(text(source, "file"), "src/main.st");
        assert_eq!(number(source, "line"), line);
        assert_eq!(text(&frames[1], "function"), main_got);
        assert!(!boolean(&frames[1], "optimized_out"));

        let response = client
            .call(
                "frame.read",
                obj([
                    (
                        "target",
                        obj([
                            ("kind", Json::string("local")),
                            ("frame", Json::unsigned(0)),
                            ("name", Json::string("A")),
                        ]),
                    ),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            )
            .expect("frame.read");
        assert_eq!(text(&response, "status"), "ok");
        let local = field(&response, "result");
        assert!(boolean(local, "available"));
        assert_eq!(text(field(local, "value"), "type"), "DINT");
        assert_eq!(text(field(local, "value"), "value"), "1");

        let response = client
            .call("exec.continue", obj([("stop_id", Json::unsigned(stop_id))]))
            .expect("exec.continue");
        assert_eq!(text(&response, "status"), "ok");
        let resumed = next_event_named(&mut client, "resumed");
        assert_eq!(number(&resumed, "stop_id"), stop_id);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert!(
        matches!(cycle, stc_runtime::CycleOutcome::Ran { generation: actual } if actual == generation),
        "expected the program cycle to complete, got {cycle:?}"
    );

    host.stop_debug_server();
    drop(client);
    assert!(stdout(&build).is_empty() || !stdout(&build).contains("error"));
}
