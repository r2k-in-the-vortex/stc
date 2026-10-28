//! M6.5 phase-6 stopped-frame tests: breakpoints, stop/continue, stepping,
//! call-stack capture, typed locals, pause bound, and sidecar availability.
//!
//! The fixtures compile a real C module that calls host service 13 at
//! checkpoints named by real compiler debug metadata. The metadata sidecar and
//! the additive frame sidecar are produced at fixture-build time; the runtime
//! validates both exactly as it does for driver-produced artifacts.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use stc_got::GotId;
use stc_got::codes;
use stc_got::json::Json;
use stc_runtime::{
    Candidate, CycleOutcome, DebugClient, DebugServerConfig, DebugServerInfo, Host, HostPlatform,
    LoadMode, LoadOutcome, ManifestStore, PROFILE, TransportKind,
};

use common::{Behavior, BuildFlags, Fixture, StopFixture, StopFlow};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Serializes tests that install the process-global stop session.
static STOP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn stop_guard() -> std::sync::MutexGuard<'static, ()> {
    STOP_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn platform() -> HostPlatform {
    HostPlatform::new(
        PROFILE,
        common::target(),
        stc_got::HOST_ABI_MAJOR,
        stc_got::HOST_ABI_MINOR,
    )
}

fn in_memory_host() -> Host {
    Host::new(platform(), ManifestStore::memory())
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stc-debug-stop-{}-{id}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test directory");
        Self { path }
    }

    fn endpoint(&self) -> PathBuf {
        self.path.join("debug.endpoint.json")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A deliberate damage to the paired debug sidecars, applied before load.
#[derive(Clone, Copy, Debug)]
enum Sidecar {
    /// Both sidecars are the fixture's valid pair.
    Valid,
    /// No stopped-frame sidecar exists.
    Missing,
    /// The stopped-frame sidecar is not a valid table.
    Malformed,
    /// The stopped-frame sidecar names a different build.
    WrongBuildId,
    /// The metadata sidecar (checkpoint table) is absent.
    MissingMetadata,
}

fn apply_sidecar(fixture: &StopFixture, sidecar: Sidecar) {
    match sidecar {
        Sidecar::Valid => {}
        Sidecar::Missing => {
            fs::remove_file(fixture.frames_path()).expect("remove frame sidecar");
        }
        Sidecar::Malformed => {
            fs::write(fixture.frames_path(), b"{\"format\":1}").expect("damage frame sidecar");
        }
        Sidecar::WrongBuildId => {
            let path = fixture.frames_path();
            let text = fs::read_to_string(&path).expect("read frame sidecar");
            let build = fixture.build_id.to_hex();
            assert!(
                text.contains(&build),
                "the fixture frame sidecar carries its build id"
            );
            let replacement = GotId::ZERO.to_hex();
            fs::write(&path, text.replacen(&build, &replacement, 1))
                .expect("mispaired frame sidecar");
        }
        Sidecar::MissingMetadata => {
            fs::remove_file(fixture.metadata_path()).expect("remove metadata sidecar");
        }
    }
}

struct StopSetup {
    host: Host,
    dir: TestDir,
    info: DebugServerInfo,
    fixture: StopFixture,
    generation: u64,
}

impl StopSetup {
    fn new(name: &str, flow: StopFlow, timeout: Duration, sidecar: Sidecar) -> Option<Self> {
        let output = common::output_with_flags(
            common::STOP_SOURCE,
            name,
            BuildFlags {
                debug_hooks: true,
                ..BuildFlags::default()
            },
        );
        let fixture = StopFixture::build(name, &output, flow)?;
        apply_sidecar(&fixture, sidecar);
        let mut host = in_memory_host();
        let candidate = Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path)
            .expect("candidate");
        let outcome = host.load(candidate, LoadMode::Auto).expect("cold install");
        let LoadOutcome::ColdInstalled { generation } = outcome else {
            panic!("expected a cold install, got {outcome:?}");
        };
        let dir = TestDir::new(name);
        let mut config = DebugServerConfig::new(dir.endpoint(), TransportKind::Socket);
        config.allow_write = true;
        config.allow_force = true;
        config.session_timeout = timeout;
        let info = host.start_debug_server(&config).expect("debug server");
        Some(Self {
            host,
            dir,
            info,
            fixture,
            generation,
        })
    }

    fn client(&self) -> DebugClient {
        DebugClient::connect_file(&self.dir.endpoint()).expect("client")
    }

    fn finish(mut self) {
        self.host.stop_debug_server();
    }
}

// ---------------------------------------------------------------------------
// JSON and protocol helpers
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

fn element_number(value: &Json) -> u64 {
    value
        .as_number()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_else(|| panic!("number in {}", value.render_compact()))
}

fn optional_number(value: &Json, key: &str) -> Option<u64> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Json::as_number)
        .and_then(|value| u64::try_from(value).ok())
}

fn boolean(value: &Json, key: &str) -> bool {
    field(value, key)
        .as_bool()
        .unwrap_or_else(|| panic!("bool `{key}` in {}", value.render_compact()))
}

fn optional_text<'a>(value: &'a Json, key: &str) -> Option<&'a str> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Json::as_str)
}

fn message_kind(message: &Json) -> Option<&str> {
    message
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(Json::as_str)
}

fn event_name(event: &Json) -> &str {
    text(event, "event")
}

fn check_target(checkpoint_id: u32) -> Json {
    obj([
        ("kind", Json::string("checkpoint")),
        ("checkpoint_id", Json::unsigned(u64::from(checkpoint_id))),
    ])
}

fn source_target(file: &str, line: u32, column: u32) -> Json {
    obj([
        ("kind", Json::string("source")),
        ("file", Json::string(file)),
        ("line", Json::unsigned(u64::from(line))),
        ("column", Json::unsigned(u64::from(column))),
    ])
}

fn path_target(path: &str) -> Json {
    obj([("kind", Json::string("path")), ("key", Json::string(path))])
}

fn local_target(frame: u64, name: &str) -> Json {
    obj([
        ("kind", Json::string("local")),
        ("frame", Json::unsigned(frame)),
        ("name", Json::string(name)),
    ])
}

fn decimal_value(type_key: &str, value: &str) -> Json {
    obj([
        ("type", Json::string(type_key)),
        ("value", Json::string(value)),
    ])
}

fn stop_id_params(stop_id: u64) -> Json {
    obj([("stop_id", Json::unsigned(stop_id))])
}

fn session_start_params() -> Json {
    obj([
        (
            "client",
            obj([
                ("name", Json::string("stc-debug-stop")),
                ("version", Json::string("1")),
            ]),
        ),
        (
            "scopes",
            Json::Array(vec![
                Json::string("observe"),
                Json::string("control"),
                Json::string("write"),
                Json::string("force"),
            ]),
        ),
    ])
}

/// Sends one request and returns the complete response object.
fn request(client: &mut DebugClient, op: &str, params: Json) -> Json {
    client
        .call(op, params)
        .unwrap_or_else(|error| panic!("{op}: {error}"))
}

fn result_of(response: &Json) -> &Json {
    assert_eq!(
        text(response, "status"),
        "ok",
        "expected success: {}",
        response.render_compact()
    );
    field(response, "result")
}

fn error_code(response: &Json) -> &str {
    assert_eq!(
        text(response, "status"),
        "error",
        "expected an error: {}",
        response.render_compact()
    );
    text(field(response, "error"), "code")
}

fn warning_codes(response: &Json) -> Vec<String> {
    field(response, "warnings")
        .as_array()
        .expect("warnings array")
        .iter()
        .map(|warning| text(warning, "code").to_owned())
        .collect()
}

fn ok(client: &mut DebugClient, op: &str, params: Json) -> Json {
    let response = request(client, op, params);
    result_of(&response).clone()
}

fn expect_error(client: &mut DebugClient, op: &str, params: Json, code: &str) -> Json {
    let response = request(client, op, params);
    assert_eq!(
        error_code(&response),
        code,
        "op `{op}`: {}",
        response.render_compact()
    );
    response
}

fn session_start_response(client: &mut DebugClient) -> Json {
    request(client, "session.start", session_start_params())
}

fn session_start(client: &mut DebugClient) -> Json {
    let response = session_start_response(client);
    result_of(&response).clone()
}

fn session_status(client: &mut DebugClient) -> Json {
    ok(client, "session.status", obj([]))
}

fn stop_json(status: &Json) -> Option<&Json> {
    let stop = field(status, "stop");
    stop.as_object().map(|_| stop)
}

fn set_checkpoint(client: &mut DebugClient, checkpoint_id: u32) -> Json {
    let response = request(
        client,
        "breakpoint.set",
        obj([("target", check_target(checkpoint_id))]),
    );
    result_of(&response).clone()
}

fn next_event(client: &mut DebugClient) -> Json {
    client.next_event().expect("event")
}

fn next_stopped(client: &mut DebugClient) -> Json {
    loop {
        let event = next_event(client);
        assert_eq!(message_kind(&event), Some("event"));
        match event_name(&event) {
            "watch" => {}
            "stopped" => return field(&event, "data").clone(),
            other => panic!(
                "expected a stopped event, got `{other}`: {}",
                event.render_compact()
            ),
        }
    }
}

fn next_resumed(client: &mut DebugClient) -> Json {
    loop {
        let event = next_event(client);
        assert_eq!(message_kind(&event), Some("event"));
        match event_name(&event) {
            "watch" => {}
            "resumed" => return field(&event, "data").clone(),
            other => panic!(
                "expected a resumed event, got `{other}`: {}",
                event.render_compact()
            ),
        }
    }
}

fn next_session_ended(client: &mut DebugClient) -> Json {
    loop {
        let event = next_event(client);
        assert_eq!(message_kind(&event), Some("event"));
        match event_name(&event) {
            "watch" => {}
            "session-ended" => return field(&event, "data").clone(),
            other => panic!(
                "expected a session-ended event, got `{other}`: {}",
                event.render_compact()
            ),
        }
    }
}

/// Waits for a capture by polling the status together with a control
/// operation, because `session.status` alone does not refresh session activity
/// and the cycle may still be running.
fn wait_stopped(client: &mut DebugClient, timeout: Duration) -> Json {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = ok(client, "breakpoint.list", obj([]));
        let status = session_status(client);
        if let Some(stop) = stop_json(&status) {
            return stop.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no stop within {timeout:?}: {}",
            status.render_compact()
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn wait_queued(client: &mut DebugClient, expected: u64) -> Json {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = session_status(client);
        if number(&status, "queued_writes") == expected {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "queued_writes did not reach {expected}: {}",
            status.render_compact()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn root_record<'a>(
    artifacts: &'a stc_codegen::module::ModuleArtifacts,
    key: &str,
) -> &'a stc_got::RootRecord {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key == key || root.key.ends_with(&format!("::{key}")))
        .unwrap_or_else(|| panic!("root `{key}`"))
}

fn assert_ran(cycle: CycleOutcome, generation: u64) {
    assert!(
        matches!(cycle, CycleOutcome::Ran { generation: actual } if actual == generation),
        "expected the cycle to run, got {cycle:?}"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn breakpoint_stop_reports_frames_and_typed_locals() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "stop-frames",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    assert_eq!(setup.info.generation, setup.generation);
    let mut client = setup.client();
    let started = session_start(&mut client);
    let capabilities = field(&started, "capabilities");
    assert!(boolean(capabilities, "hooks"));
    assert!(boolean(capabilities, "checkpoints"));
    assert!(boolean(capabilities, "stepping"));

    let breakpoint = client
        .call_ok(
            "breakpoint.set",
            obj([("target", check_target(setup.fixture.hooks.inner_first.id))]),
        )
        .expect("breakpoint.set");
    assert!(boolean(&breakpoint, "bound"));
    assert_eq!(number(&breakpoint, "breakpoint_id"), 1);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        assert_eq!(text(&stopped, "reason"), "breakpoint");
        assert_eq!(
            number(&stopped, "checkpoint_id"),
            u64::from(setup.fixture.hooks.inner_first.id)
        );
        assert_eq!(number(&stopped, "breakpoint_id"), 1);
        let stop_id = number(&stopped, "stop_id");

        let stack = client
            .call_ok("stack.list", obj([("stop_id", Json::unsigned(stop_id))]))
            .expect("stack.list");
        let frames = field(&stack, "frames").as_array().expect("frames");
        assert!(frames.len() >= 2, "expected caller frames: {stack:?}");
        assert!(!boolean(&stack, "truncated"));
        assert_eq!(number(&frames[0], "index"), 0);
        assert_eq!(text(&frames[0], "function"), setup.fixture.inner.to_hex());
        assert_eq!(text(&frames[0], "symbol"), setup.fixture.inner_symbol);
        assert!(!boolean(&frames[0], "optimized_out"));
        let source = field(&frames[0], "source");
        assert_eq!(text(source, "file"), setup.fixture.hooks.inner_first.file);
        assert_eq!(
            number(source, "line"),
            u64::from(setup.fixture.hooks.inner_first.line)
        );
        assert_eq!(
            number(source, "column"),
            u64::from(setup.fixture.hooks.inner_first.column)
        );
        assert_eq!(number(&frames[1], "index"), 1);
        assert_eq!(text(&frames[1], "function"), setup.fixture.outer.to_hex());
        assert_eq!(text(&frames[1], "symbol"), setup.fixture.outer_symbol);
        assert!(!boolean(&frames[1], "optimized_out"));

        let local = client
            .call_ok(
                "frame.read",
                obj([
                    ("target", local_target(0, "A")),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            )
            .expect("frame.read A");
        assert!(boolean(&local, "available"));
        let value = field(&local, "value");
        assert_eq!(text(value, "type"), "DINT");
        assert_eq!(text(value, "value"), "11");
        assert_eq!(number(&local, "cycle"), 1);
        let view = field(&local, "view");
        assert_eq!(text(view, "origin"), "stop_snapshot");
        assert!(!boolean(view, "manifest_assisted"));
        assert!(boolean(view, "contiguous"));
        let node = field(view, "view");
        assert_eq!(text(node, "kind"), "scalar");
        assert!(boolean(node, "available"));
        assert!(field(node, "root").as_object().is_none());

        let caller_local = client
            .call_ok(
                "frame.read",
                obj([
                    ("target", local_target(1, "CA")),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            )
            .expect("frame.read CA");
        assert!(boolean(&caller_local, "available"));
        assert_eq!(text(field(&caller_local, "value"), "value"), "33");

        let unavailable = client
            .call_ok(
                "frame.read",
                obj([
                    ("target", local_target(0, "C")),
                    ("stop_id", Json::unsigned(stop_id)),
                ]),
            )
            .expect("frame.read C");
        assert!(!boolean(&unavailable, "available"));
        assert_eq!(
            text(field(field(&unavailable, "value"), "value"), "code"),
            codes::W_DEBUG_UNAVAILABLE
        );
        let view = field(&unavailable, "view");
        let node = field(view, "view");
        assert_eq!(text(node, "kind"), "unavailable");
        assert!(!boolean(node, "available"));
        assert_eq!(
            text(field(node, "value"), "code"),
            codes::W_DEBUG_UNAVAILABLE
        );

        client
            .call_ok("exec.continue", obj([("stop_id", Json::unsigned(stop_id))]))
            .expect("exec.continue");
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn breakpoint_source_binding_list_clear_and_diagnostic_codes() {
    let _guard = stop_guard();
    let Some(setup) = StopSetup::new(
        "bp-binding",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };

    // The phase-6 fixture keeps the host ABI 2.3 / descriptor contract.
    assert_eq!(setup.info.generation, setup.generation);
    assert_eq!(setup.info.mode, "debug");
    assert_eq!(setup.info.build_id, setup.fixture.build_id);
    let descriptor = &setup.fixture.artifacts.descriptor;
    assert_eq!(descriptor.host_abi_minor, 3);
    assert_ne!(
        descriptor.entry_flags & stc_got::descriptor::entry_flag::DEBUG_HOOKS,
        0
    );
    assert_ne!(
        descriptor.required_features & stc_got::descriptor::feature::DEBUG_HOOKS,
        0
    );
    assert_eq!(
        setup.fixture.artifacts.manifest.header.build_id,
        setup.fixture.build_id
    );

    let mut client = setup.client();
    let started = session_start(&mut client);
    let capabilities = field(&started, "capabilities");
    assert!(boolean(capabilities, "stepping"));

    let site = setup.fixture.hooks.inner_first.clone();
    // A requested column of zero selects the first checkpoint on the line.
    let response = request(
        &mut client,
        "breakpoint.set",
        obj([("target", source_target(&site.file, site.line, 0))]),
    );
    let bound = result_of(&response).clone();
    assert!(boolean(&bound, "bound"));
    assert_eq!(number(&bound, "breakpoint_id"), 1);
    let checkpoints = field(&bound, "checkpoints")
        .as_array()
        .expect("checkpoints");
    assert_eq!(
        checkpoints.iter().map(element_number).collect::<Vec<_>>(),
        [u64::from(site.id)]
    );
    assert_eq!(text(field(&bound, "source"), "file"), site.file);
    assert_eq!(
        number(field(&bound, "source"), "line"),
        u64::from(site.line)
    );
    assert_eq!(
        number(field(&bound, "source"), "column"),
        u64::from(site.column)
    );

    // A requested column at or after a checkpoint binds without moving it.
    let response = request(
        &mut client,
        "breakpoint.set",
        obj([("target", source_target(&site.file, site.line, site.column))]),
    );
    let at_or_after = result_of(&response).clone();
    assert!(boolean(&at_or_after, "bound"));
    assert_eq!(number(&at_or_after, "breakpoint_id"), 2);

    // An unbound source location stays unbound and carries `W-001`.
    let response = request(
        &mut client,
        "breakpoint.set",
        obj([("target", source_target("src/main.st", 1, 1))]),
    );
    let unbound = result_of(&response).clone();
    assert!(!boolean(&unbound, "bound"));
    assert_eq!(
        field(&unbound, "checkpoints")
            .as_array()
            .expect("checkpoints")
            .len(),
        0
    );
    assert_eq!(
        warning_codes(&response),
        vec![codes::W_DEBUG_UNBOUND.to_owned()]
    );
    assert_eq!(number(field(&unbound, "source"), "line"), 1);

    // A positive but unknown checkpoint is `E-008`; a zero checkpoint is a
    // malformed request (`E-003`).
    expect_error(
        &mut client,
        "breakpoint.set",
        obj([("target", check_target(setup.fixture.checkpoint_count + 1))]),
        codes::DEBUG_UNKNOWN,
    );
    expect_error(
        &mut client,
        "breakpoint.set",
        obj([("target", check_target(0))]),
        codes::DEBUG_REQUEST,
    );

    let response = request(&mut client, "breakpoint.list", obj([]));
    let listed = result_of(&response).clone();
    let breakpoints = field(&listed, "breakpoints")
        .as_array()
        .expect("breakpoints");
    assert_eq!(breakpoints.len(), 3);
    assert_eq!(
        breakpoints
            .iter()
            .map(|info| number(info, "breakpoint_id"))
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        warning_codes(&response),
        vec![codes::W_DEBUG_UNBOUND.to_owned()]
    );

    expect_error(
        &mut client,
        "breakpoint.clear",
        obj([("breakpoint_id", Json::unsigned(999))]),
        codes::DEBUG_UNKNOWN,
    );
    for id in [2_u64, 1, 3] {
        let response = request(
            &mut client,
            "breakpoint.clear",
            obj([("breakpoint_id", Json::unsigned(id))]),
        );
        assert!(boolean(result_of(&response), "cleared"));
    }
    let listed = ok(&mut client, "breakpoint.list", obj([]));
    assert_eq!(
        field(&listed, "breakpoints")
            .as_array()
            .expect("breakpoints")
            .len(),
        0
    );

    setup.finish();
}

#[test]
fn stop_status_stale_ids_and_continue() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "stop-status",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    let breakpoint = set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);
    assert!(boolean(&breakpoint, "bound"));

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let stop_id = number(&stopped, "stop_id");
        let checkpoints = &setup.fixture.hooks;

        let status = session_status(&mut client);
        assert_eq!(text(&status, "state"), "stopped");
        assert_eq!(number(&status, "breakpoints"), 1);
        assert_eq!(number(&status, "queued_writes"), 0);
        assert_eq!(number(&status, "watches"), 0);
        let stop = stop_json(&status).expect("stop status");
        assert_eq!(number(stop, "stop_id"), stop_id);
        assert_eq!(text(stop, "reason"), "breakpoint");
        assert_eq!(
            number(stop, "checkpoint_id"),
            u64::from(checkpoints.inner_first.id)
        );
        assert_eq!(number(stop, "breakpoint_id"), 1);
        assert_eq!(
            text(field(stop, "source"), "file"),
            checkpoints.inner_first.file
        );
        let remaining = number(&status, "pause_remaining_ms");
        assert!(
            remaining > 0 && remaining <= 60_000,
            "remaining {remaining}"
        );

        let stale = stop_id + 1000;
        expect_error(
            &mut client,
            "stack.list",
            stop_id_params(stale),
            codes::DEBUG_STOP,
        );
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(0, "A")),
                ("stop_id", Json::unsigned(stale)),
            ]),
            codes::DEBUG_STOP,
        );
        expect_error(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("over")),
                ("stop_id", Json::unsigned(stale)),
            ]),
            codes::DEBUG_STOP,
        );
        expect_error(
            &mut client,
            "exec.continue",
            stop_id_params(stale),
            codes::DEBUG_STOP,
        );

        let response = request(&mut client, "exec.continue", stop_id_params(stop_id));
        let continued = result_of(&response).clone();
        assert_eq!(text(&continued, "status"), "running");
        assert_eq!(number(&continued, "stop_id"), stop_id);
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), stop_id);

        let status = session_status(&mut client);
        assert_eq!(text(&status, "state"), "active");
        assert!(stop_json(&status).is_none());
        assert!(field(&status, "pause_remaining_ms").is_null());
        assert_eq!(number(&status, "breakpoints"), 1);

        // The snapshot is invalidated as soon as the cycle resumes.
        expect_error(
            &mut client,
            "stack.list",
            stop_id_params(stop_id),
            codes::DEBUG_STOP,
        );
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(0, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_STOP,
        );
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn pause_request_stops_at_next_checkpoint() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "pause",
        StopFlow::Pause,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    let response = request(&mut client, "exec.pause", obj([]));
    assert_eq!(text(result_of(&response), "status"), "pause-requested");

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = wait_stopped(&mut client, Duration::from_secs(10));
        assert_eq!(text(&stopped, "reason"), "pause");
        assert_eq!(
            number(&stopped, "checkpoint_id"),
            u64::from(setup.fixture.hooks.inner_first.id)
        );
        assert!(optional_number(&stopped, "breakpoint_id").is_none());
        let event = next_stopped(&mut client);
        assert_eq!(number(&event, "stop_id"), number(&stopped, "stop_id"));

        let status = session_status(&mut client);
        assert_eq!(text(&status, "state"), "stopped");
        assert_eq!(number(&status, "breakpoints"), 0);
        assert!(number(&status, "pause_remaining_ms") > 0);

        let response = request(
            &mut client,
            "exec.continue",
            stop_id_params(number(&stopped, "stop_id")),
        );
        assert_eq!(text(result_of(&response), "status"), "running");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), number(&stopped, "stop_id"));
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn pause_timeout_ends_session_and_resumes_cycle() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "pause-timeout",
        StopFlow::Pause,
        Duration::from_secs(1),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    let response = request(&mut client, "exec.pause", obj([]));
    assert_eq!(text(result_of(&response), "status"), "pause-requested");

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = wait_stopped(&mut client, Duration::from_secs(10));
        assert_eq!(text(&stopped, "reason"), "pause");
        let remaining = number(&session_status(&mut client), "pause_remaining_ms");
        assert!(remaining > 0 && remaining <= 1_000, "remaining {remaining}");
        let event = next_stopped(&mut client);
        assert_eq!(number(&event, "stop_id"), number(&stopped, "stop_id"));

        let ended = next_session_ended(&mut client);
        assert_eq!(text(&ended, "reason"), "pause-timeout");
        assert_eq!(text(&ended, "code"), codes::DEBUG_SESSION_TIMEOUT);
        assert_eq!(number(&ended, "generation"), setup.generation);
        assert_eq!(text(&ended, "build_id"), setup.fixture.build_id.to_hex());
        handle.join().expect("cycle thread").expect("cycle")
    });
    // The cycle resumes and completes; no state from the timed-out stop is
    // applied to a later session.
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn step_over_skips_the_callee() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "step-over",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.outer_before.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        assert_eq!(
            number(&stopped, "checkpoint_id"),
            u64::from(setup.fixture.hooks.outer_before.id)
        );
        let stop_id = number(&stopped, "stop_id");

        let response = request(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("over")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
        );
        let stepped = result_of(&response).clone();
        assert_eq!(text(&stepped, "status"), "stepping");
        assert_eq!(text(&stepped, "mode"), "over");
        assert_eq!(number(&stepped, "stop_id"), stop_id);

        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), stop_id);
        let next = next_stopped(&mut client);
        assert_eq!(text(&next, "reason"), "step");
        assert_eq!(
            number(&next, "checkpoint_id"),
            u64::from(setup.fixture.hooks.outer_after.id)
        );
        assert_ne!(number(&next, "stop_id"), stop_id);
        assert!(optional_number(&next, "breakpoint_id").is_none());

        let response = request(
            &mut client,
            "exec.continue",
            stop_id_params(number(&next, "stop_id")),
        );
        assert_eq!(text(result_of(&response), "status"), "running");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), number(&next, "stop_id"));
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn step_into_then_out_crosses_call_boundaries() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "step-into-out",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let first_id = number(&stopped, "stop_id");

        let response = request(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("into")),
                ("stop_id", Json::unsigned(first_id)),
            ]),
        );
        assert_eq!(text(result_of(&response), "status"), "stepping");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), first_id);
        let inner = next_stopped(&mut client);
        assert_eq!(text(&inner, "reason"), "step");
        assert_eq!(
            number(&inner, "checkpoint_id"),
            u64::from(setup.fixture.hooks.inner_second.id)
        );
        let inner_id = number(&inner, "stop_id");

        let response = request(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("out")),
                ("stop_id", Json::unsigned(inner_id)),
            ]),
        );
        assert_eq!(text(result_of(&response), "status"), "stepping");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), inner_id);
        let outer = next_stopped(&mut client);
        assert_eq!(text(&outer, "reason"), "step");
        assert_eq!(
            number(&outer, "checkpoint_id"),
            u64::from(setup.fixture.hooks.outer_after.id)
        );
        assert_ne!(number(&outer, "stop_id"), inner_id);

        let response = request(
            &mut client,
            "exec.continue",
            stop_id_params(number(&outer, "stop_id")),
        );
        assert_eq!(text(result_of(&response), "status"), "running");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), number(&outer, "stop_id"));
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn breakpoint_hit_cancels_a_step() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "step-cancelled",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.outer_before.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let outer_id = number(&stopped, "stop_id");

        // A breakpoint installed while stopped takes precedence over the step
        // plan at the next checkpoint.
        let breakpoint = set_checkpoint(&mut client, setup.fixture.hooks.inner_second.id);
        assert!(boolean(&breakpoint, "bound"));
        assert_eq!(number(&breakpoint, "breakpoint_id"), 2);

        let response = request(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("over")),
                ("stop_id", Json::unsigned(outer_id)),
            ]),
        );
        assert_eq!(text(result_of(&response), "status"), "stepping");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), outer_id);
        let hit = next_stopped(&mut client);
        assert_eq!(text(&hit, "reason"), "breakpoint");
        assert_eq!(
            number(&hit, "checkpoint_id"),
            u64::from(setup.fixture.hooks.inner_second.id)
        );
        assert_eq!(number(&hit, "breakpoint_id"), 2);

        let response = request(
            &mut client,
            "exec.continue",
            stop_id_params(number(&hit, "stop_id")),
        );
        assert_eq!(text(result_of(&response), "status"), "running");
        let resumed = next_resumed(&mut client);
        assert_eq!(number(&resumed, "stop_id"), number(&hit, "stop_id"));
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn recursion_reports_nested_frames_and_optimized_callers() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "recursion",
        StopFlow::Recursive(8),
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let stop_id = number(&stopped, "stop_id");

        let response = request(&mut client, "stack.list", stop_id_params(stop_id));
        let stack = result_of(&response).clone();
        assert_eq!(
            warning_codes(&response),
            vec![codes::W_DEBUG_UNAVAILABLE.to_owned()],
            "unmapped callers must be reported as optimized out"
        );
        let frames = field(&stack, "frames").as_array().expect("frames");
        assert!(
            frames.len() >= 9,
            "expected eight recursion levels: {stack:?}"
        );
        assert!(!boolean(&stack, "truncated"));
        for (index, frame) in frames.iter().enumerate().take(8) {
            assert_eq!(number(frame, "index"), index as u64);
            assert_eq!(
                text(frame, "function"),
                setup.fixture.inner.to_hex(),
                "frame {index}"
            );
            assert_eq!(text(frame, "symbol"), setup.fixture.inner_symbol);
            assert!(!boolean(frame, "optimized_out"), "frame {index}");
        }
        let optimized = frames
            .iter()
            .position(|frame| boolean(frame, "optimized_out"))
            .expect("a caller outside the frame sidecar is optimized out");
        assert!(optimized >= 8, "optimized frame at {optimized}");
        assert!(field(&frames[optimized], "function").is_null());
        assert!(field(&frames[optimized], "symbol").is_null());
        assert!(field(&frames[optimized], "source").is_null());

        let local = ok(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(5, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
        );
        assert!(boolean(&local, "available"));
        assert_eq!(text(field(&local, "value"), "value"), "11");
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(0, "Z")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_UNKNOWN,
        );
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(optimized as u64, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_UNKNOWN,
        );
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(frames.len() as u64, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_UNKNOWN,
        );

        request(&mut client, "exec.continue", stop_id_params(stop_id));
        let _ = next_resumed(&mut client);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn stack_capture_is_truncated_at_1024_frames() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "recursion-cap",
        StopFlow::Recursive(1030),
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let stop_id = number(&stopped, "stop_id");

        let stack = ok(&mut client, "stack.list", stop_id_params(stop_id));
        let frames = field(&stack, "frames").as_array().expect("frames");
        assert_eq!(frames.len(), 1024);
        assert!(boolean(&stack, "truncated"));
        assert_eq!(text(&frames[0], "symbol"), setup.fixture.inner_symbol);
        assert_eq!(
            text(&frames[1023], "function"),
            setup.fixture.inner.to_hex()
        );
        assert!(!boolean(&frames[1023], "optimized_out"));
        let local = ok(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(1023, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
        );
        assert!(boolean(&local, "available"));
        assert_eq!(text(field(&local, "value"), "value"), "11");

        request(&mut client, "exec.continue", stop_id_params(stop_id));
        let _ = next_resumed(&mut client);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

fn assert_frame_data_unavailable(name: &str, sidecar: Sidecar) {
    let Some(mut setup) = StopSetup::new(name, StopFlow::Nested, Duration::from_secs(60), sidecar)
    else {
        return;
    };
    let mut client = setup.client();
    let response = session_start_response(&mut client);
    let started = result_of(&response).clone();
    let capabilities = field(&started, "capabilities");
    assert!(boolean(capabilities, "hooks"));
    assert!(boolean(capabilities, "checkpoints"));
    assert!(!boolean(capabilities, "stepping"));
    assert_eq!(
        warning_codes(&response),
        vec![codes::W_DEBUG_UNAVAILABLE.to_owned()]
    );

    // Breakpoints and stops still work without frame information.
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);
    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        assert_eq!(text(&stopped, "reason"), "breakpoint");
        let stop_id = number(&stopped, "stop_id");

        expect_error(
            &mut client,
            "stack.list",
            stop_id_params(stop_id),
            codes::DEBUG_INFO,
        );
        expect_error(
            &mut client,
            "frame.read",
            obj([
                ("target", local_target(0, "A")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_INFO,
        );
        expect_error(
            &mut client,
            "exec.step",
            obj([
                ("mode", Json::string("over")),
                ("stop_id", Json::unsigned(stop_id)),
            ]),
            codes::DEBUG_STOP,
        );
        request(&mut client, "exec.continue", stop_id_params(stop_id));
        let _ = next_resumed(&mut client);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn missing_frame_sidecar_disables_stack_reads() {
    let _guard = stop_guard();
    assert_frame_data_unavailable("sidecar-missing", Sidecar::Missing);
}

#[test]
fn malformed_frame_sidecar_disables_stack_reads() {
    let _guard = stop_guard();
    assert_frame_data_unavailable("sidecar-malformed", Sidecar::Malformed);
}

#[test]
fn mispaired_frame_sidecar_disables_stack_reads() {
    let _guard = stop_guard();
    assert_frame_data_unavailable("sidecar-mispaired", Sidecar::WrongBuildId);
}

#[test]
fn missing_metadata_reports_a_metadata_error_and_keeps_monitoring() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "metadata-missing",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::MissingMetadata,
    ) else {
        return;
    };
    let mut client = setup.client();
    let response = session_start_response(&mut client);
    let started = result_of(&response).clone();
    let capabilities = field(&started, "capabilities");
    assert!(!boolean(capabilities, "hooks"));
    assert!(!boolean(capabilities, "checkpoints"));
    assert!(!boolean(capabilities, "stepping"));
    let warnings = field(&response, "warnings").as_array().expect("warnings");
    assert_eq!(warnings.len(), 1);
    assert_eq!(text(&warnings[0], "code"), codes::W_DEBUG_MONITORING_ONLY);
    assert_eq!(
        text(field(&warnings[0], "detail"), "code"),
        codes::DEBUG_METADATA
    );

    // Breakpoint, execution, and frame operations report the metadata error.
    expect_error(
        &mut client,
        "breakpoint.set",
        obj([("target", check_target(setup.fixture.hooks.inner_first.id))]),
        codes::DEBUG_METADATA,
    );
    expect_error(&mut client, "exec.continue", obj([]), codes::DEBUG_METADATA);
    expect_error(
        &mut client,
        "stack.list",
        stop_id_params(1),
        codes::DEBUG_METADATA,
    );
    expect_error(
        &mut client,
        "frame.read",
        obj([
            ("target", local_target(0, "A")),
            ("stop_id", Json::unsigned(1)),
        ]),
        codes::DEBUG_METADATA,
    );

    // The module still runs; the hook is a no-op without an installed stop
    // session.
    let cycle = setup.host.cycle_at(1_000_000).expect("cycle");
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn checkpoint_mismatch_disables_breakpoints_for_the_session() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "mismatch",
        StopFlow::Mismatch,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    let breakpoint = set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);
    assert!(boolean(&breakpoint, "bound"));

    let cycle = setup.host.cycle_at(1_000_000).expect("cycle");
    assert_ran(cycle, setup.generation);

    // The mismatched hook clears breakpoints and disables control until the
    // session ends; the module itself completed normally.
    let status = session_status(&mut client);
    assert_eq!(text(&status, "state"), "active");
    assert_eq!(number(&status, "breakpoints"), 0);
    expect_error(
        &mut client,
        "breakpoint.set",
        obj([("target", check_target(setup.fixture.hooks.outer_before.id))]),
        codes::DEBUG_METADATA,
    );
    let listed = ok(&mut client, "breakpoint.list", obj([]));
    assert_eq!(
        field(&listed, "breakpoints")
            .as_array()
            .expect("breakpoints")
            .len(),
        0
    );
    setup.finish();
}

#[test]
fn debug_check_without_a_session_is_a_noop() {
    let _guard = stop_guard();
    let output = common::output_with_flags(
        common::STOP_SOURCE,
        "no-session",
        BuildFlags {
            debug_hooks: true,
            ..BuildFlags::default()
        },
    );
    let Some(fixture) = StopFixture::build("no-session", &output, StopFlow::Nested) else {
        return;
    };
    let mut host = in_memory_host();
    let candidate = Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path)
        .expect("candidate");
    let outcome = host.load(candidate, LoadMode::Auto).expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    let dir = TestDir::new("no-session");
    let mut config = DebugServerConfig::new(dir.endpoint(), TransportKind::Socket);
    config.session_timeout = Duration::from_secs(60);
    host.start_debug_server(&config).expect("debug server");

    // The module calls service 13 twice per cycle with no session installed;
    // both calls must return immediately and the cycle must complete.
    assert_ran(host.cycle_at(1_000_000).expect("cycle"), generation);
    assert_ran(host.cycle_at(2_000_000).expect("cycle"), generation);
    host.stop_debug_server();
}

#[test]
fn queued_write_is_held_until_continue() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "queued-write",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let counter = root_record(&setup.fixture.artifacts, "G_COUNTER");
    let got_id = counter.got_id;
    let type_key = counter.type_key.clone();
    assert_eq!(
        setup.host.root_storage(got_id).expect("counter storage"),
        &[0, 0, 0, 0]
    );

    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let stop_id = number(&stopped, "stop_id");

        let response = request(
            &mut client,
            "var.write",
            obj([
                ("target", path_target("G_COUNTER")),
                ("value", decimal_value(&type_key, "7")),
            ]),
        );
        let queued = result_of(&response).clone();
        assert_eq!(text(&queued, "status"), "queued");
        let status = wait_queued(&mut client, 1);
        assert_eq!(text(&status, "state"), "stopped");

        // The write is queued, not applied: the pinned read still sees the
        // pre-stop value and the outputs are not republished while parked.
        let read = ok(
            &mut client,
            "var.read",
            obj([("target", path_target("G_COUNTER"))]),
        );
        assert!(boolean(&read, "stopped"));
        assert_eq!(text(field(&read, "value"), "value"), "0");
        assert_eq!(number(&read, "cycle"), 1);

        request(&mut client, "exec.continue", stop_id_params(stop_id));
        let _ = next_resumed(&mut client);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    // The queued write is applied once at the cycle boundary, before outputs
    // publish.
    assert_eq!(
        setup.host.root_storage(got_id).expect("counter storage"),
        &7_i32.to_le_bytes()
    );
    setup.finish();
}

#[test]
fn session_stop_while_stopped_resumes_the_cycle() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "stop-session-stop",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        assert_eq!(number(&stopped, "stop_id"), 1);

        let response = request(&mut client, "session.stop", obj([]));
        assert!(boolean(result_of(&response), "ended"));
        let ended = next_session_ended(&mut client);
        assert_eq!(text(&ended, "reason"), "client");
        assert!(optional_text(&ended, "code").is_none());
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    setup.finish();
}

#[test]
fn target_exit_ends_an_active_session() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "target-exit",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    setup.host.debug_target_exited().expect("target exited");

    let ended = next_session_ended(&mut client);
    assert_eq!(text(&ended, "reason"), "target-exited");
    assert!(optional_text(&ended, "code").is_none());
    assert_eq!(number(&ended, "generation"), setup.generation);
    setup.finish();
}

#[test]
fn shutdown_ends_an_active_session() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "shutdown",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let mut client = setup.client();
    session_start(&mut client);
    setup.host.shutdown().expect("shutdown");

    let ended = next_session_ended(&mut client);
    assert_eq!(text(&ended, "reason"), "shutdown");
    assert!(optional_text(&ended, "code").is_none());
    setup.host.stop_debug_server();
}

#[test]
fn hook_free_build_is_monitoring_only() {
    let _guard = stop_guard();
    let artifacts = common::artifacts_with_flags(
        common::STOP_SOURCE,
        "release-no-hooks",
        BuildFlags {
            release: true,
            ..BuildFlags::default()
        },
    );
    let Some(fixture) = Fixture::build("release-no-hooks", artifacts, Behavior::default()) else {
        return;
    };
    let mut host = in_memory_host();
    let candidate = Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path)
        .expect("candidate");
    let outcome = host.load(candidate, LoadMode::Auto).expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    let dir = TestDir::new("release-no-hooks");
    let mut config = DebugServerConfig::new(dir.endpoint(), TransportKind::Socket);
    config.session_timeout = Duration::from_secs(60);
    host.start_debug_server(&config).expect("debug server");
    let mut client = DebugClient::connect_file(&dir.endpoint()).expect("client");

    let response = session_start_response(&mut client);
    let started = result_of(&response).clone();
    let capabilities = field(&started, "capabilities");
    assert!(!boolean(capabilities, "hooks"));
    assert!(!boolean(capabilities, "checkpoints"));
    assert!(!boolean(capabilities, "stepping"));
    let warnings = field(&response, "warnings").as_array().expect("warnings");
    assert_eq!(warnings.len(), 1);
    assert_eq!(text(&warnings[0], "code"), codes::W_DEBUG_MONITORING_ONLY);
    assert_eq!(
        text(field(&warnings[0], "detail"), "capability"),
        "checkpoints"
    );

    let response = expect_error(
        &mut client,
        "breakpoint.set",
        obj([("target", check_target(1))]),
        codes::DEBUG_NO_HOOKS,
    );
    assert_eq!(
        text(field(field(&response, "error"), "detail"), "warning"),
        codes::W_DEBUG_MONITORING_ONLY
    );
    expect_error(&mut client, "exec.pause", obj([]), codes::DEBUG_NO_HOOKS);
    expect_error(
        &mut client,
        "stack.list",
        stop_id_params(1),
        codes::DEBUG_INFO,
    );
    expect_error(
        &mut client,
        "frame.read",
        obj([
            ("target", local_target(0, "A")),
            ("stop_id", Json::unsigned(1)),
        ]),
        codes::DEBUG_INFO,
    );

    assert_ran(host.cycle_at(1_000_000).expect("cycle"), generation);
    host.stop_debug_server();
}

#[test]
fn audit_log_records_stop_lifecycle_in_order() {
    let _guard = stop_guard();
    let Some(mut setup) = StopSetup::new(
        "audit",
        StopFlow::Nested,
        Duration::from_secs(60),
        Sidecar::Valid,
    ) else {
        return;
    };
    let audit_log = setup.info.audit_log.clone();
    let mut client = setup.client();
    session_start(&mut client);
    set_checkpoint(&mut client, setup.fixture.hooks.inner_first.id);

    let cycle = std::thread::scope(|scope| {
        let host = &mut setup.host;
        let handle = scope.spawn(move || host.cycle_at(1_000_000));
        let stopped = next_stopped(&mut client);
        let stop_id = number(&stopped, "stop_id");
        request(&mut client, "exec.continue", stop_id_params(stop_id));
        let _ = next_resumed(&mut client);
        handle.join().expect("cycle thread").expect("cycle")
    });
    assert_ran(cycle, setup.generation);
    // Stop the server so the audit sink is closed, but keep the fixture
    // directory alive until the log has been read.
    setup.host.stop_debug_server();

    let contents = fs::read_to_string(&audit_log).expect("audit log");
    let mut names = Vec::new();
    let mut last_seq = 0_u64;
    for line in contents.lines().filter(|line| !line.is_empty()) {
        let event = stc_got::json::parse(line).expect("audit record");
        let seq = number(&event, "seq");
        assert!(seq > last_seq, "audit sequence must increase");
        last_seq = seq;
        names.push(text(&event, "event").to_owned());
    }
    let position = |name: &str| {
        names
            .iter()
            .position(|event| event == name)
            .unwrap_or_else(|| panic!("audit event `{name}` in {names:?}"))
    };
    let ordered = [
        "server-start",
        "session-start",
        "breakpoint-set",
        "stop",
        "resume",
        "server-stop",
    ];
    let mut previous = 0;
    for name in ordered {
        let index = position(name);
        assert!(
            index >= previous,
            "audit `{name}` out of order in {names:?}"
        );
        previous = index;
    }
}
