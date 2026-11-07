//! Local orchestration for remote debug sessions (WL-1.2).
//!
//! A remote launch mirrors the project graph to the host over SFTP, verifies
//! every uploaded file with SHA-256, starts `stc serve` detached, reads the
//! user-only `stc-remote/1` session descriptor and endpoint file, opens an SSH
//! local port-forward to the loopback debug socket, and connects the ordinary
//! `stc-debug/1` client through that tunnel. A remote attach does the same
//! without syncing or starting a host. The DAP adapter never speaks SSH.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use stc_dap::{LaunchArguments, LaunchedSession, OutputLine, SessionHost};
use stc_diag::Severity;
use stc_got::error::{Error, codes};
use stc_runtime::TransportKind;
use stc_runtime::debug_server::{DebugClient, DebugEndpointFile};

use crate::cli::Invocation;
use crate::report::Reporter;

use crate::remote::{
    RemoteDeploy, RemotePath, RemoteProgram, RemoteSession, RemoteSettings, RemoteTarget,
    SshRunner, SshTunnel, SyncFile, UPLOAD_TIMEOUT, compare_source_fingerprints,
    project_sync_files, remote_error, shell_quote,
};
use crate::sha256;

/// Budget for the remote build and for the serve endpoint to appear.
const BUILD_DEADLINE: Duration = Duration::from_secs(600);
/// Budget for one source-mirror upload and its SHA-256 verification.
const SYNC_DEADLINE: Duration = Duration::from_secs(600);
/// One remote wait slice; the local loop repeats it until [`BUILD_DEADLINE`].
const WAIT_SLICE_TIMEOUT: Duration = Duration::from_secs(40);
/// Deadline for a best-effort remote stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Name of the source mirror under the remote work directory.
const SOURCE_DIR: &str = "src";
/// Name of the remote build output directory.
const BUILD_DIR: &str = "build";
/// Name of the remote candidate build directory (attach-time deployment).
const CANDIDATE_DIR: &str = "candidate";
/// Remote serve log, diagnostics only.
const SERVE_LOG: &str = "stc-serve.log";
/// Remote serve exit-status marker used while waiting for the descriptor.
const SERVE_STATUS: &str = "stc-serve.status";
/// Retained-state file shared by every remote serve start in the work dir.
const STATE_FILE: &str = "stc-state.bin";
/// Stable copy of the active GOT manifest used as a candidate baseline.
const BASELINE_FILE: &str = "stc-baseline.stc-got.json";

/// A remote session connected through an SSH tunnel.
pub(crate) struct RemoteConnection {
    /// Connected debug client through the local tunnel endpoint.
    pub(crate) client: DebugClient,
    /// Tunnel and remote serve ownership.
    pub(crate) host: RemoteHost,
}

impl RemoteConnection {
    /// Wraps the connection as a DAP launched session.
    #[must_use]
    pub(crate) fn into_launched_session(self) -> LaunchedSession {
        LaunchedSession {
            client: self.client,
            host: Some(Box::new(self.host)),
        }
    }
}

/// A validated remote location plus its tool path.
struct ResolvedRemote {
    settings: RemoteSettings,
    work_dir: RemotePath,
    session: RemotePath,
    stc: RemoteProgram,
    source_dir: RemotePath,
    build_dir: RemotePath,
    candidate_dir: RemotePath,
    serve_log: RemotePath,
    serve_status: RemotePath,
    state_file: RemotePath,
    baseline_file: RemotePath,
}

impl ResolvedRemote {
    fn resolve(settings: RemoteSettings, runner: &SshRunner) -> Result<Self, Error> {
        let home = runner.remote_home(&settings.target)?;
        let work_dir = settings.work_dir.resolve_home(&home)?;
        let session = settings.session.resolve_home(&home)?;
        if work_dir.as_str() == "/" || work_dir == home {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                "the remote work directory must be a dedicated directory, not `/` or the login home",
            ));
        }
        if !session.is_within(&work_dir) {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote session file `{}` must be under the work directory `{}`",
                    session.as_str(),
                    work_dir.as_str()
                ),
            ));
        }
        let stc = settings.stc.resolve_home(&home)?;
        Ok(Self {
            source_dir: work_dir.join(SOURCE_DIR)?,
            build_dir: work_dir.join(BUILD_DIR)?,
            candidate_dir: work_dir.join(CANDIDATE_DIR)?,
            serve_log: work_dir.join(SERVE_LOG)?,
            serve_status: work_dir.join(SERVE_STATUS)?,
            state_file: work_dir.join(STATE_FILE)?,
            baseline_file: work_dir.join(BASELINE_FILE)?,
            work_dir,
            session,
            stc,
            settings,
        })
    }

    fn target(&self) -> &RemoteTarget {
        &self.settings.target
    }
}

/// Runs `stc remote status`.
pub(crate) fn status_command(invocation: &Invocation, reporter: &mut Reporter) -> u8 {
    let report = match status_report(invocation) {
        Ok(report) => report,
        Err(message) => {
            report_remote_error(reporter, &message);
            return 2;
        }
    };
    if invocation.format == crate::report::Format::Json {
        println!("{}", report.render_compact());
    } else {
        let string = |key: &str| report.as_object().and_then(|object| object.get(key));
        let value = |key: &str| {
            string(key)
                .and_then(stc_got::Json::as_str)
                .unwrap_or("-")
                .to_owned()
        };
        let flag = |key: &str| {
            string(key)
                .and_then(stc_got::Json::as_bool)
                .is_some_and(|value| value)
        };
        println!("remote {} drift {}", value("host"), flag("drift"));
        println!(
            "local {} remote {}",
            value("local_fingerprint"),
            value("remote_fingerprint")
        );
        println!(
            "deploy {}",
            if flag("drift") {
                if flag("online") {
                    "online or cold"
                } else {
                    "cold only"
                }
            } else {
                "none (sources match)"
            }
        );
        if let Some(reason) = string("reason").and_then(stc_got::Json::as_str) {
            println!("reason {reason}");
        }
    }
    0
}

fn status_report(invocation: &Invocation) -> Result<stc_got::Json, String> {
    status_report_with(invocation, &SshRunner::default())
}

fn status_report_with(
    invocation: &Invocation,
    runner: &SshRunner,
) -> Result<stc_got::Json, String> {
    let Some(target) = invocation.remote.remote.as_deref() else {
        return Err("`stc remote status` requires `--remote <user@host>`".to_owned());
    };
    let settings = RemoteSettings::from_strings(
        target,
        invocation.remote.remote_dir.as_deref(),
        invocation.remote.remote_stc.as_deref(),
        invocation.remote.remote_session.as_deref(),
        invocation.remote.remote_deploy.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let remote = ResolvedRemote::resolve(settings, runner).map_err(|error| error.to_string())?;
    check_remote_version(runner, &remote)?;
    let project = Path::new(invocation.manifest.as_deref().unwrap_or("stc.toml"));
    let local =
        crate::remote::local_source_fingerprint(project).map_err(|error| error.to_string())?;
    let (session, _) = read_remote_session(runner, &remote)?;
    let drift = session.source_fingerprint != local;
    let mut online = true;
    let mut reason: Option<String> = None;
    if drift {
        match project_sync_files(project) {
            Ok(sync) => match sync_project(runner, &remote, &sync.files) {
                Ok(()) => {
                    let baseline = read_active_manifest(runner, &remote, &session).ok();
                    match build_candidate(
                        runner,
                        &remote,
                        &sync,
                        session.mode == "release",
                        baseline.as_ref(),
                    ) {
                        Ok(candidate) => {
                            if let Err(error) =
                                compare_source_fingerprints(&local, &candidate.fingerprint)
                            {
                                online = false;
                                reason = Some(error.to_string());
                            } else {
                                match baseline {
                                    Some(baseline) => {
                                        let (possible, why) =
                                            online_possible(&candidate.manifest_data, &baseline);
                                        online = possible;
                                        reason = why;
                                    }
                                    None => {
                                        online = false;
                                        reason = Some(
                                            "the remote host has no active baseline; a cold download is required"
                                                .to_owned(),
                                        );
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            online = false;
                            reason = Some(error);
                        }
                    }
                }
                Err(error) => {
                    online = false;
                    reason = Some(error);
                }
            },
            Err(error) => {
                online = false;
                reason = Some(error.to_string());
            }
        }
    }
    Ok(stc_got::Json::object([
        ("build_id".into(), stc_got::Json::string(session.build_id)),
        ("cold_required".into(), stc_got::Json::Bool(!online)),
        ("drift".into(), stc_got::Json::Bool(drift)),
        (
            "generation".into(),
            stc_got::Json::unsigned(session.generation),
        ),
        ("host".into(), stc_got::Json::string(session.host)),
        ("local_fingerprint".into(), stc_got::Json::string(local)),
        ("mode".into(), stc_got::Json::string(session.mode)),
        ("online".into(), stc_got::Json::Bool(online)),
        (
            "protocol".into(),
            stc_got::Json::string("stc-remote-status/1"),
        ),
        (
            "reason".into(),
            stc_got::Json::optional(reason.map(stc_got::Json::string)),
        ),
        (
            "remote_fingerprint".into(),
            stc_got::Json::string(session.source_fingerprint),
        ),
    ]))
}

fn report_remote_error(reporter: &mut Reporter, message: &str) {
    if let Some((code, detail)) = message
        .split_once(": ")
        .filter(|(code, _)| code.starts_with("STC-"))
    {
        reporter.plain(code, Severity::Error, detail);
    } else {
        reporter.plain(codes::REMOTE_TRANSPORT, Severity::Error, message);
    }
}

/// Launches a remote host: sync, build, serve, tunnel, and connect.
///
/// # Errors
///
/// Returns a readable `STC-E-REMOTE-*` diagnostic when any remote step fails.
pub(crate) fn launch(
    arguments: &LaunchArguments,
    manifest_fallback: Option<&Path>,
) -> Result<RemoteConnection, String> {
    if arguments.test.is_some() {
        return Err("STC-E-REMOTE-007: remote launches do not support `test` sessions".to_owned());
    }
    let settings = settings(arguments)?;
    let runner = SshRunner::default();
    let remote = ResolvedRemote::resolve(settings, &runner).map_err(|error| error.to_string())?;
    check_remote_version(&runner, &remote)?;
    let project = arguments.project_path(manifest_fallback);
    let sync = project_sync_files(&project).map_err(|error| error.to_string())?;
    sync_project(&runner, &remote, &sync.files)?;
    prepare_remote_session(&runner, &remote)?;
    start_remote_serve(
        &runner,
        &remote,
        &ServeStart {
            root_manifest: &sync.root_manifest,
            out_dir: &remote.build_dir,
            start_paused: true,
            no_build: arguments.no_build,
            release: arguments.release,
            allow_write: arguments.allow_write,
            allow_force: arguments.allow_force,
        },
    )?;
    let (session, endpoint) = wait_for_remote_session(&runner, &remote)?;
    finish_launch(&runner, &remote, session, endpoint, &sync.fingerprint)
}

/// Attaches to an existing remote serve through a tunnel.
///
/// Phase 3 supports only an already-matching source fingerprint; the
/// attach-time online/cold/cancel decision is added by the next phase.
///
/// # Errors
///
/// Returns a readable `STC-E-REMOTE-*` diagnostic when the descriptor cannot be
/// read, the source fingerprint differs, or the tunnel cannot connect.
pub(crate) fn attach(
    arguments: &LaunchArguments,
    manifest_fallback: Option<&Path>,
) -> Result<RemoteConnection, String> {
    let settings = settings(arguments)?;
    let runner = SshRunner::default();
    let remote = ResolvedRemote::resolve(settings, &runner).map_err(|error| error.to_string())?;
    check_remote_version(&runner, &remote)?;
    let project = arguments.project_path(manifest_fallback);
    let local_fingerprint =
        crate::remote::local_source_fingerprint(&project).map_err(|error| error.to_string())?;
    let (mut session, mut endpoint) = read_remote_session(&runner, &remote)?;
    if compare_source_fingerprints(&local_fingerprint, &session.source_fingerprint).is_err() {
        match remote.settings.deploy {
            RemoteDeploy::Never => {
                return Err(remote_error(
                    codes::REMOTE_SOURCE_MISMATCH,
                    "the remote source fingerprint differs and `remoteDeploy` is `never`",
                )
                .to_string());
            }
            RemoteDeploy::Ask => {
                return Err(remote_error(
                    codes::REMOTE_SOURCE_MISMATCH,
                    "the remote source fingerprint differs; choose `remoteDeploy` `online` or `cold`",
                )
                .to_string());
            }
            RemoteDeploy::Online | RemoteDeploy::Cold => {
                let sync = project_sync_files(&project).map_err(|error| error.to_string())?;
                sync_project(&runner, &remote, &sync.files)?;
                let deployed = deploy_remote(
                    &runner,
                    &remote,
                    &session,
                    &sync,
                    remote.settings.deploy,
                    arguments.allow_write,
                    arguments.allow_force,
                )?;
                session = deployed.0;
                endpoint = deployed.1;
            }
        }
    }
    compare_source_fingerprints(&local_fingerprint, &session.source_fingerprint)
        .map_err(|error| error.to_string())?;
    let gate = session.start_gate.clone();
    let mut connection = connect_tunnel(&runner, &remote, &endpoint, true)?;
    connection.host.gate = gate;
    Ok(connection)
}

fn settings(arguments: &LaunchArguments) -> Result<RemoteSettings, String> {
    let Some(target) = arguments.remote.as_deref() else {
        return Err("a remote launch requires the `remote` setting".into());
    };
    RemoteSettings::from_strings(
        target,
        arguments.remote_dir.as_deref(),
        arguments.remote_stc.as_deref(),
        arguments.remote_session.as_deref(),
        arguments.remote_deploy.as_deref(),
    )
    .map_err(|error| error.to_string())
}

fn check_remote_version(runner: &SshRunner, remote: &ResolvedRemote) -> Result<(), String> {
    let argv = vec![remote.stc.as_str().to_owned(), "--version".to_owned()];
    let output = runner
        .run(remote.target(), &argv, UPLOAD_TIMEOUT)
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "cannot run `{} --version` on `{}`: {}",
                remote.stc.as_str(),
                remote.target().spelling(),
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    let version = output.stdout.trim().strip_prefix("stc ").ok_or_else(|| {
        remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "the remote `stc --version` did not report an stc version: {}",
                output.stdout.trim()
            ),
        )
        .to_string()
    })?;
    if version != env!("CARGO_PKG_VERSION") {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "the remote stc version {version} does not match the local driver {}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .to_string());
    }
    Ok(())
}

/// One remote candidate build ready for deployment.
struct Candidate {
    module: RemotePath,
    /// Binary GOT manifest passed to the control deploy.
    deploy_manifest: RemotePath,
    manifest_data: stc_got::Manifest,
    fingerprint: String,
}

/// Reads a non-secret remote text file.
fn remote_text(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    path: &RemotePath,
) -> Result<String, String> {
    let argv = vec!["cat".to_owned(), "--".to_owned(), path.as_str().to_owned()];
    let output = runner
        .run(remote.target(), &argv, SYNC_DEADLINE)
        .map_err(|error| error.to_string())?;
    if output.success() {
        Ok(output.stdout)
    } else {
        Err(remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "cannot read the remote file `{}`: {}",
                path.as_str(),
                output.stderr.trim()
            ),
        )
        .to_string())
    }
}

/// Writes a non-secret remote text file through the command's standard input.
fn write_remote_file(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    path: &RemotePath,
    contents: &str,
) -> Result<(), String> {
    let script = format!("set -eu\ncat > {}", shell_quote(path.as_str()));
    let output = runner
        .run_with_stdin(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            contents.as_bytes(),
            STOP_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if output.success() {
        Ok(())
    } else {
        Err(remote_error(
            codes::REMOTE_BUILD,
            format!(
                "cannot write the remote baseline `{}`: {}",
                path.as_str(),
                output.stderr.trim()
            ),
        )
        .to_string())
    }
}

fn reset_remote_dir(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    directory: &RemotePath,
) -> Result<(), String> {
    let script = format!(
        "set -eu\nrm -rf -- {directory}\nmkdir -p -- {directory}",
        directory = shell_quote(directory.as_str())
    );
    let output = runner
        .run(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            SYNC_DEADLINE,
        )
        .map_err(|error| error.to_string())?;
    if output.success() {
        Ok(())
    } else {
        Err(remote_error(
            codes::REMOTE_TRANSPORT,
            format!(
                "cannot prepare the remote directory `{}`: {}",
                directory.as_str(),
                output.stderr.trim()
            ),
        )
        .to_string())
    }
}

/// Builds the synced mirror into the remote candidate directory.
fn build_candidate(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    sync: &crate::remote::ProjectSync,
    release: bool,
    baseline: Option<&stc_got::Manifest>,
) -> Result<Candidate, String> {
    let baseline_path = match baseline {
        Some(manifest) => {
            let text = manifest.to_json_text().map_err(|error| {
                remote_error(
                    codes::REMOTE_BUILD,
                    format!("cannot serialize the active baseline: {}", error.message),
                )
                .to_string()
            })?;
            write_remote_file(runner, remote, &remote.baseline_file, &text)?;
            Some(remote.baseline_file.clone())
        }
        None => None,
    };
    reset_remote_dir(runner, remote, &remote.candidate_dir)?;
    let manifest = remote
        .source_dir
        .join(&sync.root_manifest)
        .map_err(|error| error.to_string())?;
    let mut argv = vec![
        remote.stc.as_str().to_owned(),
        "build".to_owned(),
        "--manifest-path".to_owned(),
        manifest.as_str().to_owned(),
        "--out-dir".to_owned(),
        remote.candidate_dir.as_str().to_owned(),
        "--emit".to_owned(),
        // The candidate must include the thin launcher `stc serve` starts
        // from after a cold restart, plus the module it loads.
        "module,exe".to_owned(),
        "--message-format".to_owned(),
        "json".to_owned(),
    ];
    if release {
        argv.push("--release".to_owned());
    }
    if let Some(baseline) = &baseline_path {
        argv.push("--got-manifest".to_owned());
        argv.push(baseline.as_str().to_owned());
    }
    let output = runner
        .run(remote.target(), &argv, SYNC_DEADLINE)
        .map_err(|error| error.to_string())?;
    if !output.success() {
        let detail = tail_text(&output.stdout, &output.stderr);
        return Err(remote_error(
            codes::REMOTE_BUILD,
            format!("the remote candidate build failed{detail}"),
        )
        .to_string());
    }
    let metadata = remote
        .candidate_dir
        .join(&format!("{}.stc-meta.json", sync.package_name))
        .map_err(|error| error.to_string())?;
    let metadata = remote_text(runner, remote, &metadata)?;
    let value = stc_got::json::parse(&metadata).map_err(|error| {
        remote_error(
            codes::REMOTE_BUILD,
            format!(
                "the remote candidate metadata is not valid JSON: {}",
                error.message
            ),
        )
        .to_string()
    })?;
    let module_file = value
        .as_object()
        .and_then(|object| object.get("module"))
        .and_then(stc_got::Json::as_object)
        .and_then(|object| object.get("file"))
        .and_then(stc_got::Json::as_str)
        .ok_or_else(|| {
            remote_error(
                codes::REMOTE_BUILD,
                "the remote candidate metadata has no module file",
            )
            .to_string()
        })?;
    let module = remote
        .candidate_dir
        .join(module_file)
        .map_err(|error| error.to_string())?;
    let deploy_manifest = remote
        .candidate_dir
        .join(&format!("{}.stc-got.bin", sync.package_name))
        .map_err(|error| error.to_string())?;
    let manifest = remote
        .candidate_dir
        .join(&format!("{}.stc-got.json", sync.package_name))
        .map_err(|error| error.to_string())?;
    let text = remote_text(runner, remote, &manifest)?;
    let manifest_data = stc_got::Manifest::decode_json(&text).map_err(|error| {
        remote_error(
            codes::REMOTE_BUILD,
            format!(
                "the remote candidate manifest is invalid: {}",
                error.message
            ),
        )
        .to_string()
    })?;
    let fingerprint = manifest_data.header.source_fingerprint.to_hex();
    Ok(Candidate {
        module,
        deploy_manifest,
        manifest_data,
        fingerprint,
    })
}

fn tail_text(stdout: &str, stderr: &str) -> String {
    let mut detail = String::new();
    for text in [stderr, stdout] {
        let text = text.trim();
        if !text.is_empty() {
            if !detail.is_empty() {
                detail.push('\n');
            }
            let tail = text
                .char_indices()
                .rev()
                .nth(2000)
                .map_or(text, |(index, _)| &text[index..]);
            detail.push_str(tail);
        }
    }
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

fn read_active_manifest(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    session: &RemoteSession,
) -> Result<stc_got::Manifest, String> {
    let mut paths = Vec::new();
    if let Some(stem) = session.manifest.as_str().strip_suffix(".stc-got.bin")
        && let Ok(json) = RemotePath::parse_absolute(&format!("{stem}.stc-got.json"))
    {
        // Older descriptors (and `serve deploy` callers that pass only the
        // binary) name the binary manifest; the build emits the JSON sibling.
        paths.push(json);
    }
    paths.push(session.manifest.clone());
    let mut last_error = None;
    for path in paths {
        let Ok(text) = remote_text(runner, remote, &path) else {
            last_error = Some(format!(
                "cannot read the active remote baseline `{}`",
                path.as_str()
            ));
            continue;
        };
        match stc_got::Manifest::decode_json(&text) {
            Ok(manifest) => return Ok(manifest),
            Err(error) => {
                last_error = Some(format!(
                    "cannot read the active remote baseline `{}`: {}",
                    path.as_str(),
                    error.message
                ));
            }
        }
    }
    Err(remote_error(
        codes::REMOTE_DEPLOY,
        last_error.unwrap_or_else(|| "cannot read the active remote baseline".to_owned()),
    )
    .to_string())
}

fn online_possible(
    candidate: &stc_got::Manifest,
    baseline: &stc_got::Manifest,
) -> (bool, Option<String>) {
    match stc_runtime::compare_manifests(candidate, Some(baseline)) {
        Ok(diff)
            if diff.decision == Some(stc_runtime::PlanDecision::Online) && diff.error.is_none() =>
        {
            (true, None)
        }
        Ok(diff) => {
            let reason = diff.error.map_or_else(
                || format!("the online-change plan verdict is {:?}", diff.verdict),
                |error| format!("{}: {}", error.code, error.message),
            );
            (false, Some(reason))
        }
        Err(error) => (false, Some(format!("{}: {}", error.code, error.message))),
    }
}

/// Deploys the synced mirror and returns the refreshed session and endpoint.
fn deploy_remote(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    session: &RemoteSession,
    sync: &crate::remote::ProjectSync,
    choice: crate::remote::RemoteDeploy,
    allow_write: bool,
    allow_force: bool,
) -> Result<(RemoteSession, DebugEndpointFile), String> {
    let release = session.mode == "release";
    let baseline = read_active_manifest(runner, remote, session).ok();
    let candidate = build_candidate(runner, remote, sync, release, baseline.as_ref())?;
    compare_source_fingerprints(&sync.fingerprint, &candidate.fingerprint)
        .map_err(|error| error.to_string())?;
    match choice {
        crate::remote::RemoteDeploy::Online => {
            let Some(baseline) = baseline else {
                return Err(remote_error(
                    codes::REMOTE_DEPLOY,
                    "the remote host has no active baseline; a cold download is required",
                )
                .to_string());
            };
            let (online, reason) = online_possible(&candidate.manifest_data, &baseline);
            if !online {
                return Err(remote_error(
                    codes::REMOTE_DEPLOY,
                    format!(
                        "the remote host requires a cold download: {}",
                        reason.unwrap_or_else(|| "the online plan was rejected".to_owned())
                    ),
                )
                .to_string());
            }
            let argv = vec![
                remote.stc.as_str().to_owned(),
                "serve".to_owned(),
                "deploy".to_owned(),
                "--session".to_owned(),
                remote.session.as_str().to_owned(),
                "--module".to_owned(),
                candidate.module.as_str().to_owned(),
                "--deploy-manifest".to_owned(),
                candidate.deploy_manifest.as_str().to_owned(),
                "--message-format".to_owned(),
                "json".to_owned(),
            ];
            let output = runner
                .run(remote.target(), &argv, SYNC_DEADLINE)
                .map_err(|error| error.to_string())?;
            if !output.success() {
                let detail = tail_text(&output.stdout, &output.stderr);
                return Err(remote_error(
                    codes::REMOTE_DEPLOY,
                    format!("the remote online deploy was rejected{detail}"),
                )
                .to_string());
            }
            if output.stdout.contains("\"cold_required\":true") {
                return Err(remote_error(
                    codes::REMOTE_DEPLOY,
                    "the remote host requires a cold download",
                )
                .to_string());
            }
            read_remote_session(runner, remote)
        }
        crate::remote::RemoteDeploy::Cold => {
            prepare_remote_session(runner, remote)?;
            start_remote_serve(
                runner,
                remote,
                &ServeStart {
                    root_manifest: &sync.root_manifest,
                    out_dir: &remote.candidate_dir,
                    start_paused: false,
                    no_build: true,
                    release,
                    allow_write,
                    allow_force,
                },
            )?;
            let (session, endpoint) = wait_for_remote_session(runner, remote)?;
            compare_source_fingerprints(&sync.fingerprint, &session.source_fingerprint)
                .map_err(|error| error.to_string())?;
            Ok((session, endpoint))
        }
        crate::remote::RemoteDeploy::Never | crate::remote::RemoteDeploy::Ask => Err(remote_error(
            codes::REMOTE_DEPLOY,
            "the remote source fingerprint differs and deployment is disabled",
        )
        .to_string()),
    }
}

/// Resets the remote source mirror and uploads every file, verifying hashes.
fn sync_project(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    files: &[SyncFile],
) -> Result<(), String> {
    let reset = format!(
        "set -eu\nrm -rf -- {root}\nmkdir -p -- {root}",
        root = shell_quote(remote.source_dir.as_str())
    );
    let output = runner
        .run(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), reset],
            WAIT_SLICE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_TRANSPORT,
            format!(
                "cannot reset the remote source mirror `{}`: {}",
                remote.source_dir.as_str(),
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    let mut expected = BTreeMap::new();
    for file in files {
        let bytes = fs::read(&file.local).map_err(|error| {
            format!(
                "cannot read the local source `{}`: {error}",
                file.local.display()
            )
        })?;
        expected.insert(file.relative.clone(), sha256::digest(&bytes));
    }
    let full_batch = sftp_batch_text(files, &remote.source_dir)?;
    let output = runner
        .sftp_batch(remote.target(), &full_batch, SYNC_DEADLINE)
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_TRANSPORT,
            format!(
                "cannot upload the project mirror to `{}`: {}",
                remote.source_dir.as_str(),
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    verify_hashes(runner, remote, files, &expected)
}

/// Builds the SFTP batch for one project mirror.
fn sftp_batch_text(files: &[SyncFile], source_dir: &RemotePath) -> Result<String, String> {
    let mut directories = BTreeMap::new();
    let mut batch = String::new();
    for file in files {
        let destination = source_dir
            .join(&file.relative)
            .map_err(|error| error.to_string())?;
        if !destination.is_shell_safe() {
            return Err(remote_error(
                codes::REMOTE_SESSION,
                format!(
                    "the remote mirror path `{}` contains unsupported characters",
                    destination.as_str()
                ),
            )
            .to_string());
        }
        if let Some((parent, _)) = file.relative.rsplit_once('/') {
            let mut prefix = String::new();
            for part in parent.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                let directory = source_dir
                    .join(&prefix)
                    .map_err(|error| error.to_string())?;
                directories.insert(directory.as_str().to_owned(), ());
            }
        }
        batch.push_str(&format!(
            "put {} {}\n",
            sftp_quote(&local_sftp_path(&file.local)),
            sftp_quote(destination.as_str())
        ));
    }
    let mut full_batch = String::new();
    for directory in directories.keys() {
        full_batch.push_str(&format!("-mkdir {}\n", sftp_quote(directory)));
    }
    full_batch.push_str(&batch);
    Ok(full_batch)
}

fn verify_hashes(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    files: &[SyncFile],
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    let script = format!(
        "set -eu\ncd {}\nwhile IFS= read -r f; do sha256sum -- \"$f\"; done",
        shell_quote(remote.source_dir.as_str())
    );
    let mut input = String::new();
    for file in files {
        input.push_str(&file.relative);
        input.push('\n');
    }
    let output = runner
        .run_with_stdin(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            input.as_bytes(),
            SYNC_DEADLINE,
        )
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "cannot verify the remote source mirror: {}",
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    let mut seen = BTreeMap::new();
    for line in output.stdout.lines() {
        let Some((digest, path)) = line.split_once(' ') else {
            continue;
        };
        let path = path.trim_start_matches(' ');
        seen.insert(path.to_owned(), digest.to_owned());
    }
    for (relative, digest) in expected {
        match seen.get(relative) {
            Some(actual) if actual == digest => {}
            Some(actual) => {
                return Err(remote_error(
                    codes::REMOTE_SOURCE_MISMATCH,
                    format!(
                        "the uploaded file `{relative}` has SHA-256 {actual}, expected {digest}"
                    ),
                )
                .to_string());
            }
            None => {
                return Err(remote_error(
                    codes::REMOTE_SOURCE_MISMATCH,
                    format!("the uploaded file `{relative}` was not reported by the host"),
                )
                .to_string());
            }
        }
    }
    Ok(())
}

/// Converts a local path to the forward-slash form OpenSSH `sftp` accepts.
fn local_sftp_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Quotes one `sftp` batch path.
fn sftp_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        if character == '"' || character == '\\' {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Stops a previous serve session and clears its descriptor/output files.
fn prepare_remote_session(runner: &SshRunner, remote: &ResolvedRemote) -> Result<(), String> {
    let argv = vec![
        remote.stc.as_str().to_owned(),
        "serve".to_owned(),
        "stop".to_owned(),
        "--session".to_owned(),
        remote.session.as_str().to_owned(),
    ];
    // A missing or stale session is expected; the stop result is advisory.
    let _ = runner.run(remote.target(), &argv, STOP_TIMEOUT);
    let script = format!(
        "set -eu\nrm -f -- {} {}",
        shell_quote(remote.session.as_str()),
        shell_quote(remote.serve_status.as_str())
    );
    let output = runner
        .run(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            WAIT_SLICE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_SESSION,
            format!(
                "cannot clear the previous remote session: {}",
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    Ok(())
}

/// One remote `stc serve start` request.
struct ServeStart<'a> {
    root_manifest: &'a str,
    out_dir: &'a RemotePath,
    start_paused: bool,
    no_build: bool,
    release: bool,
    allow_write: bool,
    allow_force: bool,
}

fn start_remote_serve(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    start: &ServeStart<'_>,
) -> Result<(), String> {
    let manifest = remote
        .source_dir
        .join(start.root_manifest)
        .map_err(|error| error.to_string())?;
    let mut command = format!(
        "{} serve start --manifest-path {} --work-dir {} --session {} --out-dir {} --message-format json",
        shell_quote(remote.stc.as_str()),
        shell_quote(manifest.as_str()),
        shell_quote(remote.work_dir.as_str()),
        shell_quote(remote.session.as_str()),
        shell_quote(start.out_dir.as_str())
    );
    command.push_str(" --state-file ");
    command.push_str(&shell_quote(remote.state_file.as_str()));
    if start.start_paused {
        command.push_str(" --start-paused");
    }
    if start.release {
        command.push_str(" --release");
    }
    if start.no_build {
        command.push_str(" --no-build");
    }
    if start.allow_write {
        command.push_str(" --allow-write");
    }
    if start.allow_force {
        command.push_str(" --allow-force");
    }
    let script = format!(
        "set -eu\nrm -f -- {log} {status}\nnohup sh -c {inner} </dev/null >/dev/null 2>&1 &\nexit 0",
        log = shell_quote(remote.serve_log.as_str()),
        status = shell_quote(remote.serve_status.as_str()),
        inner = shell_quote(&format!(
            "{command} > {} 2>&1; echo $? > {}",
            shell_quote(remote.serve_log.as_str()),
            shell_quote(remote.serve_status.as_str())
        ))
    );
    let output = runner
        .run(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            WAIT_SLICE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if !output.success() {
        return Err(remote_error(
            codes::REMOTE_BUILD,
            format!(
                "cannot start `stc serve` on `{}`: {}",
                remote.target().spelling(),
                output.stderr.trim()
            ),
        )
        .to_string());
    }
    Ok(())
}

fn wait_for_remote_session(
    runner: &SshRunner,
    remote: &ResolvedRemote,
) -> Result<(RemoteSession, DebugEndpointFile), String> {
    let script = format!(
        "i=0\nwhile [ ! -f {session} ] && [ ! -f {status} ]; do\n  i=$((i+1))\n  if [ \"$i\" -gt 300 ]; then exit 2; fi\n  sleep 0.1\ndone\n[ -f {session} ] && exit 0\nexit 1",
        session = shell_quote(remote.session.as_str()),
        status = shell_quote(remote.serve_status.as_str())
    );
    let deadline = Instant::now() + BUILD_DEADLINE;
    loop {
        let output = runner
            .run(
                remote.target(),
                &["sh".to_owned(), "-c".to_owned(), script.clone()],
                WAIT_SLICE_TIMEOUT,
            )
            .map_err(|error| error.to_string())?;
        match output.status {
            Some(0) => break,
            Some(2) if Instant::now() < deadline => continue,
            Some(1) | Some(2) => return Err(serve_failure(runner, remote)),
            _ => return Err(serve_failure(runner, remote)),
        }
    }
    read_remote_session(runner, remote)
}

fn serve_failure(runner: &SshRunner, remote: &ResolvedRemote) -> String {
    let script = format!(
        "set -eu\ntail -n 40 -- {} || true",
        shell_quote(remote.serve_log.as_str())
    );
    let detail = runner
        .run(
            remote.target(),
            &["sh".to_owned(), "-c".to_owned(), script],
            STOP_TIMEOUT,
        )
        .ok()
        .map(|output| output.stdout.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(|text| format!(": {text}"))
        .unwrap_or_default();
    remote_error(
        codes::REMOTE_BUILD,
        format!("the remote serve host did not publish its session descriptor{detail}"),
    )
    .to_string()
}

fn read_remote_session(
    runner: &SshRunner,
    remote: &ResolvedRemote,
) -> Result<(RemoteSession, DebugEndpointFile), String> {
    let text = runner
        .read_private_file(remote.target(), &remote.session)
        .map_err(|error| error.to_string())?;
    let session = RemoteSession::parse(&text)
        .map_err(|error| format!("{}: {error}", codes::REMOTE_SESSION))?;
    if session.stc_version != env!("CARGO_PKG_VERSION") {
        return Err(remote_error(
            codes::REMOTE_COMMAND,
            format!(
                "the remote serve version {} does not match the local driver {}",
                session.stc_version,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .to_string());
    }
    let endpoint_text = runner
        .read_private_file(remote.target(), &session.debug_endpoint_file)
        .map_err(|error| error.to_string())?;
    let endpoint = DebugEndpointFile::parse(&endpoint_text).map_err(|error| {
        format!(
            "{}: cannot read the remote debug endpoint: {error}",
            codes::REMOTE_SESSION
        )
    })?;
    if endpoint.transport != TransportKind::Socket {
        return Err(remote_error(
            codes::REMOTE_UNSUPPORTED,
            "the remote debug endpoint is not a loopback socket",
        )
        .to_string());
    }
    if endpoint.build_id != session.build_id || endpoint.generation != session.generation {
        return Err(remote_error(
            codes::REMOTE_SESSION,
            "the remote endpoint and session descriptor identify different builds",
        )
        .to_string());
    }
    Ok((session, endpoint))
}

fn finish_launch(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    session: RemoteSession,
    endpoint: DebugEndpointFile,
    local_fingerprint: &str,
) -> Result<RemoteConnection, String> {
    compare_source_fingerprints(local_fingerprint, &session.source_fingerprint)
        .map_err(|error| error.to_string())?;
    let Some(gate) = session.start_gate.clone() else {
        return Err(remote_error(
            codes::REMOTE_SESSION,
            "the launch-owned serve session did not publish a start gate",
        )
        .to_string());
    };
    let mut connection = connect_tunnel(runner, remote, &endpoint, false)?;
    connection.host.gate = Some(gate);
    Ok(connection)
}

fn connect_tunnel(
    runner: &SshRunner,
    remote: &ResolvedRemote,
    endpoint: &DebugEndpointFile,
    attached: bool,
) -> Result<RemoteConnection, String> {
    let tunnel = runner
        .open_tunnel(remote.target(), &endpoint.endpoint)
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let client = loop {
        match DebugClient::connect_known(
            tunnel.local_endpoint(),
            &endpoint.token,
            TransportKind::Socket,
        ) {
            Ok(client) => break client,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                return Err(remote_error(
                    codes::REMOTE_TRANSPORT,
                    format!(
                        "cannot connect to the tunneled debug endpoint `{}`: {error}",
                        tunnel.local_endpoint()
                    ),
                )
                .to_string());
            }
        }
    };
    Ok(RemoteConnection {
        client,
        host: RemoteHost {
            runner: runner.clone(),
            target: remote.target().clone(),
            stc: remote.stc.clone(),
            session: remote.session.clone(),
            status: remote.serve_status.clone(),
            tunnel: Some(tunnel),
            gate: None,
            attached,
            stopped: false,
            exit_code: None,
        },
    })
}

/// Owns the tunnel and, for a launch, the remote serve process.
pub(crate) struct RemoteHost {
    runner: SshRunner,
    target: RemoteTarget,
    stc: RemoteProgram,
    session: RemotePath,
    status: RemotePath,
    tunnel: Option<SshTunnel>,
    gate: Option<RemotePath>,
    attached: bool,
    stopped: bool,
    exit_code: Option<i32>,
}

impl RemoteHost {
    /// Releases a start-paused remote host after the session is configured.
    ///
    /// # Errors
    ///
    /// Returns a readable message when `stc serve release` fails.
    pub(crate) fn release_start_gate(&mut self) -> Result<(), String> {
        let Some(gate) = self.gate.clone() else {
            return Ok(());
        };
        let argv = vec![
            self.stc.as_str().to_owned(),
            "serve".to_owned(),
            "release".to_owned(),
            "--session".to_owned(),
            self.session.as_str().to_owned(),
        ];
        let output = self
            .runner
            .run(&self.target, &argv, STOP_TIMEOUT)
            .map_err(|error| error.to_string())?;
        if !output.success() {
            return Err(remote_error(
                codes::REMOTE_COMMAND,
                format!(
                    "cannot release the remote start gate `{}`: {}",
                    gate.as_str(),
                    output.stderr.trim()
                ),
            )
            .to_string());
        }
        self.gate = None;
        Ok(())
    }

    /// Stops a launch-owned remote serve and closes the tunnel.
    pub(crate) fn stop(&mut self) {
        if !self.attached && !self.stopped {
            self.stopped = true;
            let argv = vec![
                self.stc.as_str().to_owned(),
                "serve".to_owned(),
                "stop".to_owned(),
                "--session".to_owned(),
                self.session.as_str().to_owned(),
            ];
            let _ = self.runner.run(&self.target, &argv, STOP_TIMEOUT);
            self.exit_code = self.read_remote_exit();
        }
        self.tunnel.take();
    }

    /// Reads the serve exit status after a stop request.
    fn read_remote_exit(&self) -> Option<i32> {
        let script = format!(
            "i=0\nwhile [ ! -f {status} ] && [ \"$i\" -lt 50 ]; do\n  i=$((i+1))\n  sleep 0.1\ndone\nif [ -f {status} ]; then cat -- {status}; fi",
            status = shell_quote(self.status.as_str())
        );
        let output = self
            .runner
            .run(
                &self.target,
                &["sh".to_owned(), "-c".to_owned(), script],
                STOP_TIMEOUT,
            )
            .ok()?;
        output.stdout.trim().parse().ok()
    }
}

impl SessionHost for RemoteHost {
    fn drain_output(&mut self) -> Vec<OutputLine> {
        Vec::new()
    }

    fn try_exit(&mut self) -> Option<i32> {
        self.exit_code
    }

    fn configuration_complete(&mut self) -> Result<(), String> {
        self.release_start_gate()
    }

    fn attached(&self) -> bool {
        self.attached
    }

    fn shutdown(&mut self) {
        self.stop();
    }
}

impl Drop for RemoteHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(local: &str, relative: &str) -> SyncFile {
        SyncFile {
            local: PathBuf::from(local),
            relative: relative.to_owned(),
        }
    }

    #[test]
    fn sftp_batch_creates_every_ancestor_before_put() {
        let source = RemotePath::parse("/home/r2k/stc-wl12/src").expect("source");
        let files = [
            file("C:\\work\\app\\stc.toml", "app/stc.toml"),
            file("C:\\work\\app\\src\\main.st", "app/src/main.st"),
            file("C:\\work\\lib\\stc.toml", "lib/stc.toml"),
        ];
        let batch = sftp_batch_text(&files, &source).expect("batch");
        let lines = batch.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "-mkdir \"/home/r2k/stc-wl12/src/app\"");
        assert_eq!(lines[1], "-mkdir \"/home/r2k/stc-wl12/src/app/src\"");
        assert_eq!(lines[2], "-mkdir \"/home/r2k/stc-wl12/src/lib\"");
        assert_eq!(
            lines[3],
            "put \"C:/work/app/stc.toml\" \"/home/r2k/stc-wl12/src/app/stc.toml\""
        );
        assert_eq!(
            lines[4],
            "put \"C:/work/app/src/main.st\" \"/home/r2k/stc-wl12/src/app/src/main.st\""
        );
        assert_eq!(
            lines[5],
            "put \"C:/work/lib/stc.toml\" \"/home/r2k/stc-wl12/src/lib/stc.toml\""
        );
    }

    #[test]
    fn sftp_batch_rejects_unsafe_remote_paths() {
        let source = RemotePath::parse("/home/r2k/stc-wl12/src").expect("source");
        let error = sftp_batch_text(&[file("a", "a b/c.st")], &source).expect_err("unsafe");
        assert!(error.contains("STC-E-REMOTE-005"), "{error}");
    }

    #[test]
    fn sftp_quoting_and_local_spelling_are_stable() {
        let quoted = sftp_quote("a \"b\" c\\d");
        assert_eq!(quoted, "\"a \\\"b\\\" c\\\\d\"");
        assert_eq!(local_sftp_path(Path::new("C:\\a b\\c.st")), "C:/a b/c.st");
        assert_eq!(local_sftp_path(Path::new("src/c.st")), "src/c.st");
    }

    #[test]
    fn launch_rejects_remote_test_sessions_and_missing_targets() {
        let mut arguments = LaunchArguments {
            test: Some("suite".into()),
            remote: Some("r2k@ecat".into()),
            ..LaunchArguments::default()
        };
        let error = launch(&arguments, None).err().expect("test");
        assert!(error.contains("STC-E-REMOTE-007"), "{error}");
        arguments.test = None;
        arguments.remote = None;
        let error = launch(&arguments, None).err().expect("no target");
        assert!(error.contains("requires the `remote` setting"), "{error}");
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
            std::fs::create_dir_all(directory).expect("script directory");
            let path = directory.join(name);
            std::fs::write(&path, body).expect("write script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
            path
        }

        fn project(root: &Path) -> PathBuf {
            std::fs::create_dir_all(root.join("src")).expect("project");
            std::fs::write(
                root.join("stc.toml"),
                "schema = 1\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\
                 language = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n",
            )
            .expect("manifest");
            std::fs::write(root.join("src/main.st"), "PROGRAM Main\n  ;\nEND_PROGRAM\n")
                .expect("source");
            root.join("stc.toml")
        }

        fn tools(root: &Path, fail_serve: bool) -> (SshRunner, PathBuf, PathBuf) {
            tools_with(root, fail_serve, false)
        }

        fn tools_with(
            root: &Path,
            fail_serve: bool,
            fail_build: bool,
        ) -> (SshRunner, PathBuf, PathBuf) {
            let bin = root.join("bin");
            let ssh = script(
                &bin,
                "ssh",
                "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
            );
            script(
                &bin,
                "sftp",
                "#!/bin/sh\nwhile read -r cmd first second; do\n\
                 \x20 case \"$cmd\" in\n\
                 \x20   -mkdir) p=${first#\\\"}; p=${p%\\\"}; mkdir -p -- \"$p\" ;;\n\
                 \x20   put) s=${first#\\\"}; s=${s%\\\"}; d=${second#\\\"}; d=${d%\\\"}; cp -- \"$s\" \"$d\" ;;\n\
                 \x20 esac\ndone\n",
            );
            let version = env!("CARGO_PKG_VERSION");
            let serve = if fail_serve {
                "echo 'remote build failed' >&2\nexit 1"
            } else {
                "exit 0"
            };
            let build = if fail_build {
                "echo 'remote candidate build failed' >&2\nexit 1"
            } else {
                "exit 0"
            };
            let stc = script(
                &bin,
                "stc",
                &format!(
                    "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'stc %s\\n' '{version}'; exit 0; fi\nif [ \"$1\" = \"serve\" ] && [ \"$2\" = \"start\" ]; then {serve}; fi\nif [ \"$1\" = \"build\" ]; then {build}; fi\nexit 0\n"
                ),
            );
            (SshRunner::new(ssh.to_string_lossy().into_owned()), stc, bin)
        }

        fn work_and_settings(
            root: &Path,
            stc: &Path,
            work: &Path,
        ) -> (ResolvedRemote, SshRunner, RemotePath) {
            std::fs::create_dir_all(work).expect("work");
            let session = work.join("stc-remote-session.json");
            let settings = RemoteSettings::from_strings(
                "r2k@ecat",
                Some(&work.to_string_lossy()),
                Some(&stc.to_string_lossy()),
                Some(&session.to_string_lossy()),
                Some("never"),
            )
            .expect("settings");
            let runner = SshRunner::new(root.join("bin/ssh").to_string_lossy().into_owned());
            let remote = ResolvedRemote::resolve(settings, &runner).expect("resolve");
            let session =
                RemotePath::parse_absolute(&session.to_string_lossy()).expect("session path");
            (remote, runner, session)
        }

        fn write_fixtures(remote: &ResolvedRemote, fingerprint: &str) {
            let work = remote.work_dir.as_str();
            let path = |name: &str| {
                RemotePath::parse_absolute(&format!("{work}/{name}")).expect("remote path")
            };
            let build_id = format!("0x{}", "11".repeat(16));
            let session = RemoteSession {
                host: "ecat-host".into(),
                work_dir: remote.work_dir.clone(),
                project: path("src/stc.toml"),
                module: path("build/demo.so"),
                manifest: path("build/demo.stc-got.json"),
                debug_endpoint_file: path("stc-debug.endpoint"),
                control_endpoint: path("stc-control.sock"),
                control_nonce_file: path("stc-control.nonce"),
                start_gate: Some(path("stc-start.gate")),
                build_id: build_id.clone(),
                generation: 1,
                source_fingerprint: fingerprint.to_owned(),
                mode: "debug".into(),
                stc_version: env!("CARGO_PKG_VERSION").to_owned(),
                pid: 1234,
            };
            let endpoint = format!(
                "{{\"audit_log\":\"{work}/stc-debug.audit.jsonl\",\"build_id\":\"{build_id}\",\
                 \"endpoint\":\"127.0.0.1:9\",\"format\":1,\"generation\":1,\"mode\":\"debug\",\
                 \"protocol\":\"stc-debug/1\",\"token\":\"{}\",\"transport\":\"socket\"}}\n",
                "ab".repeat(32)
            );
            write_private(
                Path::new(&format!("{work}/stc-remote-session.json")),
                &session.to_json_text(),
            );
            write_private(Path::new(&format!("{work}/stc-debug.endpoint")), &endpoint);
        }

        fn write_private(path: &Path, text: &str) {
            std::fs::write(path, text).expect("write private fixture");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("chmod private fixture");
        }

        #[test]
        fn sync_start_wait_and_descriptor_round_trip() {
            let root = temp_dir("round-trip");
            let (_runner_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            let (remote, runner, _session) = work_and_settings(&root, &stc, &work);
            check_remote_version(&runner, &remote).expect("version");
            let manifest = project(&root.join("project"));
            let sync = project_sync_files(&manifest).expect("sync");
            sync_project(&runner, &remote, &sync.files).expect("upload");
            assert!(work.join("src/stc.toml").is_file());
            assert!(work.join("src/src/main.st").is_file());
            let hashes = sync
                .files
                .iter()
                .map(|file| {
                    (
                        file.relative.clone(),
                        sha256::digest(&std::fs::read(&file.local).expect("read")),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            verify_hashes(&runner, &remote, &sync.files, &hashes).expect("verify");
            prepare_remote_session(&runner, &remote).expect("prepare");
            write_fixtures(&remote, &sync.fingerprint);
            start_remote_serve(
                &runner,
                &remote,
                &ServeStart {
                    root_manifest: &sync.root_manifest,
                    out_dir: &remote.build_dir,
                    start_paused: true,
                    no_build: false,
                    release: false,
                    allow_write: false,
                    allow_force: false,
                },
            )
            .expect("start");
            let (session, endpoint) = wait_for_remote_session(&runner, &remote).expect("wait");
            assert_eq!(session.source_fingerprint, sync.fingerprint);
            assert_eq!(endpoint.transport, TransportKind::Socket);
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn serve_failure_reports_the_log_and_hash_mismatch_is_rejected() {
            let root = temp_dir("failures");
            let (_runner_probe, stc, _bin) = tools(&root, true);
            let work = root.join("work");
            let (remote, runner, _session) = work_and_settings(&root, &stc, &work);
            let manifest = project(&root.join("project"));
            let sync = project_sync_files(&manifest).expect("sync");
            sync_project(&runner, &remote, &sync.files).expect("upload");
            prepare_remote_session(&runner, &remote).expect("prepare");
            start_remote_serve(
                &runner,
                &remote,
                &ServeStart {
                    root_manifest: &sync.root_manifest,
                    out_dir: &remote.build_dir,
                    start_paused: true,
                    no_build: false,
                    release: false,
                    allow_write: false,
                    allow_force: false,
                },
            )
            .expect("start");
            let error = wait_for_remote_session(&runner, &remote).expect_err("serve failure");
            assert!(error.contains("STC-E-REMOTE-006"), "{error}");
            assert!(error.contains("remote build failed"), "{error}");

            let mut wrong = BTreeMap::new();
            for file in &sync.files {
                wrong.insert(file.relative.clone(), "00".repeat(32));
            }
            let error =
                verify_hashes(&runner, &remote, &sync.files, &wrong).expect_err("hash mismatch");
            assert!(error.contains("STC-E-REMOTE-003"), "{error}");
            let mut missing = BTreeMap::new();
            missing.insert("not-uploaded.st".to_owned(), "00".repeat(32));
            let error =
                verify_hashes(&runner, &remote, &sync.files, &missing).expect_err("missing");
            assert!(error.contains("STC-E-REMOTE-003"), "{error}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn descriptor_and_endpoint_validation_rejects_drift() {
            let root = temp_dir("descriptor");
            let (_runner_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            let (remote, runner, _session) = work_and_settings(&root, &stc, &work);
            let manifest = project(&root.join("project"));
            let sync = project_sync_files(&manifest).expect("sync");
            write_fixtures(&remote, &sync.fingerprint);
            let (_, endpoint) = read_remote_session(&runner, &remote).expect("read");
            assert_eq!(endpoint.build_id, format!("0x{}", "11".repeat(16)));

            let session_path = work.join("stc-remote-session.json");
            let text = std::fs::read_to_string(&session_path).expect("session");
            write_private(
                &session_path,
                &text.replace("\"stc_version\":\"", "\"stc_version\":\"9"),
            );
            let error = read_remote_session(&runner, &remote).expect_err("version");
            assert!(error.contains("STC-E-REMOTE-002"), "{error}");

            write_private(&session_path, &text);
            let endpoint_path = work.join("stc-debug.endpoint");
            let endpoint_text = std::fs::read_to_string(&endpoint_path).expect("endpoint");
            write_private(
                &endpoint_path,
                &endpoint_text.replace("\"transport\":\"socket\"", "\"transport\":\"pipe\""),
            );
            let error = read_remote_session(&runner, &remote).expect_err("transport");
            assert!(error.contains("STC-E-REMOTE-007"), "{error}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn resolve_rejects_the_home_or_root_work_directory() {
            let root = temp_dir("resolve");
            let (_runner_probe, stc, _bin) = tools(&root, false);
            let runner = SshRunner::new(root.join("bin/ssh").to_string_lossy().into_owned());
            let root_settings = RemoteSettings::from_strings(
                "r2k@ecat",
                Some("/"),
                Some(&stc.to_string_lossy()),
                None,
                None,
            );
            assert!(
                root_settings.is_err(),
                "the root work directory is rejected"
            );
            let home_settings = RemoteSettings::from_strings(
                "r2k@ecat",
                Some("~"),
                Some(&stc.to_string_lossy()),
                None,
                None,
            )
            .expect("home settings");
            let error = ResolvedRemote::resolve(home_settings, &runner)
                .err()
                .expect("home work directory");
            assert_eq!(error.code, codes::REMOTE_SESSION);
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn remote_host_releases_a_gate_and_reports_the_exit_status() {
            let root = temp_dir("gate");
            let (_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            std::fs::create_dir_all(&work).expect("work");
            std::fs::write(work.join("stc-serve.status"), "7\n").expect("status");
            let mut host = remote_host(&root, &stc, &work, false, true);
            host.release_start_gate().expect("release");
            assert!(host.gate.is_none());
            host.release_start_gate().expect("released already");
            host.stop();
            assert_eq!(SessionHost::try_exit(&mut host), Some(7));
            assert!(SessionHost::drain_output(&mut host).is_empty());
            assert!(!SessionHost::attached(&host));
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn attached_remote_host_does_not_stop_or_report_exit() {
            let root = temp_dir("attached");
            let (_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            std::fs::create_dir_all(&work).expect("work");
            std::fs::write(work.join("stc-serve.status"), "9\n").expect("status");
            let mut host = remote_host(&root, &stc, &work, true, false);
            host.stop();
            assert_eq!(SessionHost::try_exit(&mut host), None);
            assert!(SessionHost::attached(&host));
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn status_reports_a_match_and_build_failures_as_cold_only() {
            let root = temp_dir("status");
            let (_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            let manager = project(&root.join("project"));
            let (remote, runner, _session) = work_and_settings(&root, &stc, &work);
            let sync = project_sync_files(&manager).expect("sync");
            write_fixtures(&remote, &sync.fingerprint);
            let invocation = invocation(&[
                "remote",
                "status",
                "--remote",
                "r2k@ecat",
                "--remote-dir",
                &work.to_string_lossy(),
                "--remote-stc",
                &stc.to_string_lossy(),
                "--manifest-path",
                &manager.to_string_lossy(),
                "--message-format",
                "json",
            ]);
            let report = status_report_with(&invocation, &runner).expect("status");
            assert_eq!(
                report.as_object().and_then(|object| object.get("drift")),
                Some(&stc_got::Json::Bool(false))
            );
            assert_eq!(
                report.as_object().and_then(|object| object.get("online")),
                Some(&stc_got::Json::Bool(true))
            );

            let (_probe, stc, _bin) = tools_with(&root, false, true);
            let (remote, runner, _session) = work_and_settings(&root, &stc, &work);
            write_fixtures(&remote, &format!("0x{}", "22".repeat(32)));
            let report = status_report_with(&invocation, &runner).expect("drift status");
            let object = report.as_object().expect("object");
            assert_eq!(object.get("drift"), Some(&stc_got::Json::Bool(true)));
            assert_eq!(
                object.get("cold_required"),
                Some(&stc_got::Json::Bool(true))
            );
            let reason = object
                .get("reason")
                .and_then(stc_got::Json::as_str)
                .expect("reason");
            assert!(reason.contains("STC-E-REMOTE-006"), "{reason}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn sync_and_start_failures_are_reported() {
            let root = temp_dir("sync-failures");
            let (_probe, stc, _bin) = tools(&root, false);
            let work = root.join("work");
            let manager = project(&root.join("project"));
            let (remote, _runner, _session) = work_and_settings(&root, &stc, &work);
            let sync = project_sync_files(&manager).expect("sync");

            let bin = root.join("failing-bin");
            script(
                &bin,
                "ssh",
                "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
            );
            script(&bin, "sftp", "#!/bin/sh\nexit 1\n");
            let failing = SshRunner::new(bin.join("ssh").to_string_lossy().into_owned());
            let error = sync_project(&failing, &remote, &sync.files).expect_err("upload");
            assert!(error.contains("STC-E-REMOTE-001"), "{error}");

            let missing = SshRunner::new("/nonexistent/stc-ssh");
            let error = sync_project(&missing, &remote, &sync.files).expect_err("reset");
            assert!(error.contains("STC-E-REMOTE-001"), "{error}");
            let error = prepare_remote_session(&missing, &remote).expect_err("prepare");
            assert!(error.contains("STC-E-REMOTE-001"), "{error}");
            let error = start_remote_serve(
                &missing,
                &remote,
                &ServeStart {
                    root_manifest: &sync.root_manifest,
                    out_dir: &remote.build_dir,
                    start_paused: true,
                    no_build: false,
                    release: false,
                    allow_write: false,
                    allow_force: false,
                },
            )
            .expect_err("start");
            assert!(error.contains("STC-E-REMOTE-001"), "{error}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn version_and_private_file_failures_are_classified() {
            let root = temp_dir("version");
            let bin = root.join("bin");
            script(
                &bin,
                "ssh",
                "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nexec /bin/sh -c \"$last\"\n",
            );
            let bad = script(&bin, "badstc", "#!/bin/sh\nprintf 'stc 0.0.0\\n'\n");
            let work = root.join("work");
            let (_remote, _runner, _session) = work_and_settings(&root, &bad, &work);
            let (remote, runner, _session2) = work_and_settings(&root, &bad, &work);
            let error = check_remote_version(&runner, &remote).expect_err("mismatch");
            assert!(error.contains("STC-E-REMOTE-002"), "{error}");
            let (remote, runner, _session3) =
                work_and_settings(&root, Path::new("/nonexistent/stc"), &work);
            let error = check_remote_version(&runner, &remote).expect_err("missing");
            assert!(error.contains("STC-E-REMOTE-002"), "{error}");

            let manager = project(&root.join("project"));
            let (remote, runner, _session4) = work_and_settings(&root, &bad, &work);
            let sync = project_sync_files(&manager).expect("sync");
            write_fixtures(&remote, &sync.fingerprint);
            let path = work.join("stc-remote-session.json");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
            let error = read_remote_session(&runner, &remote).expect_err("permissions");
            assert!(error.contains("STC-E-REMOTE-005"), "{error}");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
            let endpoint = work.join("stc-debug.endpoint");
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o644))
                .expect("chmod");
            let error = read_remote_session(&runner, &remote).expect_err("endpoint permissions");
            assert!(error.contains("STC-E-REMOTE-005"), "{error}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn ssh_auth_failure_is_reported_without_prompting() {
            let root = temp_dir("auth");
            let bin = root.join("bin");
            script(
                &bin,
                "ssh",
                "#!/bin/sh\necho 'Permission denied (publickey).' >&2\nexit 255\n",
            );
            let runner = SshRunner::new(bin.join("ssh").to_string_lossy().into_owned());
            let target = RemoteTarget::parse("r2k@ecat").expect("target");
            let error = runner.remote_home(&target).expect_err("auth failure");
            assert_eq!(error.code, codes::REMOTE_TRANSPORT);
            assert!(error.message.contains("Permission denied"), "{error}");
            std::fs::remove_dir_all(&root).expect("clean up");
        }

        #[test]
        fn tail_text_limits_each_stream() {
            assert_eq!(tail_text("", ""), "");
            assert_eq!(tail_text("out", "err"), ": err\nout");
            let long = "x".repeat(3000);
            let detail = tail_text(&long, "");
            assert!(detail.len() < 2100, "{}", detail.len());
        }

        fn remote_host(
            root: &Path,
            stc: &Path,
            work: &Path,
            attached: bool,
            gate: bool,
        ) -> RemoteHost {
            let absolute = |path: &Path| {
                RemotePath::parse_absolute(&path.to_string_lossy()).expect("remote path")
            };
            RemoteHost {
                runner: SshRunner::new(root.join("bin/ssh").to_string_lossy().into_owned()),
                target: RemoteTarget::parse("r2k@ecat").expect("target"),
                stc: RemoteProgram::parse(&stc.to_string_lossy()).expect("stc"),
                session: absolute(&work.join("stc-remote-session.json")),
                status: absolute(&work.join("stc-serve.status")),
                tunnel: None,
                gate: gate.then(|| absolute(&work.join("stc-start.gate"))),
                attached,
                stopped: false,
                exit_code: None,
            }
        }

        fn invocation(args: &[&str]) -> Invocation {
            let args = args
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>();
            crate::cli::parse(&args)
                .expect("parse")
                .expect("invocation")
        }

        fn temp_dir(name: &str) -> PathBuf {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "stc-remote-launch-{}-{name}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).expect("temp directory");
            directory
        }
    }
}
