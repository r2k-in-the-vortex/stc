//! M6.5 phase-5 `stc-debug/1` protocol tests.
//!
//! The tests compile real modules, start the local debug server on a real
//! transport, and drive it through the public [`DebugClient`]. Host commands
//! are serviced by pumping the cycle thread (`Host::debug_pump_at`) while the
//! client request is outstanding; the protocol server never touches storage.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use stc_got::codes;
use stc_got::json::{self, Json};
use stc_runtime::{
    Candidate, DEBUG_PROTOCOL, DebugClient, DebugClientError, DebugEndpointFile, DebugServerConfig,
    DebugServerInfo, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore, PROFILE,
    TransportKind,
};

use common::{Behavior, BuildFlags, Fixture};

const SOURCE: &str = r#"
TYPE
  Point : STRUCT
    x : DINT;
    y : BOOL;
  END_STRUCT;
END_TYPE

VAR_GLOBAL RETAIN
  G_RETAINED : DINT;
END_VAR

VAR_GLOBAL
  G_DINT : DINT;
  G_BOOL : BOOL;
  G_ARRAY : ARRAY[1..3] OF DINT;
  G_BIG : ARRAY[1..1024] OF DINT;
  G_POINT : Point;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

static COUNTER: AtomicU64 = AtomicU64::new(0);

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

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn cold_install(host: &mut Host, fixture: &Fixture) -> u64 {
    let outcome = host
        .load(open(fixture), LoadMode::Auto)
        .expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    generation
}

fn root<'a>(
    artifacts: &'a stc_codegen::module::ModuleArtifacts,
    key: &str,
) -> &'a stc_got::RootRecord {
    match artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key == key || root.key.ends_with(&format!("::{key}")))
    {
        Some(root) => root,
        None => panic!("root `{key}`"),
    }
}

fn root_type_key(artifacts: &stc_codegen::module::ModuleArtifacts, key: &str) -> String {
    root(artifacts, key).type_key.clone()
}

fn put(host: &mut Host, artifacts: &stc_codegen::module::ModuleArtifacts, key: &str, bytes: &[u8]) {
    let root = root(artifacts, key);
    let storage = host.root_storage_mut(root.got_id).expect("root storage");
    assert_eq!(storage.len(), bytes.len(), "storage size for `{key}`");
    storage.copy_from_slice(bytes);
}

/// A unique private directory for one test's endpoint and audit files.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stc-debug-protocol-{}-{id}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test directory");
        Self { path }
    }

    fn endpoint(&self) -> PathBuf {
        self.path.join("debug.endpoint.json")
    }

    fn named(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn start(
    host: &mut Host,
    config_path: PathBuf,
    transport: TransportKind,
    allow_write: bool,
    allow_force: bool,
    timeout: Duration,
) -> DebugServerInfo {
    let mut config = DebugServerConfig::new(config_path, transport);
    config.allow_write = allow_write;
    config.allow_force = allow_force;
    config.session_timeout = timeout;
    host.start_debug_server(&config).expect("debug server")
}

struct Setup {
    host: Host,
    dir: TestDir,
    info: DebugServerInfo,
    artifacts: stc_codegen::module::ModuleArtifacts,
    fixture: Fixture,
}

impl Setup {
    fn new(name: &str, allow_write: bool, allow_force: bool, timeout: Duration) -> Option<Self> {
        Self::new_with(
            name,
            TransportKind::Socket,
            allow_write,
            allow_force,
            timeout,
        )
    }

    fn new_with(
        name: &str,
        transport: TransportKind,
        allow_write: bool,
        allow_force: bool,
        timeout: Duration,
    ) -> Option<Self> {
        let artifacts = common::artifacts(SOURCE, name);
        let fixture = Fixture::build(name, artifacts.clone(), Behavior::default())?;
        let mut host = in_memory_host();
        cold_install(&mut host, &fixture);
        let dir = TestDir::new(name);
        let info = start(
            &mut host,
            dir.endpoint(),
            transport,
            allow_write,
            allow_force,
            timeout,
        );
        Some(Self {
            host,
            dir,
            info,
            artifacts,
            fixture,
        })
    }

    fn client(&self) -> DebugClient {
        DebugClient::connect_file(&self.dir.endpoint()).expect("client")
    }

    fn connect_known(&self, token: &str) -> DebugClient {
        let endpoint = self.endpoint_file();
        DebugClient::connect_known(&endpoint.endpoint, token, endpoint.transport).expect("connect")
    }

    fn endpoint_file(&self) -> DebugEndpointFile {
        DebugEndpointFile::read(&self.dir.endpoint()).expect("endpoint file")
    }

    fn audit(&self) -> Vec<Json> {
        audit_events(&self.info.audit_log)
    }

    fn dint_type(&self) -> String {
        root_type_key(&self.artifacts, "G_DINT")
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
    match field(value, key).as_str() {
        Some(text) => text,
        None => panic!("string `{key}` in {}", value.render_compact()),
    }
}

fn number(value: &Json, key: &str) -> u64 {
    match field(value, key)
        .as_number()
        .and_then(|value| u64::try_from(value).ok())
    {
        Some(number) => number,
        None => panic!("number `{key}` in {}", value.render_compact()),
    }
}

fn boolean(value: &Json, key: &str) -> bool {
    match field(value, key).as_bool() {
        Some(boolean) => boolean,
        None => panic!("bool `{key}` in {}", value.render_compact()),
    }
}

fn message_kind(message: &Json) -> Option<&str> {
    message
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(Json::as_str)
}

fn target_path(name: &str) -> Json {
    obj([("kind", Json::string("path")), ("key", Json::string(name))])
}

fn decimal_value(type_key: &str, value: &str) -> Json {
    obj([
        ("type", Json::string(type_key)),
        ("value", Json::string(value)),
    ])
}

fn request_frame(id: u64, token: &str, op: &str, params: Json) -> String {
    format!(
        "{{\"kind\":\"request\",\"protocol\":\"{DEBUG_PROTOCOL}\",\"id\":{id},\"token\":\"{token}\",\"op\":\"{op}\",\"params\":{}}}",
        params.render_compact()
    )
}

fn response_code(response: &Json) -> &str {
    text(field(response, "error"), "code")
}

fn response_result(response: &Json) -> &Json {
    assert_eq!(
        text(response, "status"),
        "ok",
        "{}",
        response.render_compact()
    );
    field(response, "result")
}

fn response_warning_codes(response: &Json) -> Vec<String> {
    field(response, "warnings")
        .as_array()
        .expect("warnings")
        .iter()
        .map(|warning| text(warning, "code").to_owned())
        .collect()
}

fn event_name(event: &Json) -> &str {
    text(event, "event")
}

fn event_data(event: &Json) -> &Json {
    field(event, "data")
}

fn session_ended(event: &Json) -> (&str, Option<&str>, u64, String) {
    assert_eq!(message_kind(event), Some("event"));
    assert_eq!(event_name(event), "session-ended");
    let data = event_data(event);
    let code = data
        .as_object()
        .and_then(|object| object.get("code"))
        .and_then(Json::as_str);
    (
        text(data, "reason"),
        code,
        number(data, "generation"),
        text(data, "build_id").to_owned(),
    )
}

fn send_frame(client: &mut DebugClient, id: u64, token: &str, op: &str, params: Json) -> Json {
    let frame = request_frame(id, token, op, params);
    client
        .write_frame(frame.as_bytes())
        .expect("write request frame");
    client
        .read_message()
        .expect("read response")
        .expect("response before close")
}

fn send_text(client: &mut DebugClient, text: &str) -> Json {
    client
        .write_frame(text.as_bytes())
        .expect("write raw frame");
    client
        .read_message()
        .expect("read response")
        .expect("response before close")
}

fn send_stream_frame(stream: &mut TcpStream, id: u64, token: &str, op: &str, params: Json) -> bool {
    let mut frame = request_frame(id, token, op, params).into_bytes();
    frame.push(b'\n');
    stream.write_all(&frame).is_ok()
}

fn read_stream_message(reader: &mut BufReader<TcpStream>) -> Option<Json> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => {
            let line = line.strip_suffix('\n').unwrap_or(&line);
            Some(json::parse_protocol(line).expect("server frame"))
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
            ) =>
        {
            None
        }
        Err(error) => panic!("rate transport read: {error}"),
    }
}

fn session_start_params(scopes: &[&str]) -> Json {
    obj([
        (
            "client",
            obj([
                ("name", Json::string("stc-debug-test")),
                ("version", Json::string("1")),
            ]),
        ),
        (
            "scopes",
            Json::Array(scopes.iter().map(|scope| Json::string(*scope)).collect()),
        ),
    ])
}

fn session_start_response(client: &mut DebugClient, scopes: &[&str]) -> Json {
    let response = client
        .call("session.start", session_start_params(scopes))
        .expect("session.start response");
    assert_eq!(
        text(&response, "status"),
        "ok",
        "{}",
        response.render_compact()
    );
    response
}

fn start_session(client: &mut DebugClient, scopes: &[&str]) -> Json {
    response_result(&session_start_response(client, scopes)).clone()
}

fn find_session_ended(messages: &[Json]) -> Json {
    match messages.iter().find(|message| {
        message_kind(message) == Some("event")
            && message
                .as_object()
                .and_then(|object| object.get("event"))
                .and_then(Json::as_str)
                == Some("session-ended")
    }) {
        Some(event) => event.clone(),
        None => {
            let names: Vec<_> = messages
                .iter()
                .filter_map(|message| message.as_object())
                .filter_map(|object| object.get("event"))
                .filter_map(Json::as_str)
                .collect();
            panic!("no session-ended event; got {names:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Host pumping helpers
// ---------------------------------------------------------------------------

fn pump_until_finished<T>(
    host: &mut Host,
    handle: &std::thread::ScopedJoinHandle<'_, T>,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "`{what}` did not finish in time");
        host.debug_pump_at(0);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn call_ok_pumped(
    host: &mut Host,
    client: &mut DebugClient,
    op: &str,
    params: Json,
) -> std::result::Result<Json, DebugClientError> {
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| client.call_ok(op, params));
        pump_until_finished(host, &handle, op);
        handle.join().expect("client request thread")
    })
}

fn next_event_pumped(host: &mut Host, client: &mut DebugClient) -> Json {
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| client.next_event());
        pump_until_finished(host, &handle, "event");
        handle.join().expect("event thread").expect("event")
    })
}

/// Waits for `session-ended`, discarding watch batches already in flight.
///
/// A watch is due at its creation pump, so one batch may be queued before a
/// later teardown event.
fn next_session_ended_pumped(host: &mut Host, client: &mut DebugClient) -> Json {
    loop {
        let event = next_event_pumped(host, client);
        if event_name(&event) == "session-ended" {
            return event;
        }
        assert_eq!(event_name(&event), "watch", "{}", event.render_compact());
    }
}

/// Reads messages until the server closes while the host services any teardown
/// command.
fn drain_until_eof(host: &mut Host, client: &mut DebugClient) -> Vec<Json> {
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            let mut messages = Vec::new();
            loop {
                match client.read_message() {
                    Ok(Some(message)) => messages.push(message),
                    Ok(None) => break,
                    Err(error) => panic!("read while draining: {error}"),
                }
            }
            messages
        });
        pump_until_finished(host, &handle, "eof");
        handle.join().expect("drain thread")
    })
}

fn stop_session(host: &mut Host, client: &mut DebugClient) -> Json {
    let result = call_ok_pumped(host, client, "session.stop", obj([])).expect("session.stop");
    assert!(boolean(&result, "ended"));
    let mut messages = client.take_events();
    messages.extend(drain_until_eof(host, client));
    find_session_ended(&messages)
}

fn settle(host: &mut Host, millis: u64) {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        host.debug_pump_at(0);
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------
// Audit helpers
// ---------------------------------------------------------------------------

fn audit_events(path: &Path) -> Vec<Json> {
    let text = fs::read_to_string(path).expect("audit log");
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| match json::parse(line) {
            Ok(value) => value,
            Err(error) => panic!("audit line `{line}`: {error:?}"),
        })
        .collect()
}

fn audit_names(path: &Path) -> Vec<String> {
    audit_events(path)
        .iter()
        .map(|event| text(event, "event").to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Endpoint file and audit lifecycle
// ---------------------------------------------------------------------------

#[test]
fn endpoint_file_and_audit_log_describe_the_server_privately() {
    let Some(mut setup) = Setup::new("endpoint-file", false, false, Duration::ZERO) else {
        return;
    };
    let generation = setup.info.generation;
    let build_id = setup.info.build_id.to_hex();
    let endpoint = setup.endpoint_file();
    assert_eq!(endpoint.endpoint, setup.info.endpoint);
    assert_eq!(endpoint.protocol, DEBUG_PROTOCOL);
    assert_eq!(endpoint.transport, TransportKind::Socket);
    assert_eq!(endpoint.build_id, build_id);
    assert_eq!(endpoint.generation, generation);
    assert_eq!(endpoint.mode, "debug");
    assert_eq!(
        endpoint.audit_log,
        setup.info.audit_log.to_string_lossy().into_owned()
    );
    assert_eq!(endpoint.token.len(), 64);
    assert!(
        endpoint
            .token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "the token must be lowercase hex"
    );
    assert!(endpoint.token != "0".repeat(64));

    let raw = fs::read_to_string(setup.dir.endpoint()).expect("endpoint raw");
    assert!(raw.contains("\"format\":1"));
    let mut previous = 0;
    for key in [
        "audit_log",
        "build_id",
        "endpoint",
        "format",
        "generation",
        "mode",
        "pid",
        "protocol",
        "token",
        "transport",
    ] {
        let position = match raw.find(&format!("\"{key}\":")) {
            Some(position) => position,
            None => panic!("endpoint key `{key}`"),
        };
        assert!(position >= previous, "endpoint keys must be sorted");
        previous = position;
    }
    assert!(raw.contains(&format!("\"pid\":{}", std::process::id())));

    let audit_raw = fs::read_to_string(&setup.info.audit_log).expect("audit raw");
    assert!(
        !audit_raw.contains(&endpoint.token),
        "the token must not be audited"
    );
    let directory = setup.dir.path.to_string_lossy().into_owned();
    assert!(
        !audit_raw.contains(&directory),
        "the audit log must not contain an absolute host path"
    );
    let events = setup.audit();
    assert_eq!(text(&events[0], "event"), "server-start");
    assert_eq!(number(&events[0], "format"), 1);
    assert_eq!(number(&events[0], "seq"), 1);
    assert_eq!(number(&events[0], "generation"), generation);
    assert_eq!(text(&events[0], "build_id"), build_id.as_str());
    assert!(number(&events[0], "monotonic_ns") > 0);
    assert_eq!(text(field(&events[0], "detail"), "transport"), "socket");
    assert!(field(&events[0], "session").is_null());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&setup.info.audit_log), 0o600);
        assert_eq!(mode(&setup.dir.endpoint()), 0o600);
    }

    setup.host.stop_debug_server();
    assert!(
        !setup.dir.endpoint().exists(),
        "the endpoint file is deleted"
    );
    assert!(setup.info.audit_log.exists(), "the audit log is kept");
    let events = setup.audit();
    let names: Vec<_> = events.iter().map(|event| text(event, "event")).collect();
    assert_eq!(names.last().copied(), Some("server-stop"));
    for (index, event) in events.iter().enumerate() {
        assert_eq!(number(event, "format"), 1);
        assert_eq!(
            number(event, "seq"),
            u64::try_from(index).expect("index") + 1,
            "audit sequence starts at 1 and has no gaps"
        );
        assert!(event.as_object().is_some());
    }
}

#[cfg(windows)]
#[test]
fn pipe_transport_is_a_local_named_pipe() {
    let Some(mut setup) = Setup::new_with(
        "pipe-windows",
        TransportKind::Pipe,
        false,
        false,
        Duration::ZERO,
    ) else {
        return;
    };
    let endpoint = setup.endpoint_file();
    assert_eq!(endpoint.transport, TransportKind::Pipe);
    assert!(endpoint.endpoint.starts_with(r"\\.\pipe\"));
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let status =
        call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([])).expect("status");
    assert_eq!(text(&status, "state"), "active");
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[cfg(unix)]
#[test]
fn pipe_transport_is_a_user_private_unix_socket() {
    use std::os::unix::fs::PermissionsExt;

    let Some(mut setup) = Setup::new_with(
        "pipe-unix",
        TransportKind::Pipe,
        false,
        false,
        Duration::ZERO,
    ) else {
        return;
    };
    let endpoint = setup.endpoint_file();
    assert_eq!(endpoint.transport, TransportKind::Pipe);
    let socket = PathBuf::from(&endpoint.endpoint);
    let directory = socket.parent().expect("socket directory").to_path_buf();
    let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode(&directory), 0o700, "socket directory is user-private");
    assert_eq!(mode(&socket), 0o600, "socket is user-only");
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
    assert!(!socket.exists(), "the Unix socket is removed");
    assert!(
        !directory.exists(),
        "the private socket directory is removed"
    );
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[test]
fn authentication_requires_the_exact_endpoint_token() {
    let Some(mut setup) = Setup::new("auth", false, false, Duration::ZERO) else {
        return;
    };
    let endpoint = setup.endpoint_file();

    let mut wrong = setup.connect_known(&"0".repeat(64));
    let response = wrong.call("session.status", obj([])).expect("response");
    assert_eq!(response_code(&response), codes::DEBUG_AUTH);
    assert!(
        matches!(wrong.read_message(), Ok(None)),
        "auth failure closes"
    );

    let mut upper = setup.connect_known(&endpoint.token.to_uppercase());
    let response = upper.call("session.status", obj([])).expect("response");
    assert_eq!(response_code(&response), codes::DEBUG_AUTH);

    let mut short = setup.connect_known(&endpoint.token[..32]);
    let response = short.call("session.status", obj([])).expect("response");
    assert_eq!(response_code(&response), codes::DEBUG_AUTH);

    // A request without a token is also an authentication failure.
    let mut bare = setup.connect_known(&endpoint.token);
    bare.write_frame(
        format!(
            "{{\"kind\":\"request\",\"protocol\":\"{DEBUG_PROTOCOL}\",\"id\":1,\"op\":\"session.status\",\"params\":{{}}}}"
        )
        .as_bytes(),
    )
    .expect("frame");
    let response = bare.read_message().expect("read").expect("response");
    assert_eq!(response_code(&response), codes::DEBUG_AUTH);
    assert!(matches!(bare.read_message(), Ok(None)));

    // The exact token succeeds, and neither the token nor a host path is
    // written to the audit log or echoed in a response.
    let mut client = setup.client();
    let result = start_session(&mut client, &["observe"]);
    assert!(text(&result, "session").starts_with("s1-"));
    let events = setup.audit();
    let failures = events
        .iter()
        .filter(|event| text(event, "event") == "auth-fail")
        .count();
    assert_eq!(failures, 4);
    let audit_raw = fs::read_to_string(&setup.info.audit_log).expect("audit raw");
    assert!(!audit_raw.contains(&endpoint.token));
    assert!(!response.render_compact().contains(&endpoint.token));
    let ended = stop_session(&mut setup.host, &mut client);
    assert_eq!(session_ended(&ended).0, "client");
    setup.host.stop_debug_server();
}

#[test]
fn authentication_failure_limits_refuse_new_connections() {
    let Some(mut setup) = Setup::new("auth-limit", false, false, Duration::ZERO) else {
        return;
    };
    let wrong = "f".repeat(64);
    for _ in 0..20 {
        let mut client = setup.connect_known(&wrong);
        let response = client.call("session.status", obj([])).expect("response");
        assert_eq!(response_code(&response), codes::DEBUG_AUTH);
        assert!(matches!(client.read_message(), Ok(None)));
    }

    // The endpoint now refuses even the correct token for 60 seconds.
    let endpoint = setup.endpoint_file();
    let mut client = setup.connect_known(&endpoint.token);
    if let Ok(response) = client.call("session.status", obj([])) {
        panic!(
            "the endpoint must refuse new connections: {}",
            response.render_compact()
        );
    }
    let events = setup.audit();
    assert_eq!(
        events
            .iter()
            .filter(|event| text(event, "event") == "auth-fail")
            .count(),
        20
    );
    assert!(events.iter().any(|event| {
        text(event, "event") == "transport-error"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("reason"))
                .and_then(Json::as_str)
                == Some("auth-refused")
    }));
    setup.host.stop_debug_server();
}

// ---------------------------------------------------------------------------
// Framing and protocol negotiation
// ---------------------------------------------------------------------------

#[test]
fn protocol_version_negotiation_retries_once() {
    let Some(mut setup) = Setup::new("protocol-version", false, false, Duration::ZERO) else {
        return;
    };
    let token = setup.endpoint_file().token;
    let mut client = setup.client();
    let wrong = request_frame(1, &token, "session.start", session_start_params(&[]))
        .replace(DEBUG_PROTOCOL, "stc-debug/2");
    let response = send_text(&mut client, &wrong);
    assert_eq!(response_code(&response), codes::DEBUG_PROTOCOL);
    let detail = field(field(&response, "error"), "detail");
    assert_eq!(
        field(detail, "supported").as_array().expect("supported")[0].as_str(),
        Some(DEBUG_PROTOCOL)
    );

    // The one permitted retry with the exact version succeeds.
    let response = send_frame(
        &mut client,
        2,
        &token,
        "session.start",
        session_start_params(&["observe"]),
    );
    assert_eq!(text(&response, "status"), "ok");

    // A later wrong version closes the connection.
    let wrong =
        request_frame(3, &token, "session.status", obj([])).replace(DEBUG_PROTOCOL, "stc-debug/0");
    let response = send_text(&mut client, &wrong);
    assert_eq!(response_code(&response), codes::DEBUG_PROTOCOL);
    let messages = drain_until_eof(&mut setup.host, &mut client);
    let ended = find_session_ended(&messages);
    assert_eq!(session_ended(&ended).0, "protocol-error");
    setup.host.stop_debug_server();
}

#[test]
fn malformed_frames_close_after_three() {
    let Some(mut setup) = Setup::new("malformed", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    for frame in ["{", "\u{feff}{}", ""] {
        let response = send_text(&mut client, frame);
        assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
        assert_eq!(number(&response, "id"), 0);
    }
    assert!(matches!(client.read_message(), Ok(None)));
    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "disconnect"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("reason"))
                .and_then(Json::as_str)
                == Some("protocol-error")
    }));
    setup.host.stop_debug_server();
}

#[test]
fn request_shape_ids_and_raw_addresses_are_validated() {
    let Some(mut setup) = Setup::new("request-shape", false, false, Duration::ZERO) else {
        return;
    };
    let token = setup.endpoint_file().token;

    // Unknown request key and unknown kind; the connection remains usable,
    // and a duplicate id is the third malformed frame.
    let mut client = setup.client();
    let unknown_key = obj([
        ("kind", Json::string("request")),
        ("protocol", Json::string(DEBUG_PROTOCOL)),
        ("id", Json::unsigned(1)),
        ("token", Json::string(token.clone())),
        ("op", Json::string("session.status")),
        ("params", obj([])),
        ("extra", Json::Bool(true)),
    ]);
    let response = send_text(&mut client, &unknown_key.render_compact());
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let unknown_kind = obj([
        ("kind", Json::string("response")),
        ("protocol", Json::string(DEBUG_PROTOCOL)),
        ("id", Json::unsigned(2)),
        ("token", Json::string(token.clone())),
        ("op", Json::string("session.status")),
        ("params", obj([])),
    ]);
    let response = send_text(&mut client, &unknown_kind.render_compact());
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let response = send_frame(
        &mut client,
        3,
        &token,
        "session.start",
        session_start_params(&["observe"]),
    );
    assert_eq!(text(&response, "status"), "ok");
    let duplicate = request_frame(3, &token, "session.status", obj([]));
    let response = send_text(&mut client, &duplicate);
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let messages = drain_until_eof(&mut setup.host, &mut client);
    assert_eq!(
        session_ended(&find_session_ended(&messages)).0,
        "protocol-error"
    );

    // Unknown operation and missing op; a valid session still works and stops.
    let mut client = setup.client();
    let response = send_frame(&mut client, 1, &token, "var.poke", obj([]));
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let missing_op = obj([
        ("kind", Json::string("request")),
        ("protocol", Json::string(DEBUG_PROTOCOL)),
        ("id", Json::unsigned(2)),
        ("token", Json::string(token.clone())),
        ("params", obj([])),
    ]);
    let response = send_text(&mut client, &missing_op.render_compact());
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let response = send_frame(
        &mut client,
        3,
        &token,
        "session.start",
        session_start_params(&["observe"]),
    );
    assert_eq!(text(&response, "status"), "ok");
    let response = send_frame(&mut client, 4, &token, "session.stop", obj([]));
    assert!(boolean(response_result(&response), "ended"));
    let messages = drain_until_eof(&mut setup.host, &mut client);
    assert_eq!(session_ended(&find_session_ended(&messages)).0, "client");

    // Missing params and an out-of-range id do not close a fresh connection.
    let mut client = setup.client();
    let missing_params = obj([
        ("kind", Json::string("request")),
        ("protocol", Json::string(DEBUG_PROTOCOL)),
        ("id", Json::unsigned(1)),
        ("token", Json::string(token.clone())),
        ("op", Json::string("session.status")),
    ]);
    let response = send_text(&mut client, &missing_params.render_compact());
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let out_of_range = request_frame(0, &token, "session.status", obj([]));
    let response = send_text(&mut client, &out_of_range);
    assert_eq!(response_code(&response), codes::DEBUG_REQUEST);
    let response = send_frame(
        &mut client,
        2,
        &token,
        "session.start",
        session_start_params(&["observe"]),
    );
    assert_eq!(text(&response, "status"), "ok");
    let response = send_frame(&mut client, 3, &token, "session.stop", obj([]));
    assert!(boolean(response_result(&response), "ended"));
    let messages = drain_until_eof(&mut setup.host, &mut client);
    assert_eq!(session_ended(&find_session_ended(&messages)).0, "client");

    // Raw addresses are never accepted as targets.
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let read = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("0x1234"))]),
    )
    .expect_err("raw address");
    assert_eq!(read.code.as_deref(), Some(codes::DEBUG_UNKNOWN));
    let address_key = obj([(
        "target",
        obj([
            ("kind", Json::string("path")),
            ("key", Json::string("G_DINT")),
            ("address", Json::unsigned(0x1234)),
        ]),
    )]);
    let read = call_ok_pumped(&mut setup.host, &mut client, "var.read", address_key)
        .expect_err("address field");
    assert_eq!(read.code.as_deref(), Some(codes::DEBUG_REQUEST));
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[test]
fn oversize_request_frame_is_e004_and_closes() {
    let Some(mut setup) = Setup::new("oversize", false, false, Duration::ZERO) else {
        return;
    };
    let endpoint = setup.endpoint_file();
    assert_eq!(endpoint.transport, TransportKind::Socket);
    let stream = std::net::TcpStream::connect(&endpoint.endpoint).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut writer = stream.try_clone().expect("clone");
    let response = std::thread::scope(|scope| {
        // Write an unterminated frame past the 1 MiB limit while the response
        // is read concurrently, so the error is observed even if the server
        // closes with unread input.
        let handle = scope.spawn(move || {
            let chunk = vec![b'{'; 16 * 1024];
            for _ in 0..65 {
                if std::io::Write::write_all(&mut writer, &chunk).is_err() {
                    return;
                }
            }
            let _ = std::io::Write::write_all(&mut writer, b"\n");
        });
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("error response line");
        let response = json::parse(&line).expect("error response JSON");
        handle.join().expect("writer thread");
        response
    });
    assert_eq!(response_code(&response), codes::DEBUG_FRAME);
    assert_eq!(number(&response, "id"), 0);
    let names = audit_names(&setup.info.audit_log);
    assert!(names.iter().any(|name| name == "transport-error"));
    setup.host.stop_debug_server();
}

#[test]
fn first_request_deadline_closes_the_connection() {
    let Some(mut setup) = Setup::new("first-request", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let started = Instant::now();
    assert!(matches!(client.read_message(), Ok(None)));
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "the first-request deadline is five seconds"
    );
    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "timeout"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("phase"))
                .and_then(Json::as_str)
                == Some("first-request")
    }));
    setup.host.stop_debug_server();
}

#[test]
fn request_processing_timeout_returns_e013_and_keeps_the_session() {
    let Some(mut setup) = Setup::new("request-timeout", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    // No pump: the typed read cannot complete within its processing deadline.
    let error = client
        .call_ok("var.read", obj([("target", target_path("G_DINT"))]))
        .expect_err("the read must time out");
    assert_eq!(error.code.as_deref(), Some(codes::DEBUG_TIMEOUT));
    // Drain the stale command now that the cycle thread pumps again.
    settle(&mut setup.host, 50);
    // The session and connection remain usable.
    let status = call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([]))
        .expect("status after timeout");
    assert_eq!(text(&status, "state"), "active");
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

// ---------------------------------------------------------------------------
// Authorization, scopes, and unavailable phase-6 operations
// ---------------------------------------------------------------------------

#[test]
fn scope_authorization_matrix_denies_every_ungranted_operation() {
    let Some(mut setup) = Setup::new("scopes", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let result = start_session(&mut client, &["observe"]);
    let capabilities = field(&result, "capabilities");
    assert_eq!(
        field(capabilities, "scopes")
            .as_array()
            .expect("scopes")
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>(),
        vec!["observe"]
    );

    let dint = setup.dint_type();
    // Observe operations are granted.
    let read = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("G_DINT"))]),
    )
    .expect("observe read");
    assert_eq!(text(field(&read, "value"), "type"), dint.as_str());
    let watches =
        call_ok_pumped(&mut setup.host, &mut client, "watch.list", obj([])).expect("watch.list");
    assert!(
        field(&watches, "watches")
            .as_array()
            .expect("watches")
            .is_empty()
    );
    let added = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            ("targets", Json::Array(vec![target_path("G_DINT")])),
            ("interval_ms", Json::unsigned(50)),
        ]),
    )
    .expect("watch.add");
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.remove",
        obj([("watch_id", Json::unsigned(number(&added, "watch_id")))]),
    )
    .expect("watch.remove");
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "manifest.get",
        obj([("format", Json::string("json"))]),
    )
    .expect("manifest.get");
    let status =
        call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([])).expect("status");
    assert_eq!(text(&status, "state"), "active");

    // Phase-6 observe operations are documented as unavailable.
    let stack = call_ok_pumped(&mut setup.host, &mut client, "stack.list", obj([]))
        .expect_err("stack unavailable");
    assert_eq!(stack.code.as_deref(), Some(codes::DEBUG_INFO));
    let frame = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "frame.read",
        obj([(
            "target",
            obj([
                ("kind", Json::string("local")),
                ("frame", Json::unsigned(0)),
                ("name", Json::string("x")),
            ]),
        )]),
    )
    .expect_err("frame unavailable");
    assert_eq!(frame.code.as_deref(), Some(codes::DEBUG_INFO));

    // Every control/write/force operation is denied by scope.
    let denied: [(&str, Json); 10] = [
        ("breakpoint.set", obj([("target", target_path("G_DINT"))])),
        (
            "breakpoint.clear",
            obj([("breakpoint_id", Json::unsigned(1))]),
        ),
        ("breakpoint.list", obj([])),
        ("exec.continue", obj([])),
        ("exec.step", obj([("mode", Json::string("over"))])),
        ("exec.pause", obj([])),
        (
            "var.write",
            obj([
                ("target", target_path("G_DINT")),
                ("value", decimal_value(&dint, "1")),
            ]),
        ),
        (
            "var.force",
            obj([
                ("target", target_path("G_DINT")),
                ("value", decimal_value(&dint, "1")),
            ]),
        ),
        ("var.release", obj([("target", target_path("G_DINT"))])),
        ("var.release-all", obj([])),
    ];
    for (op, params) in denied {
        let error = call_ok_pumped(&mut setup.host, &mut client, op, params)
            .expect_err("operation must be denied");
        assert_eq!(
            error.code.as_deref(),
            Some(codes::DEBUG_AUTHORIZATION),
            "operation `{op}`"
        );
    }
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[test]
fn host_allow_flags_gate_write_and_force_and_phase6_ops_are_unavailable() {
    let Some(mut setup) = Setup::new("allow-flags", true, true, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let result = start_session(&mut client, &["observe", "control", "write", "force"]);
    let scopes: Vec<_> = field(field(&result, "capabilities"), "scopes")
        .as_array()
        .expect("scopes")
        .iter()
        .filter_map(Json::as_str)
        .collect();
    assert_eq!(scopes, vec!["observe", "control", "write", "force"]);

    let dint = setup.dint_type();
    for (op, params) in [
        ("breakpoint.set", obj([("target", target_path("G_DINT"))])),
        (
            "breakpoint.clear",
            obj([("breakpoint_id", Json::unsigned(1))]),
        ),
        ("breakpoint.list", obj([])),
        ("exec.continue", obj([])),
        ("exec.step", obj([("mode", Json::string("over"))])),
        ("exec.pause", obj([])),
    ] {
        let error = call_ok_pumped(&mut setup.host, &mut client, op, params)
            .expect_err("phase-6 operation unavailable");
        assert_eq!(error.code.as_deref(), Some(codes::DEBUG_NO_HOOKS), "{op}");
    }
    let written = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("G_DINT")),
            ("value", decimal_value(&dint, "3")),
        ]),
    )
    .expect("write granted");
    assert_eq!(text(&written, "status"), "queued");
    let forced = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.force",
        obj([
            ("target", target_path("G_DINT")),
            ("value", decimal_value(&dint, "4")),
        ]),
    )
    .expect("force granted");
    assert_eq!(text(&forced, "status"), "queued");
    setup.host.cycle_at(1_000_000).expect("cycle");
    let _ = next_event_pumped(&mut setup.host, &mut client);
    let _ = next_event_pumped(&mut setup.host, &mut client);
    let released = call_ok_pumped(&mut setup.host, &mut client, "var.release-all", obj([]))
        .expect("release-all granted");
    assert_eq!(number(&released, "released"), 1);
    let ended = stop_session(&mut setup.host, &mut client);
    assert_eq!(session_ended(&ended).0, "client");
    setup.host.stop_debug_server();
}

#[test]
fn disabled_allow_flags_are_audited_policy_denials() {
    let Some(mut setup) = Setup::new("allow-off", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let result = start_session(&mut client, &["observe", "write", "force"]);
    let scopes: Vec<_> = field(field(&result, "capabilities"), "scopes")
        .as_array()
        .expect("scopes")
        .iter()
        .filter_map(Json::as_str)
        .collect();
    assert_eq!(scopes, vec!["observe"]);
    let events = setup.audit();
    let denials: Vec<_> = events
        .iter()
        .filter(|event| text(event, "event") == "policy-deny")
        .collect();
    assert_eq!(denials.len(), 2, "write and force are both denied");
    assert!(denials.iter().all(|event| {
        event
            .as_object()
            .and_then(|object| object.get("detail"))
            .and_then(Json::as_object)
            .and_then(|detail| detail.get("reason"))
            .and_then(Json::as_str)
            == Some("scope-disabled")
    }));
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

#[test]
fn session_status_stop_second_session_and_capabilities() {
    let Some(mut setup) = Setup::new("session-lifecycle", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let response = session_start_response(&mut client, &["observe"]);
    let result = response_result(&response);
    let session_id = text(result, "session").to_owned();
    assert!(session_id.starts_with("s1-"));
    let capabilities = field(result, "capabilities");
    assert!(!boolean(capabilities, "hooks"));
    assert!(!boolean(capabilities, "checkpoints"));
    assert!(!boolean(capabilities, "stepping"));
    assert!(field(capabilities, "debug_info").as_str().is_some());
    assert_eq!(number(capabilities, "max_watch_points"), 256);
    assert_eq!(number(capabilities, "max_breakpoints"), 1024);
    assert_eq!(number(result, "generation"), setup.info.generation);
    assert_eq!(
        text(result, "build_id"),
        setup.info.build_id.to_hex().as_str()
    );
    assert_eq!(text(result, "mode"), "debug");
    let warnings = response_warning_codes(&response);
    assert!(
        warnings
            .iter()
            .any(|code| code == codes::W_DEBUG_MONITORING_ONLY),
        "a hook-free build reports monitoring-only: {warnings:?}"
    );

    let status =
        call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([])).expect("status");
    assert_eq!(text(&status, "state"), "active");
    assert_eq!(text(&status, "session"), session_id);
    assert_eq!(number(&status, "generation"), setup.info.generation);
    assert_eq!(
        text(&status, "build_id"),
        setup.info.build_id.to_hex().as_str()
    );
    assert!(field(&status, "stop").is_null());
    assert!(field(&status, "pause_remaining_ms").is_null());
    assert_eq!(number(&status, "queued_writes"), 0);
    assert_eq!(number(&status, "active_forces"), 0);
    assert_eq!(number(&status, "watches"), 0);
    assert_eq!(number(&status, "breakpoints"), 0);

    // A second connection cannot start a session on the same host.
    let mut second = setup.client();
    let error = second
        .start_session(&["observe"])
        .expect_err("second session");
    assert_eq!(error.code.as_deref(), Some(codes::DEBUG_SESSION));
    let status = call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([]))
        .expect("status after denial");
    assert_eq!(text(&status, "state"), "active");

    let ended = stop_session(&mut setup.host, &mut client);
    let (reason, code, generation, build) = session_ended(&ended);
    assert_eq!(reason, "client");
    assert!(code.is_none());
    assert_eq!(generation, setup.info.generation);
    assert_eq!(build, setup.info.build_id.to_hex());

    // A fresh session on the same endpoint starts at sequence 1 with no
    // replayed events.
    let mut third = setup.client();
    let result = start_session(&mut third, &["observe"]);
    assert!(text(&result, "session").starts_with("s2-"));
    assert!(third.take_events().is_empty(), "no event is replayed");
    let status = call_ok_pumped(&mut setup.host, &mut third, "session.status", obj([]))
        .expect("second session status");
    assert_eq!(text(&status, "state"), "active");
    let _ = stop_session(&mut setup.host, &mut third);
    setup.host.stop_debug_server();
}

#[test]
fn disconnect_clears_state_and_reconnect_has_no_replay() {
    let Some(mut setup) = Setup::new("disconnect", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            ("targets", Json::Array(vec![target_path("G_DINT")])),
            ("interval_ms", Json::unsigned(50)),
        ]),
    )
    .expect("watch.add");
    drop(client);

    // Reconnect on the same endpoint. `session.start` returns E-007 until the
    // disconnected session's teardown completes; the server is pumped so the
    // clear command can finish.
    let mut client = setup.client();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut started = None;
    while Instant::now() < deadline {
        match client.start_session(&["observe"]) {
            Ok(result) => {
                started = Some(result);
                break;
            }
            Err(error) if error.code.as_deref() == Some(codes::DEBUG_SESSION) => {
                settle(&mut setup.host, 10);
            }
            Err(error) => panic!("reconnect failed: {error}"),
        }
    }
    let started = started.expect("reconnect session");
    assert!(text(&started, "session").starts_with("s2-"));
    assert!(client.take_events().is_empty(), "no watch or state replay");
    let list =
        call_ok_pumped(&mut setup.host, &mut client, "watch.list", obj([])).expect("watch.list");
    assert!(
        field(&list, "watches")
            .as_array()
            .expect("watches")
            .is_empty()
    );
    let status =
        call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([])).expect("status");
    assert_eq!(number(&status, "watches"), 0);

    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "disconnect"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("reason"))
                .and_then(Json::as_str)
                == Some("disconnect")
    }));
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[test]
fn idle_timeout_ends_the_session_but_not_while_a_watch_is_active() {
    let Some(mut setup) = Setup::new("idle-timeout", false, false, Duration::from_secs(2)) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let watch = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            ("targets", Json::Array(vec![target_path("G_DINT")])),
            ("interval_ms", Json::unsigned(50)),
        ]),
    )
    .expect("watch.add");
    let watch_id = number(&watch, "watch_id");
    // A watch keeps the session alive past the bound. The bound is generous
    // enough that setting up the watch is not itself subject to a scheduling
    // race under a loaded parallel test run.
    std::thread::sleep(Duration::from_millis(2500));
    let status = call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([]))
        .expect("status with watch");
    assert_eq!(text(&status, "state"), "active");
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.remove",
        obj([("watch_id", Json::unsigned(watch_id))]),
    )
    .expect("watch.remove");

    let event = next_session_ended_pumped(&mut setup.host, &mut client);
    let (reason, code, generation, build) = session_ended(&event);
    assert_eq!(reason, "idle-timeout");
    assert_eq!(code, Some(codes::DEBUG_SESSION_TIMEOUT));
    assert_eq!(generation, setup.info.generation);
    assert_eq!(build, setup.info.build_id.to_hex());
    let _ = drain_until_eof(&mut setup.host, &mut client);
    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "timeout"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("reason"))
                .and_then(Json::as_str)
                == Some("idle")
    }));
    setup.host.stop_debug_server();
}

#[test]
fn swap_teardown_ends_the_session_with_e019_and_refreshes_the_endpoint() {
    let Some(mut setup) = Setup::new("swap", false, false, Duration::ZERO) else {
        return;
    };
    let old_generation = setup.info.generation;
    let old_build = setup.info.build_id;
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            ("targets", Json::Array(vec![target_path("G_DINT")])),
            ("interval_ms", Json::unsigned(50)),
        ]),
    )
    .expect("watch.add");

    let (event, outcome) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            loop {
                let event = client.next_event().expect("event");
                if event_name(&event) == "session-ended" {
                    break event;
                }
                assert_eq!(event_name(&event), "watch", "{}", event.render_compact());
            }
        });
        let outcome = setup
            .host
            .load(open(&setup.fixture), LoadMode::Cold)
            .expect("cold swap");
        pump_until_finished(&mut setup.host, &handle, "swap event");
        let event = handle.join().expect("event thread");
        (event, outcome)
    });
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    assert!(generation > old_generation);
    let (reason, code, event_generation, event_build) = session_ended(&event);
    assert_eq!(reason, "swap");
    assert_eq!(code, Some(codes::DEBUG_SWAP));
    assert_eq!(event_generation, old_generation);
    assert_eq!(event_build, old_build.to_hex());
    let _ = drain_until_eof(&mut setup.host, &mut client);

    let endpoint = setup.endpoint_file();
    let info = setup.host.debug_server_info().expect("running server");
    assert_eq!(endpoint.generation, generation);
    assert_eq!(info.generation, generation);
    assert_eq!(endpoint.build_id, info.build_id.to_hex());
    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "swap-end" && number(event, "generation") == old_generation
    }));

    // The host remains serving a new session on the new generation. The old
    // handler may still be releasing its session view, so retry E-007.
    let mut client = setup.client();
    let mut started = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match client.start_session(&["observe"]) {
            Ok(result) => {
                started = Some(result);
                break;
            }
            Err(error) if error.code.as_deref() == Some(codes::DEBUG_SESSION) => {
                settle(&mut setup.host, 10);
            }
            Err(error) => panic!("session after swap failed: {error}"),
        }
    }
    let started = started.expect("session after swap");
    assert_eq!(number(&started, "generation"), generation);
    let status = call_ok_pumped(&mut setup.host, &mut client, "session.status", obj([]))
        .expect("status after swap");
    assert_eq!(number(&status, "generation"), generation);
    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[test]
fn host_shutdown_ends_the_session_and_removes_the_endpoint() {
    let Some(mut setup) = Setup::new("shutdown", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let event = std::thread::scope(|scope| {
        let handle = scope.spawn(|| client.next_event());
        setup.host.shutdown().expect("host shutdown");
        pump_until_finished(&mut setup.host, &handle, "shutdown event");
        handle.join().expect("event thread").expect("event")
    });
    let (reason, code, generation, build) = session_ended(&event);
    assert_eq!(reason, "shutdown");
    assert!(code.is_none());
    assert_eq!(generation, setup.info.generation);
    assert_eq!(build, setup.info.build_id.to_hex());
    assert!(matches!(client.read_message(), Ok(None)));
    assert!(!setup.dir.endpoint().exists());
    assert!(setup.host.debug_server_info().is_none());
    let names = audit_names(&setup.info.audit_log);
    assert!(names.iter().any(|name| name == "session-end"));
    assert_eq!(names.last().map(String::as_str), Some("server-stop"));
}

#[test]
fn stopping_the_server_ends_the_session_and_removes_the_endpoint() {
    let Some(mut setup) = Setup::new("server-stop", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    setup.host.stop_debug_server();
    assert!(!setup.dir.endpoint().exists());
    assert!(setup.host.debug_server_info().is_none());
    let messages = drain_until_eof(&mut setup.host, &mut client);
    let ended = find_session_ended(&messages);
    let (reason, code, generation, build) = session_ended(&ended);
    assert_eq!(reason, "shutdown");
    assert!(code.is_none());
    assert_eq!(generation, setup.info.generation);
    assert_eq!(build, setup.info.build_id.to_hex());
    let names = audit_names(&setup.info.audit_log);
    assert!(names.iter().any(|name| name == "session-end"));
    assert!(names.iter().any(|name| name == "server-stop"));
}

// ---------------------------------------------------------------------------
// Typed reads, writes, forces, releases, and audit content
// ---------------------------------------------------------------------------

#[test]
fn var_read_write_force_release_and_queue_results() {
    let Some(mut setup) = Setup::new("var-operations", true, true, Duration::ZERO) else {
        return;
    };
    let dint = setup.dint_type();
    put(
        &mut setup.host,
        &setup.artifacts,
        "G_DINT",
        &7_i32.to_le_bytes(),
    );
    put(
        &mut setup.host,
        &setup.artifacts,
        "G_ARRAY",
        &[
            1_i32.to_le_bytes(),
            2_i32.to_le_bytes(),
            3_i32.to_le_bytes(),
        ]
        .concat(),
    );
    put(
        &mut setup.host,
        &setup.artifacts,
        "G_RETAINED",
        &11_i32.to_le_bytes(),
    );
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe", "write", "force"]);

    let read = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("G_DINT"))]),
    )
    .expect("read");
    assert_eq!(text(field(&read, "value"), "type"), dint.as_str());
    assert_eq!(text(field(&read, "value"), "value"), "7");
    assert_eq!(number(&read, "generation"), setup.info.generation);
    assert_eq!(
        text(&read, "build_id"),
        setup.info.build_id.to_hex().as_str()
    );
    assert!(boolean(&read, "consistent"));
    assert!(boolean(&read, "atomic"));
    assert!(!boolean(&read, "stopped"));
    assert!(!read.render_compact().contains("address"));

    let range = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([
            ("target", target_path("G_ARRAY")),
            (
                "range",
                obj([("start", Json::unsigned(1)), ("count", Json::unsigned(2))]),
            ),
        ]),
    )
    .expect("array range");
    let elements = field(field(field(&range, "value"), "value"), "elements")
        .as_array()
        .expect("elements");
    assert_eq!(elements.len(), 2);
    assert_eq!(text(&elements[0], "value"), "2");
    assert_eq!(text(&elements[1], "value"), "3");
    let range_error = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([
            ("target", target_path("G_DINT")),
            (
                "range",
                obj([("start", Json::unsigned(0)), ("count", Json::unsigned(1))]),
            ),
        ]),
    )
    .expect_err("range on a scalar");
    assert_eq!(range_error.code.as_deref(), Some(codes::DEBUG_TYPE));

    // Writes and forces are queued and applied at the boundary.
    let written = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("G_DINT")),
            ("value", decimal_value(&dint, "5")),
        ]),
    )
    .expect("write");
    assert_eq!(text(&written, "status"), "queued");
    let queue_id = number(&written, "queue_id");
    setup.host.cycle_at(1_000_000).expect("cycle");
    let event = next_event_pumped(&mut setup.host, &mut client);
    assert_eq!(event_name(&event), "queue-result");
    let data = event_data(&event);
    assert_eq!(text(data, "operation"), "write");
    assert_eq!(text(data, "outcome"), "applied");
    assert_eq!(number(data, "queue_id"), queue_id);
    assert!(number(data, "generation") > 0);
    assert_eq!(
        text(data, "build_id"),
        setup.info.build_id.to_hex().as_str()
    );
    let read = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("G_DINT"))]),
    )
    .expect("read after write");
    assert_eq!(text(field(&read, "value"), "value"), "5");

    let forced = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.force",
        obj([
            ("target", target_path("G_DINT")),
            ("value", decimal_value(&dint, "9")),
        ]),
    )
    .expect("force");
    let force_queue = number(&forced, "queue_id");
    setup.host.cycle_at(2_000_000).expect("force cycle");
    let event = next_event_pumped(&mut setup.host, &mut client);
    assert_eq!(number(event_data(&event), "queue_id"), force_queue);
    assert_eq!(text(event_data(&event), "operation"), "force");
    assert_eq!(text(event_data(&event), "outcome"), "applied");
    let read = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("G_DINT"))]),
    )
    .expect("read after force");
    assert_eq!(text(field(&read, "value"), "value"), "9");

    let released = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.release",
        obj([("target", target_path("G_DINT"))]),
    )
    .expect("release");
    assert!(boolean(&released, "released"));
    assert_eq!(text(&released, "effective"), "next-boundary");
    let released = call_ok_pumped(&mut setup.host, &mut client, "var.release-all", obj([]))
        .expect("release-all");
    assert_eq!(number(&released, "released"), 0);

    // Stale, unknown, and non-leaf writes are refused.
    let stale = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("G_DINT")),
            ("value", decimal_value(&dint, "1")),
            (
                "expected_generation",
                Json::unsigned(setup.info.generation + 5),
            ),
        ]),
    )
    .expect_err("stale write");
    assert_eq!(stale.code.as_deref(), Some(codes::DEBUG_STALE));
    let unknown = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("NO_SUCH_ROOT")),
            ("value", decimal_value(&dint, "1")),
        ]),
    )
    .expect_err("unknown write");
    assert_eq!(unknown.code.as_deref(), Some(codes::DEBUG_UNKNOWN));
    let aggregate = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("G_POINT")),
            ("value", decimal_value(&dint, "1")),
        ]),
    )
    .expect_err("aggregate write");
    assert_eq!(aggregate.code.as_deref(), Some(codes::DEBUG_UNKNOWN));

    // A protected read is audited once.
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.read",
        obj([("target", target_path("G_RETAINED"))]),
    )
    .expect("protected read");
    let events = setup.audit();
    let protected: Vec<_> = events
        .iter()
        .filter(|event| text(event, "event") == "read-protected")
        .collect();
    assert_eq!(protected.len(), 1);
    assert_eq!(text(protected[0], "scope"), "observe");
    assert!(text(field(protected[0], "target"), "key").ends_with("G_RETAINED"));

    // A value over 4 KiB is truncated in the audit log.
    let big_type = root_type_key(&setup.artifacts, "G_BIG");
    let elements = (0..1024)
        .map(|value| decimal_value("DINT", &value.to_string()))
        .collect::<Vec<_>>();
    let big = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "var.write",
        obj([
            ("target", target_path("G_BIG")),
            (
                "value",
                obj([
                    ("type", Json::string(big_type)),
                    ("value", obj([("elements", Json::Array(elements))])),
                ]),
            ),
        ]),
    )
    .expect("big write");
    assert_eq!(text(&big, "status"), "queued");
    setup.host.cycle_at(3_000_000).expect("big cycle");
    let _ = next_event_pumped(&mut setup.host, &mut client);
    let events = setup.audit();
    let big_event = events
        .iter()
        .rev()
        .find(|event| text(event, "event") == "write")
        .expect("big write audit");
    let value = field(big_event, "value");
    assert!(
        boolean(value, "truncated"),
        "{}",
        big_event.render_compact()
    );
    assert!(number(value, "size") > 4096);

    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

// ---------------------------------------------------------------------------
// Watches and manifest export
// ---------------------------------------------------------------------------

#[test]
fn watch_add_list_batches_and_remove() {
    let Some(mut setup) = Setup::new("watches", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let added = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            (
                "targets",
                Json::Array(vec![target_path("G_DINT"), target_path("G_BOOL")]),
            ),
            ("interval_ms", Json::unsigned(50)),
        ]),
    )
    .expect("watch.add");
    let watch_id = number(&added, "watch_id");
    assert_eq!(number(&added, "interval_ms"), 50);
    let targets = field(&added, "targets").as_array().expect("targets");
    assert_eq!(targets.len(), 2);
    assert!(
        targets
            .iter()
            .filter_map(Json::as_str)
            .all(|target| target.contains("G_"))
    );

    let list =
        call_ok_pumped(&mut setup.host, &mut client, "watch.list", obj([])).expect("watch.list");
    let watches = field(&list, "watches").as_array().expect("watches");
    assert_eq!(watches.len(), 1);
    assert_eq!(number(&watches[0], "watch_id"), watch_id);

    let emitted = setup.host.debug_pump_at(60_000_000);
    assert!(emitted >= 1);
    let event = next_event_pumped(&mut setup.host, &mut client);
    assert_eq!(event_name(&event), "watch");
    let data = event_data(&event);
    assert_eq!(number(data, "watch_id"), watch_id);
    assert_eq!(number(data, "generation"), setup.info.generation);
    assert_eq!(
        text(data, "build_id"),
        setup.info.build_id.to_hex().as_str()
    );
    assert!(boolean(data, "consistent"));
    let values = field(data, "values").as_array().expect("values");
    assert_eq!(values.len(), 2);
    for value in values {
        assert!(boolean(value, "available"));
        assert_eq!(text(field(value, "target"), "kind"), "path");
        assert!(field(field(value, "target"), "key").as_str().is_some());
        assert!(field(value, "value").as_object().is_some());
        assert!(!value.render_compact().contains("address"));
    }

    let removed = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.remove",
        obj([("watch_id", Json::unsigned(watch_id))]),
    )
    .expect("watch.remove");
    assert!(boolean(&removed, "removed"));
    let unknown = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.remove",
        obj([("watch_id", Json::unsigned(watch_id))]),
    )
    .expect_err("unknown watch");
    assert_eq!(unknown.code.as_deref(), Some(codes::DEBUG_UNKNOWN));

    // The interval is clamped to the documented range.
    let clamped = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([
            ("targets", Json::Array(vec![target_path("G_DINT")])),
            ("interval_ms", Json::unsigned(0)),
        ]),
    )
    .expect("clamped watch");
    assert_eq!(number(&clamped, "interval_ms"), 50);
    let _ = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.remove",
        obj([("watch_id", Json::unsigned(number(&clamped, "watch_id")))]),
    )
    .expect("remove clamped");

    let too_many = Json::Array((0..257).map(|_| target_path("G_DINT")).collect());
    let error = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "watch.add",
        obj([("targets", too_many), ("interval_ms", Json::unsigned(50))]),
    )
    .expect_err("watch limit");
    assert_eq!(error.code.as_deref(), Some(codes::DEBUG_LIMIT));

    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

#[test]
fn manifest_get_returns_json_and_binary_with_audit() {
    let Some(mut setup) = Setup::new("manifest", false, false, Duration::ZERO) else {
        return;
    };
    let mut client = setup.client();
    let _ = start_session(&mut client, &["observe"]);
    let json_result = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "manifest.get",
        obj([("format", Json::string("json"))]),
    )
    .expect("manifest json");
    assert_eq!(text(&json_result, "format"), "json");
    let digest = text(&json_result, "digest").to_owned();
    assert_eq!(digest.len(), 66);
    assert!(digest.starts_with("0x"), "digest is 0x-prefixed hex");
    let data = field(&json_result, "data");
    let object = data.as_object().expect("manifest object");
    assert!(object.contains_key("body"));
    assert!(object.contains_key("digest"));
    let body = field(data, "body");
    assert!(
        body.as_object()
            .expect("manifest body")
            .contains_key("header")
    );
    assert!(
        body.as_object()
            .expect("manifest body")
            .contains_key("roots")
    );
    assert!(
        body.as_object()
            .expect("manifest body")
            .contains_key("slots")
    );

    let binary = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "manifest.get",
        obj([("format", Json::string("binary"))]),
    )
    .expect("manifest binary");
    assert_eq!(text(&binary, "format"), "binary");
    assert_eq!(text(&binary, "digest"), digest.as_str());
    let encoded = text(&binary, "data");
    assert!(!encoded.is_empty());
    assert_eq!(encoded.len() % 4, 0);

    let invalid = call_ok_pumped(
        &mut setup.host,
        &mut client,
        "manifest.get",
        obj([("format", Json::string("xml"))]),
    )
    .expect_err("invalid manifest format");
    assert_eq!(invalid.code.as_deref(), Some(codes::DEBUG_REQUEST));

    let events = setup.audit();
    let exports: Vec<_> = events
        .iter()
        .filter(|event| text(event, "event") == "manifest-export")
        .collect();
    assert_eq!(exports.len(), 2);
    assert_eq!(text(exports[0], "scope"), "observe");
    assert_eq!(text(field(exports[0], "detail"), "format"), "json");
    assert_eq!(text(field(exports[1], "detail"), "format"), "binary");

    let _ = stop_session(&mut setup.host, &mut client);
    setup.host.stop_debug_server();
}

// ---------------------------------------------------------------------------
// Rate limits and startup refusal
// ---------------------------------------------------------------------------

#[test]
fn request_rate_limit_closes_after_twenty_rate_errors() {
    let Some(mut setup) = Setup::new("rate-limit", false, false, Duration::ZERO) else {
        return;
    };
    let endpoint = setup.endpoint_file();
    let token = endpoint.token;
    // A pipelined flood requires independent write and read halves; reading
    // only after writing all frames would deadlock once the server's send
    // buffer filled. The raw loopback stream keeps that test deterministic.
    let mut stream = TcpStream::connect(&endpoint.endpoint).expect("rate transport");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("rate stream clone"));
    send_stream_frame(
        &mut stream,
        1,
        &token,
        "session.start",
        session_start_params(&["observe", "control"]),
    );
    let start = read_stream_message(&mut reader).expect("session.start response");
    assert_eq!(text(&start, "status"), "ok");

    // Read each small batch before sending the next. A batch is smaller than
    // the twenty-error breaker, and every earlier batch was error-free, so the
    // connection is still open while the first E-012 responses are read. That
    // avoids the Linux RST that can discard queued responses when a flood
    // makes the server close with unread input in its socket.
    let mut rate_errors = 0_u64;
    let mut next_id = 2_u64;
    for _ in 0..40 {
        for id in next_id..next_id + 19 {
            assert!(
                send_stream_frame(&mut stream, id, &token, "exec.continue", obj([])),
                "rate batch write"
            );
        }
        let _ = stream.flush();
        for _ in 0..19 {
            let message = read_stream_message(&mut reader).expect("rate batch response");
            assert_eq!(message_kind(&message), Some("response"));
            if message
                .as_object()
                .and_then(|object| object.get("error"))
                .and_then(Json::as_object)
                .and_then(|error| error.get("code"))
                .and_then(Json::as_str)
                == Some(codes::DEBUG_RATE)
            {
                rate_errors += 1;
            }
        }
        next_id += 19;
        if rate_errors > 0 {
            break;
        }
    }
    assert!(rate_errors >= 1, "the burst limit was exceeded");

    // Once an E-012 has been observed, flood far past the breaker. This close
    // may reset the socket before the final responses are read, so the limit
    // and teardown assertions below use the audit log.
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for id in next_id..next_id + 2000 {
                if !send_stream_frame(&mut stream, id, &token, "exec.continue", obj([])) {
                    break;
                }
            }
            let _ = stream.flush();
        });
        let handle = scope.spawn(|| while read_stream_message(&mut reader).is_some() {});
        pump_until_finished(&mut setup.host, &handle, "rate flood");
        handle.join().expect("rate reader");
        writer.join().expect("rate writer");
    });
    settle(&mut setup.host, 50);
    let events = setup.audit();
    assert!(events.iter().any(|event| {
        text(event, "event") == "transport-error"
            && event
                .as_object()
                .and_then(|object| object.get("detail"))
                .and_then(Json::as_object)
                .and_then(|detail| detail.get("reason"))
                .and_then(Json::as_str)
                == Some("rate-limit")
    }));
    assert!(
        events
            .iter()
            .any(|event| text(event, "event") == "session-end")
    );
    setup.host.stop_debug_server();
}

#[test]
fn server_start_refuses_insecure_paths_and_audit_failures() {
    let Some(mut setup) = Setup::new("startup", false, false, Duration::ZERO) else {
        return;
    };
    // A second server on the same host is refused.
    let error = setup
        .host
        .start_debug_server(&DebugServerConfig::new(
            setup.dir.named("second.json"),
            TransportKind::Socket,
        ))
        .expect_err("second server");
    assert_eq!(error.code, codes::DEBUG_TRANSPORT);
    setup.host.stop_debug_server();

    // A missing parent directory is refused while the audit log is created
    // first, so the audit failure is reported.
    let missing = setup.dir.path.join("missing").join("debug.json");
    let error = setup
        .host
        .start_debug_server(&DebugServerConfig::new(
            missing.clone(),
            TransportKind::Socket,
        ))
        .expect_err("missing parent");
    assert_eq!(error.code, codes::DEBUG_AUDIT);
    assert!(!missing.exists());

    // An existing endpoint file is never reused.
    let existing = setup.dir.named("existing.json");
    fs::write(&existing, b"stale").expect("stale endpoint");
    let error = setup
        .host
        .start_debug_server(&DebugServerConfig::new(
            existing.clone(),
            TransportKind::Socket,
        ))
        .expect_err("stale endpoint");
    assert_eq!(error.code, codes::DEBUG_TRANSPORT);
    assert_eq!(fs::read(&existing).expect("stale bytes"), b"stale");
    assert!(
        !error
            .message
            .contains(&setup.dir.path.to_string_lossy().into_owned())
    );

    // An uncreatable audit log prevents startup.
    let audit_endpoint = setup.dir.named("auditcase.json");
    let audit_path = audit_endpoint.with_extension("audit.jsonl");
    fs::write(&audit_path, b"occupied").expect("occupied audit path");
    let error = setup
        .host
        .start_debug_server(&DebugServerConfig::new(
            audit_endpoint.clone(),
            TransportKind::Socket,
        ))
        .expect_err("audit path");
    assert_eq!(error.code, codes::DEBUG_AUDIT);
    assert!(!audit_endpoint.exists());

    #[cfg(unix)]
    {
        let target = setup.dir.named("symlink-target.json");
        fs::write(&target, b"target").expect("target");
        let link = setup.dir.named("symlink.json");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = setup
            .host
            .start_debug_server(&DebugServerConfig::new(link, TransportKind::Socket))
            .expect_err("symlink endpoint");
        assert_eq!(error.code, codes::DEBUG_TRANSPORT);
        assert_eq!(fs::read(&target).expect("target bytes"), b"target");
    }

    // A host without an active generation cannot start the service.
    let mut empty = in_memory_host();
    let empty_dir = TestDir::new("empty-host");
    let error = empty
        .start_debug_server(&DebugServerConfig::new(
            empty_dir.endpoint(),
            TransportKind::Socket,
        ))
        .expect_err("empty host");
    assert_eq!(error.code, codes::DEBUG_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Build capabilities
// ---------------------------------------------------------------------------

#[test]
fn capabilities_cover_release_test_and_hook_free_builds() {
    let cases = [
        ("cap-default", BuildFlags::default(), "debug", None),
        (
            "cap-release",
            BuildFlags {
                release: true,
                ..BuildFlags::default()
            },
            "release",
            None,
        ),
        (
            "cap-test",
            BuildFlags {
                test_build: true,
                ..BuildFlags::default()
            },
            "debug",
            None,
        ),
        (
            "cap-hooks",
            BuildFlags {
                debug_hooks: true,
                ..BuildFlags::default()
            },
            "debug",
            Some(codes::DEBUG_METADATA),
        ),
    ];
    for (name, flags, mode, metadata_code) in cases {
        let artifacts = common::artifacts_with_flags(SOURCE, name, flags);
        let Some(fixture) = Fixture::build(name, artifacts.clone(), Behavior::default()) else {
            return;
        };
        let mut host = in_memory_host();
        cold_install(&mut host, &fixture);
        let dir = TestDir::new(name);
        let info = start(
            &mut host,
            dir.endpoint(),
            TransportKind::Socket,
            false,
            false,
            Duration::ZERO,
        );
        assert_eq!(info.mode, mode, "{name}");
        let endpoint = DebugEndpointFile::read(&dir.endpoint()).expect("endpoint");
        assert_eq!(endpoint.mode, mode, "{name}");
        let mut client = DebugClient::connect_file(&dir.endpoint()).expect("client");
        let response = session_start_response(&mut client, &["observe"]);
        let result = response_result(&response);
        assert_eq!(text(result, "mode"), mode, "{name}");
        let capabilities = field(result, "capabilities");
        assert!(!boolean(capabilities, "hooks"), "{name}");
        assert!(!boolean(capabilities, "checkpoints"), "{name}");
        assert!(!boolean(capabilities, "stepping"), "{name}");
        assert_eq!(number(capabilities, "max_watch_points"), 256);
        assert_eq!(number(capabilities, "max_breakpoints"), 1024);
        let warnings = response_warning_codes(&response);
        assert!(
            warnings
                .iter()
                .any(|code| code == codes::W_DEBUG_MONITORING_ONLY),
            "{name} must warn monitoring-only"
        );
        let warning = field(&response, "warnings")
            .as_array()
            .expect("warnings")
            .iter()
            .find(|warning| text(warning, "code") == codes::W_DEBUG_MONITORING_ONLY)
            .expect("warning");
        let detail = field(warning, "detail");
        match metadata_code {
            Some(code) => {
                assert_eq!(text(detail, "code"), code, "{name}");
                assert!(field(detail, "message").as_str().is_some());
            }
            None => assert!(detail.as_object().is_some(), "{name}"),
        }
        host.stop_debug_server();
    }
}
