//! `stc serve`: persistent remote-debug runtime on the host (WL-1.2).
//!
//! The host-side serve command builds (or reuses) a project, starts the runtime
//! with a loopback debug socket and a control endpoint, and writes the
//! user-only `stc-remote-session.json` descriptor. `status`, `deploy`, and
//! `stop` act on that descriptor through the runtime control channel. The local
//! driver invokes these commands over SSH; the debug adapter never speaks SSH.

use std::fs;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use stc_diag::Severity;
use stc_got::error::codes;
use stc_got::json::Json;
use stc_runtime::control::ControlClient;
use stc_runtime::{DebugEndpointFile, Download};

use crate::build::BuildMode;
use crate::cli::{Invocation, ServeAction};
use crate::project;
use crate::remote::{self, RemotePath, RemoteSession};
use crate::report::{Log, Reporter};

/// Deadline for the runtime to publish its debug endpoint.
const ENDPOINT_DEADLINE: Duration = Duration::from_secs(10);
/// Deadline for a control request to connect.
const CONTROL_DEADLINE: Duration = Duration::from_secs(10);
const DEFAULT_SESSION_FILE: &str = "stc-remote-session.json";
const DEFAULT_DEBUG_ENDPOINT: &str = "stc-debug.endpoint";
const DEFAULT_CONTROL_ENDPOINT: &str = "stc-control.sock";
const DEFAULT_NONCE_FILE: &str = "stc-control.nonce";
const NONCE_FILE_LIMIT: u64 = 4096;

/// Runs one `stc serve` action.
pub(crate) fn run(invocation: &Invocation, reporter: &mut Reporter, log: Log) -> u8 {
    if !cfg!(unix) {
        reporter.plain(
            codes::REMOTE_UNSUPPORTED,
            Severity::Error,
            "`stc serve` is supported on the remote Unix host; this build is not Unix",
        );
        return 2;
    }
    match invocation.serve.action {
        ServeAction::Start => start(invocation, reporter, log),
        ServeAction::Status => status(invocation, reporter),
        ServeAction::Deploy => deploy(invocation, reporter),
        ServeAction::Release => release(invocation, reporter),
        ServeAction::Stop => stop(invocation, reporter),
    }
}

fn start(invocation: &Invocation, reporter: &mut Reporter, log: Log) -> u8 {
    let work_dir = match resolve_work_dir(invocation) {
        Ok(directory) => directory,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let session_path = match output_path(
        &work_dir,
        invocation.serve.session.as_deref(),
        DEFAULT_SESSION_FILE,
    ) {
        Ok(path) => path,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let debug_endpoint = match output_path(
        &work_dir,
        invocation.debug.server_endpoint.as_deref(),
        DEFAULT_DEBUG_ENDPOINT,
    ) {
        Ok(path) => path,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let control_endpoint = match output_path(
        &work_dir,
        invocation.serve.control_endpoint.as_deref(),
        DEFAULT_CONTROL_ENDPOINT,
    ) {
        Ok(path) => path,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let nonce_path = work_dir.join(DEFAULT_NONCE_FILE);
    let start_gate = invocation
        .serve
        .start_paused
        .then(|| work_dir.join("stc-start.gate"));
    if let Some(gate) = &start_gate {
        let _ = fs::remove_file(gate);
    }
    for (label, path) in [
        ("session", &session_path),
        ("debug endpoint", &debug_endpoint),
        ("control endpoint", &control_endpoint),
        ("control nonce", &nonce_path),
    ] {
        if !path.starts_with(&work_dir) {
            reporter.plain(
                codes::REMOTE_SESSION,
                Severity::Error,
                &format!(
                    "the {label} path `{}` must be under the work directory `{}`",
                    path.display(),
                    work_dir.display()
                ),
            );
            return 2;
        }
    }
    if session_path.exists() {
        reporter.plain(
            codes::REMOTE_SESSION,
            Severity::Error,
            &format!(
                "a serve session already exists at `{}`; stop it before starting another",
                session_path.display()
            ),
        );
        return 2;
    }
    if let Err(message) = crate::debug::prepare_debug_endpoint(&debug_endpoint) {
        reporter.plain(codes::DEBUG_TRANSPORT, Severity::Error, &message);
        return 2;
    }
    match fs::remove_file(&control_endpoint) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            reporter.plain(
                codes::DEBUG_TRANSPORT,
                Severity::Error,
                &format!(
                    "cannot remove the stale control endpoint `{}`: {error}",
                    control_endpoint.display()
                ),
            );
            return 2;
        }
    }

    let build = match build(invocation, reporter, log) {
        Ok(build) => build,
        Err(code) => return code,
    };
    let Some(module) = build.module.clone() else {
        reporter.plain(
            codes::REMOTE_BUILD,
            Severity::Error,
            "the build did not report a loadable module path",
        );
        return 2;
    };
    if !module.is_file() {
        reporter.plain(
            codes::REMOTE_BUILD,
            Severity::Error,
            &format!("the built module `{}` does not exist", module.display()),
        );
        return 2;
    }
    if !build.got_manifest.is_file() {
        reporter.plain(
            codes::REMOTE_BUILD,
            Severity::Error,
            &format!(
                "the built manifest `{}` does not exist",
                build.got_manifest.display()
            ),
        );
        return 2;
    }
    let source_fingerprint = match remote::local_source_fingerprint(&build.manifest_path) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            reporter.plain(error.code, Severity::Error, &error.message);
            return 2;
        }
    };

    let nonce = crate::debug::random_hex();
    if let Err(message) = write_private_file(&nonce_path, format!("{nonce}\n").as_bytes()) {
        reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
        return 2;
    }

    let mut args =
        crate::run::run_arguments(&build.run, &build.manifest, &build.root_dir, log.verbose());
    args.extend(crate::debug::server_arguments_for(
        &invocation.debug,
        &build.manifest,
        &debug_endpoint,
    ));
    args.push("--control".to_owned());
    args.push(control_endpoint.to_string_lossy().into_owned());
    args.push("--control-nonce-file".to_owned());
    args.push(nonce_path.to_string_lossy().into_owned());
    if let Some(gate) = &start_gate {
        args.push("--start-gate".to_owned());
        args.push(gate.to_string_lossy().into_owned());
    }
    let mut child = match Command::new(&build.launcher)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_file(&nonce_path);
            reporter.plain(
                codes::REMOTE_TRANSPORT,
                Severity::Error,
                &format!(
                    "cannot start the serve host `{}`: {error}",
                    build.launcher.display()
                ),
            );
            return 2;
        }
    };

    let endpoint = match wait_for_endpoint(&mut child, &debug_endpoint) {
        Ok(endpoint) => endpoint,
        Err(message) => {
            let _ = child.kill();
            let _ = child.wait();
            cleanup(&session_path, &nonce_path);
            reporter.plain(codes::REMOTE_TRANSPORT, Severity::Error, &message);
            return 2;
        }
    };

    let session = match remote_session(
        &work_dir,
        &build,
        &module,
        &debug_endpoint,
        &control_endpoint,
        &nonce_path,
        start_gate.as_deref(),
        &endpoint,
        &source_fingerprint,
        child.id(),
    ) {
        Ok(session) => session,
        Err(message) => {
            let _ = child.kill();
            let _ = child.wait();
            cleanup(&session_path, &nonce_path);
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    if let Err(error) = session.validate() {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(&session_path, &nonce_path);
        reporter.plain(error.code, Severity::Error, &error.message);
        return 2;
    }
    if let Err(message) = write_private_file(&session_path, session.to_json_text().as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(&session_path, &nonce_path);
        reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
        return 2;
    }
    log.status(format!(
        "serve session {} (pid {}, build {}, generation {})",
        session_path.display(),
        child.id(),
        session.build_id,
        session.generation
    ));
    if invocation.format == crate::report::Format::Json {
        println!("{}", session.to_json_text().trim_end());
    }

    let status = child.wait();
    cleanup(&session_path, &nonce_path);
    status
        .ok()
        .and_then(|status| status.code())
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

fn build(
    invocation: &Invocation,
    reporter: &mut Reporter,
    log: Log,
) -> Result<crate::project::ProjectBuild, u8> {
    if invocation.serve.no_build {
        let project_path = Path::new(invocation.manifest.as_deref().unwrap_or("stc.toml"));
        let arguments = stc_dap::LaunchArguments {
            out_dir: invocation.build.out_dir.clone(),
            release: invocation.build.mode_override == Some(BuildMode::Release),
            no_build: true,
            ..stc_dap::LaunchArguments::default()
        };
        let mut build = crate::dap::no_build(project_path, &arguments).map_err(|message| {
            reporter.plain(codes::REMOTE_BUILD, Severity::Error, &message);
            2
        })?;
        build.run = invocation.build.run.clone();
        Ok(build)
    } else {
        project::build_for_launch(
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            reporter,
            log,
        )
    }
}

fn wait_for_endpoint(child: &mut Child, endpoint: &Path) -> Result<DebugEndpointFile, String> {
    let deadline = Instant::now() + ENDPOINT_DEADLINE;
    while !endpoint.is_file() {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the serve host exited before publishing its debug endpoint (exit code {})",
                status.code().unwrap_or(1)
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the serve host did not publish its debug endpoint `{}` within {} seconds",
                endpoint.display(),
                ENDPOINT_DEADLINE.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    DebugEndpointFile::read(endpoint).map_err(|error| {
        format!(
            "cannot read the serve debug endpoint `{}`: {error}",
            endpoint.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn remote_session(
    work_dir: &Path,
    build: &crate::project::ProjectBuild,
    module: &Path,
    debug_endpoint: &Path,
    control_endpoint: &Path,
    nonce_path: &Path,
    start_gate: Option<&Path>,
    endpoint: &DebugEndpointFile,
    source_fingerprint: &str,
    pid: u32,
) -> Result<RemoteSession, String> {
    let path = |value: &Path| -> Result<RemotePath, String> {
        RemotePath::parse_absolute(&canonical_string(value)?).map_err(|error| error.to_string())
    };
    Ok(RemoteSession {
        host: host_name(),
        work_dir: path(work_dir)?,
        project: path(&build.manifest_path)?,
        module: path(module)?,
        manifest: path(&build.got_manifest)?,
        debug_endpoint_file: path(debug_endpoint)?,
        control_endpoint: path(control_endpoint)?,
        control_nonce_file: path(nonce_path)?,
        start_gate: start_gate.map(path).transpose()?,
        build_id: endpoint.build_id.clone(),
        generation: endpoint.generation,
        source_fingerprint: source_fingerprint.to_owned(),
        mode: endpoint.mode.clone(),
        stc_version: env!("CARGO_PKG_VERSION").to_owned(),
        pid,
    })
}

fn status(invocation: &Invocation, reporter: &mut Reporter) -> u8 {
    let (session, nonce) = match load_session(invocation) {
        Ok(session) => session,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let mut client =
        match ControlClient::connect(session.control_endpoint.as_str(), CONTROL_DEADLINE) {
            Ok(client) => client,
            Err(message) => {
                reporter.plain(codes::REMOTE_TRANSPORT, Severity::Error, &message);
                return 2;
            }
        };
    let control = match client.request(
        &nonce,
        Json::object([("op".into(), Json::string("status"))]),
    ) {
        Ok(response) => response,
        Err(message) => {
            reporter.plain(codes::REMOTE_COMMAND, Severity::Error, &message);
            return 2;
        }
    };
    let control_ok = control
        .as_object()
        .and_then(|object| object.get("ok"))
        .and_then(Json::as_bool)
        == Some(true);
    let active = client
        .request(
            &nonce,
            Json::object([("op".into(), Json::string("download"))]),
        )
        .ok()
        .and_then(|response| {
            response
                .as_object()
                .and_then(|object| object.get("download"))
                .and_then(Json::as_str)
                .and_then(|text| Download::from_json_text(text).ok())
        })
        .map(|download| {
            Json::object([
                (
                    "build_id".into(),
                    Json::string(download.deployment.build_id.to_hex()),
                ),
                (
                    "source_fingerprint".into(),
                    Json::string(download.manifest().header.source_fingerprint.to_hex()),
                ),
            ])
        });
    if invocation.format == crate::report::Format::Json {
        let report = Json::object([
            ("active".into(), Json::optional(active)),
            ("control".into(), control),
            ("session".into(), session.to_json()),
        ]);
        println!("{}", report.render_compact());
    } else {
        println!(
            "serve {} pid {} build {} generation {}",
            session.host, session.pid, session.build_id, session.generation
        );
        println!("control {}", control.render_compact());
        if let Some(active) = active {
            println!("active {}", active.render_compact());
        }
    }
    if control_ok { 0 } else { 2 }
}

fn deploy(invocation: &Invocation, reporter: &mut Reporter) -> u8 {
    let Some(module) = invocation.serve.deploy_module.as_deref() else {
        reporter.plain(
            codes::REMOTE_DEPLOY,
            Severity::Error,
            "`stc serve deploy` needs `--module <path>` and `--deploy-manifest <path>`",
        );
        return 2;
    };
    let Some(manifest) = invocation.serve.deploy_manifest.as_deref() else {
        reporter.plain(
            codes::REMOTE_DEPLOY,
            Severity::Error,
            "`stc serve deploy` needs `--module <path>` and `--deploy-manifest <path>`",
        );
        return 2;
    };
    let (session, nonce) = match load_session(invocation) {
        Ok(session) => session,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let session_path = match session_descriptor_path(invocation) {
        Ok(path) => path,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let mut client =
        match ControlClient::connect(session.control_endpoint.as_str(), CONTROL_DEADLINE) {
            Ok(client) => client,
            Err(message) => {
                reporter.plain(codes::REMOTE_TRANSPORT, Severity::Error, &message);
                return 2;
            }
        };
    let request = Json::object([
        ("manifest".into(), Json::string(manifest)),
        ("module".into(), Json::string(module)),
        ("op".into(), Json::string("deploy")),
    ]);
    let response = match client.request(&nonce, request) {
        Ok(response) => response,
        Err(message) => {
            reporter.plain(codes::REMOTE_DEPLOY, Severity::Error, &message);
            return 2;
        }
    };
    let ok = response
        .as_object()
        .and_then(|object| object.get("ok"))
        .and_then(Json::as_bool)
        == Some(true);
    if ok {
        let generation = response
            .as_object()
            .and_then(|object| object.get("generation"))
            .and_then(Json::as_number)
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(session.generation);
        if let Err(message) = refresh_session_descriptor(
            &mut client,
            &nonce,
            &session,
            &session_path,
            module,
            manifest,
            generation,
        ) {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    }
    if invocation.format == crate::report::Format::Json {
        println!("{}", response.render_compact());
    } else {
        println!("deploy {}", response.render_compact());
    }
    if ok { 0 } else { 2 }
}

/// Updates the session descriptor after an online deployment.
fn refresh_session_descriptor(
    client: &mut ControlClient,
    nonce: &str,
    session: &RemoteSession,
    path: &Path,
    module: &str,
    manifest: &str,
    generation: u64,
) -> Result<(), String> {
    let response = client.request(
        nonce,
        Json::object([("op".into(), Json::string("download"))]),
    )?;
    let download = response
        .as_object()
        .and_then(|object| object.get("download"))
        .and_then(Json::as_str)
        .and_then(|text| Download::from_json_text(text).ok())
        .ok_or_else(|| "the control download response is invalid".to_owned())?;
    let mut updated = session.clone();
    updated.build_id = download.deployment.build_id.to_hex();
    updated.source_fingerprint = download.manifest().header.source_fingerprint.to_hex();
    updated.generation = generation;
    updated.module = RemotePath::parse_absolute(module).map_err(|error| error.to_string())?;
    updated.manifest = RemotePath::parse_absolute(manifest).map_err(|error| error.to_string())?;
    updated.validate().map_err(|error| error.to_string())?;
    write_private_file(path, updated.to_json_text().as_bytes())
}

fn release(invocation: &Invocation, reporter: &mut Reporter) -> u8 {
    let session = match load_session_descriptor(invocation) {
        Ok(session) => session,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let Some(gate) = session.start_gate.as_ref() else {
        reporter.plain(
            codes::REMOTE_SESSION,
            Severity::Error,
            "the serve session is not start-paused",
        );
        return 2;
    };
    match write_private_file(Path::new(gate.as_str()), b"start\n") {
        Ok(()) => 0,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            2
        }
    }
}

fn stop(invocation: &Invocation, reporter: &mut Reporter) -> u8 {
    let (session, nonce) = match load_session(invocation) {
        Ok(session) => session,
        Err(message) => {
            reporter.plain(codes::REMOTE_SESSION, Severity::Error, &message);
            return 2;
        }
    };
    let mut client =
        match ControlClient::connect(session.control_endpoint.as_str(), CONTROL_DEADLINE) {
            Ok(client) => client,
            Err(message) => {
                reporter.plain(codes::REMOTE_TRANSPORT, Severity::Error, &message);
                return 2;
            }
        };
    match client.request(&nonce, Json::object([("op".into(), Json::string("stop"))])) {
        Ok(response) => {
            if invocation.format == crate::report::Format::Json {
                println!("{}", response.render_compact());
            } else {
                println!("stop requested");
            }
            0
        }
        Err(message) => {
            reporter.plain(codes::REMOTE_COMMAND, Severity::Error, &message);
            2
        }
    }
}

fn load_session(invocation: &Invocation) -> Result<(RemoteSession, String), String> {
    let session = load_session_descriptor(invocation)?;
    let nonce = read_private_file(Path::new(session.control_nonce_file.as_str()))?;
    Ok((session, nonce))
}

fn load_session_descriptor(invocation: &Invocation) -> Result<RemoteSession, String> {
    let session_path = session_descriptor_path(invocation)?;
    let text = fs::read_to_string(&session_path).map_err(|error| {
        format!(
            "cannot read the serve session `{}`: {error}",
            session_path.display()
        )
    })?;
    RemoteSession::parse(&text).map_err(|error| error.to_string())
}

fn session_descriptor_path(invocation: &Invocation) -> Result<PathBuf, String> {
    let work_dir = resolve_work_dir(invocation)?;
    output_path(
        &work_dir,
        invocation.serve.session.as_deref(),
        DEFAULT_SESSION_FILE,
    )
}

fn resolve_work_dir(invocation: &Invocation) -> Result<PathBuf, String> {
    let path = invocation
        .serve
        .work_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "cannot create the work directory `{}`: {error}",
            path.display()
        )
    })?;
    fs::canonicalize(&path).map_err(|error| {
        format!(
            "cannot resolve the work directory `{}`: {error}",
            path.display()
        )
    })
}

fn output_path(work_dir: &Path, value: Option<&str>, default: &str) -> Result<PathBuf, String> {
    let path = value.map_or_else(|| work_dir.join(default), PathBuf::from);
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "the output path `{}` must not contain `..`",
            path.display()
        ));
    }
    Ok(if path.is_absolute() {
        path
    } else {
        work_dir.join(path)
    })
}

fn canonical_string(path: &Path) -> Result<String, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot read the current directory: {error}"))?
            .join(path)
    };
    Ok(fs::canonicalize(&absolute)
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned())
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "stc-host".to_owned())
}

#[cfg(unix)]
fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| {
            format!(
                "cannot create the private file `{}`: {error}",
                temporary.display()
            )
        })?;
    file.write_all(contents)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync `{}`: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "cannot install the private file `{}`: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn write_private_file(_path: &Path, _contents: &[u8]) -> Result<(), String> {
    Err("`stc serve` is supported on Unix hosts only".to_owned())
}

fn read_private_file(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "cannot inspect the private file `{}`: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "the private path `{}` is not a regular file",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "the private file `{}` is not user-only",
                path.display()
            ));
        }
    }
    if metadata.len() > NONCE_FILE_LIMIT {
        return Err(format!(
            "the private file `{}` is larger than {NONCE_FILE_LIMIT} bytes",
            path.display()
        ));
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read the private file `{}`: {error}", path.display()))?;
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("the private file `{}` is empty", path.display()));
    }
    Ok(text.to_owned())
}

fn cleanup(session: &Path, nonce: &Path) {
    let _ = fs::remove_file(session);
    let _ = fs::remove_file(nonce);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "stc-serve-unit-{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create temp directory");
        directory
    }

    #[test]
    fn output_paths_join_under_the_work_directory_and_reject_parent_components() {
        let work = Path::new("/home/r2k/stc-wl12");
        assert_eq!(
            output_path(work, None, "session.json").expect("default"),
            work.join("session.json")
        );
        assert_eq!(
            output_path(work, Some("debug.endpoint"), "unused").expect("relative"),
            work.join("debug.endpoint")
        );
        assert_eq!(
            output_path(work, Some("/tmp/endpoint"), "unused").expect("absolute"),
            PathBuf::from("/tmp/endpoint")
        );
        assert!(output_path(work, Some("../escape"), "unused").is_err());
    }

    #[test]
    fn canonical_string_resolves_relative_paths() {
        let directory = temp_dir("canonical");
        let file = directory.join("file.json");
        fs::write(&file, b"{}").expect("write file");
        let resolved = canonical_string(&file).expect("canonical");
        assert!(Path::new(&resolved).is_absolute(), "{resolved}");
        fs::remove_dir_all(&directory).expect("clean up");
    }

    #[test]
    fn host_name_is_never_empty() {
        assert!(!host_name().is_empty());
    }

    #[test]
    fn cleanup_removes_both_private_files() {
        let directory = temp_dir("cleanup");
        let session = directory.join("session.json");
        let nonce = directory.join("nonce");
        fs::write(&session, b"{}").expect("session");
        fs::write(&nonce, b"x").expect("nonce");
        cleanup(&session, &nonce);
        assert!(!session.exists());
        assert!(!nonce.exists());
        fs::remove_dir_all(&directory).expect("clean up");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn private_files_round_trip_and_reject_group_access() {
        use std::os::unix::fs::PermissionsExt;
        let directory = temp_dir("private");
        let path = directory.join("nonce");
        write_private_file(&path, b"abc123\n").expect("write private");
        assert_eq!(read_private_file(&path).expect("read private"), "abc123");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("mode");
        assert!(read_private_file(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
        fs::write(&path, b"   \n").expect("empty");
        assert!(read_private_file(&path).is_err());
        let missing = directory.join("missing");
        assert!(read_private_file(&missing).is_err());
        assert!(read_private_file(&directory).is_err());
        fs::remove_dir_all(&directory).expect("clean up");
    }
}
