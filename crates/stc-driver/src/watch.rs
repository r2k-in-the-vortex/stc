//! `stc run --watch`: rebuild on source changes and deploy online (D-155).
//!
//! The driver builds against the runtime store's active baseline, launches the
//! host with a local control endpoint, and sends module/manifest pairs after
//! each change. Deploy preparation runs on the host's cycle thread between
//! scans; a build error, incompatible change, or prepare failure leaves the
//! old generation active. Ctrl-C stops the host and exits 130; standard-input
//! EOF is also a graceful stop and exits 0 (D-213).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use stc_diag::Severity;
use stc_got::{Json, Manifest};
use stc_manifest::{LoadOptions, ProjectGraph};
use stc_runtime::control::ControlClient;
use stc_runtime::store::ManifestStore;

use crate::build::Request;
use crate::cli::DebugOptions;
use crate::compat::CompatibilityReport;
use crate::report::{DiagnosticPaths, Log, Reporter};
use crate::run as run_util;

/// Set by the stdin reader when standard input reaches EOF.
static STDIN_EOF: AtomicBool = AtomicBool::new(false);

/// Treats standard-input EOF as a graceful stop request (D-213).
///
/// The reader consumes stdin so a terminal Ctrl-D/Ctrl-Z or a closed pipe is
/// observed without blocking the watch loop. The extension closes the child's
/// stdin pipe to stop an IDE-owned watch.
fn watch_stdin_for_eof() {
    std::thread::spawn(|| {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 256];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        STDIN_EOF.store(true, Ordering::SeqCst);
    });
}

fn stdin_closed() -> bool {
    STDIN_EOF.load(Ordering::SeqCst)
}

/// Runs the watch workflow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    manifest: Option<&str>,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    debug: &DebugOptions,
    store: Option<&str>,
    timeout_ns: Option<u64>,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    stc_runtime::signal::install_stop_handler();
    watch_stdin_for_eof();
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_graph(manifest_input, locked, reporter) {
        Some(graph) => graph,
        None => return 2,
    };
    let root = graph.root_package();
    let root_dir = root.root_dir.clone();
    let name = root.name.clone();
    let mode = crate::project::select_mode(request, &root.manifest);
    let out_dir = request.out_dir.as_ref().map_or_else(
        || {
            if root.manifest.build.out_dir_explicit {
                root_dir.join(&root.manifest.build.out_dir)
            } else {
                root_dir.join(crate::build::default_out_dir(mode))
            }
        },
        PathBuf::from,
    );
    let store_dir = store_directory(store, &root.manifest.runtime.manifest_dir, &root_dir);
    log.status(format!("watch store {}", store_dir.display()));

    let token = random_token();
    let endpoint = endpoint(&token);
    let nonce = random_token();
    let baseline_dir = out_dir.join(".watch");
    if let Err(error) = std::fs::create_dir_all(&baseline_dir) {
        reporter.plain(
            "STC-E-BUILD-002",
            Severity::Error,
            &format!("cannot create `{}`: {error}", baseline_dir.display()),
        );
        return 1;
    }

    let initial = match build_once(
        manifest_input,
        &name,
        locked,
        diagnostic_paths,
        request,
        mode,
        &out_dir,
        &baseline_dir,
        &store_dir,
        reporter,
        log,
        true,
    ) {
        Ok(build) => build,
        Err(code) => return code,
    };

    let mut command = Command::new(build_launcher(&out_dir, &name));
    command.args(run_util::run_arguments(
        &request.run,
        &root.manifest,
        &root_dir,
        log.verbose(),
    ));
    command
        .arg("--control")
        .arg(&endpoint)
        .arg("--control-nonce")
        .arg(&nonce)
        .arg("--manifest-dir")
        .arg(&store_dir);
    if debug.server {
        let (debug_args, _) = crate::debug::server_arguments(debug, &root.manifest, &out_dir);
        command.args(debug_args);
    }
    let mut child = match command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot start the launcher: {error}"),
            );
            return 1;
        }
    };
    log.status(format!(
        "watching for changes (control endpoint {endpoint})"
    ));
    log.info(format!("control nonce {nonce}"));

    let mut client = match ControlClient::connect(&endpoint, Duration::from_secs(10)) {
        Ok(client) => client,
        Err(message) => {
            reporter.plain("STC-E-ONLINE-007", Severity::Error, &message);
            let _ = child.kill();
            let _ = child.wait();
            return 1;
        }
    };
    let mut last_deployed = initial.manifest.header.build_id;
    let mut files = snapshot(&graph);
    let started = Instant::now();
    let timeout = timeout_ns.map(Duration::from_nanos);
    let mut rebuild = false;
    let mut last_survey = Instant::now();
    loop {
        if stc_runtime::signal::stop_requested() {
            stop_host(&mut client, &nonce);
            let _ = child.wait();
            log.status("watch interrupted");
            return 130;
        }
        if stdin_closed() {
            stop_host(&mut client, &nonce);
            let _ = child.wait();
            log.status("watch stopped");
            return 0;
        }
        if let Some(timeout) = timeout
            && started.elapsed() >= timeout
        {
            stop_host(&mut client, &nonce);
            let code = wait_code(&mut child);
            log.status("watch timeout reached");
            return code.unwrap_or(0);
        }
        if let Some(status) = child.try_wait().unwrap_or(None) {
            let code = status.code().unwrap_or(1);
            log.status(format!("host exited with status {code}"));
            return u8::try_from(code).unwrap_or(1);
        }
        std::thread::sleep(Duration::from_millis(150));
        let current = snapshot(&graph);
        if current != files {
            // Debounce editors that write a file more than once.
            std::thread::sleep(Duration::from_millis(150));
            let settled = snapshot(&graph);
            files = settled;
            rebuild = true;
        }
        // A new or removed source file changes the discovered path set; re-run
        // discovery periodically to see it without a manifest edit.
        if last_survey.elapsed() >= Duration::from_millis(500) {
            last_survey = Instant::now();
            if let Ok(survey) =
                stc_manifest::load_project_readonly(manifest_input, &LoadOptions { locked })
            {
                let discovered = snapshot(&survey);
                if discovered != files {
                    files = discovered;
                    rebuild = true;
                }
            }
        }
        if !rebuild {
            continue;
        }
        rebuild = false;
        let Some(fresh) = load_graph(manifest_input, locked, reporter) else {
            log.status("manifest error; keeping the active generation");
            continue;
        };
        files = snapshot(&fresh);
        match build_once(
            manifest_input,
            &name,
            locked,
            diagnostic_paths,
            request,
            mode,
            &out_dir,
            &baseline_dir,
            &store_dir,
            reporter,
            log,
            false,
        ) {
            Ok(build) => {
                let outcome = deploy(&mut client, &nonce, &build, reporter, log);
                let outcome = match outcome {
                    Ok(DeployOutcome::Stale) => {
                        log.status("stale baseline; rebuilding against the active manifest");
                        match build_once(
                            manifest_input,
                            &name,
                            locked,
                            diagnostic_paths,
                            request,
                            mode,
                            &out_dir,
                            &baseline_dir,
                            &store_dir,
                            reporter,
                            log,
                            false,
                        ) {
                            Ok(retry) => match deploy(&mut client, &nonce, &retry, reporter, log) {
                                Ok(DeployOutcome::Stale) => Ok(DeployOutcome::Rejected),
                                other => other,
                            },
                            Err(_) => Ok(DeployOutcome::Rejected),
                        }
                    }
                    other => other,
                };
                match outcome {
                    Ok(DeployOutcome::Deployed(build_id)) => {
                        last_deployed = build_id;
                        log.status(format!("deployed build {}", last_deployed.to_hex()));
                    }
                    Ok(DeployOutcome::Rejected | DeployOutcome::Stale) => {
                        log.status(format!(
                            "keeping generation {}; candidate was rejected",
                            last_deployed.to_hex()
                        ));
                    }
                    Err(_) => {
                        log.status(format!(
                            "keeping generation {}; deploy failed",
                            last_deployed.to_hex()
                        ));
                    }
                }
            }
            Err(_) => {
                log.status(format!(
                    "build failed; keeping generation {}",
                    last_deployed.to_hex()
                ));
            }
        }
    }
}

struct Built {
    module: PathBuf,
    manifest_path: PathBuf,
    manifest: Manifest,
    baseline: Option<Manifest>,
}

#[allow(clippy::too_many_arguments)]
fn build_once(
    manifest_input: &str,
    name: &str,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    mode: crate::build::BuildMode,
    out_dir: &Path,
    baseline_dir: &Path,
    store_dir: &Path,
    reporter: &mut Reporter,
    log: Log,
    initial: bool,
) -> Result<Built, u8> {
    // The launcher is linked once; later rebuilds must not overwrite the
    // executable while the host process holds it open.
    let emit = if initial {
        "exe,module,metadata"
    } else {
        "module,metadata"
    };
    let baseline = if request.cold {
        None
    } else {
        load_baseline(store_dir, reporter)?
    };
    if initial {
        match &baseline {
            Some(active) => log.status(format!(
                "online baseline {} from {}",
                active.header.build_id.to_hex(),
                store_dir.display()
            )),
            None => log.status("cold layout: no active baseline".to_owned()),
        }
    }
    let baseline_path = baseline_dir.join("baseline.stc-got.json");
    let mut got_manifest = None;
    if let Some(active) = &baseline {
        let text = match active.to_json_text() {
            Ok(text) => text,
            Err(error) => {
                reporter.plain("STC-E-ONLINE-009", Severity::Error, &error.message);
                return Err(1);
            }
        };
        if let Err(error) = std::fs::write(&baseline_path, text) {
            return Err(build_error(
                reporter,
                &format!("cannot write `{}`: {error}", baseline_path.display()),
            ));
        }
        got_manifest = Some(baseline_path.clone());
    }

    let mut command = Command::new(current_exe());
    command.args([
        "build",
        "--manifest-path",
        manifest_input,
        "--emit",
        emit,
        "--out-dir",
    ]);
    command.arg(out_dir);
    command.arg(if mode.is_release() {
        "--release"
    } else {
        "--debug"
    });
    if let Some(path) = &got_manifest {
        command.arg("--got-manifest").arg(path);
    }
    if request.cold {
        command.arg("--cold");
    }
    if request.relaxed {
        command.arg("--relaxed");
    }
    if locked {
        command.arg("--locked");
    }
    if diagnostic_paths == DiagnosticPaths::Absolute {
        command.args(["--diagnostic-paths", "absolute"]);
    }
    if let Some(target) = &request.target {
        command.arg("--target").arg(target);
    }
    if let Some(clang) = &request.clang {
        command.arg("--clang").arg(clang);
    }
    if let Some(runtime) = &request.runtime {
        command.arg("--runtime").arg(runtime);
    }
    let output = command
        .output()
        .map_err(|error| build_error(reporter, &format!("cannot run `stc build`: {error}")))?;
    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        if initial {
            log.status(format!("initial build failed with status {code}"));
        }
        return Err(u8::try_from(code).unwrap_or(1));
    }
    let manifest_path = out_dir.join(format!("{name}.stc-got.json"));
    let manifest = match crate::online::read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(message) => {
            reporter.plain("STC-E-MANIFEST-001", Severity::Error, &message);
            return Err(1);
        }
    };
    let metadata_path = out_dir.join(format!("{name}.stc-meta.json"));
    let module = match module_file(&metadata_path) {
        Some(file) => out_dir.join(file),
        None => {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot read `{}`", metadata_path.display()),
            );
            return Err(1);
        }
    };
    Ok(Built {
        module,
        manifest_path,
        manifest,
        baseline,
    })
}

enum DeployOutcome {
    Deployed(stc_got::GotId),
    Rejected,
    Stale,
}

fn deploy(
    client: &mut ControlClient,
    nonce: &str,
    build: &Built,
    reporter: &mut Reporter,
    log: Log,
) -> Result<DeployOutcome, ()> {
    let response = send_deploy(client, nonce, build).map_err(|message| {
        reporter.plain("STC-E-ONLINE-007", Severity::Error, &message);
    })?;
    if let Some(ns) = response
        .as_object()
        .and_then(|object| object.get("prepare_ns"))
        .and_then(Json::as_number)
    {
        log.info(format!(
            "deploy prepare/maintenance {:.2}ms",
            ns as f64 / 1_000_000.0
        ));
    }
    let report = CompatibilityReport::new(&build.manifest, build.baseline.as_ref());
    report.print(reporter, log);
    if response_ok(&response) {
        return Ok(DeployOutcome::Deployed(build.manifest.header.build_id));
    }
    if response_bool(&response, "cold_required") {
        return Ok(DeployOutcome::Stale);
    }
    let code = response_str(&response, "code").unwrap_or("STC-E-ONLINE-008");
    let message = response_str(&response, "message").unwrap_or("the deploy was rejected");
    reporter.plain(code, Severity::Error, message);
    Ok(DeployOutcome::Rejected)
}

fn send_deploy(client: &mut ControlClient, nonce: &str, build: &Built) -> Result<Json, String> {
    // The host loads the compact binary manifest; the JSON projection was
    // already read for the compatibility report.
    let binary = {
        let name = build
            .manifest_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app.stc-got.json");
        let stem = name.strip_suffix(".stc-got.json").unwrap_or(name);
        build
            .manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{stem}.stc-got.bin"))
    };
    client.request(
        nonce,
        Json::object([
            ("manifest".into(), Json::String(absolute(&binary))),
            ("module".into(), Json::String(absolute(&build.module))),
            ("op".into(), Json::String("deploy".into())),
        ]),
    )
}

fn stop_host(client: &mut ControlClient, nonce: &str) {
    let _ = client.request(nonce, Json::object([("op".into(), Json::string("stop"))]));
}

fn wait_code(child: &mut Child) -> Option<u8> {
    match child.wait() {
        Ok(status) => status.code().and_then(|code| u8::try_from(code).ok()),
        Err(_) => None,
    }
}

fn load_graph(manifest: &str, locked: bool, reporter: &mut Reporter) -> Option<ProjectGraph> {
    match stc_manifest::load_project_readonly(manifest, &LoadOptions { locked }) {
        Ok(graph) => {
            let root = graph.root_package();
            reporter.merge_manifest_lints(&root.manifest.lints);
            reporter.diagnostics(&graph.warnings, &graph.sources);
            Some(graph)
        }
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            None
        }
    }
}

fn module_file(metadata_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(metadata_path).ok()?;
    let value = stc_got::json::parse(&text).ok()?;
    value
        .as_object()?
        .get("module")?
        .as_object()?
        .get("file")?
        .as_str()
        .map(str::to_owned)
}

fn load_baseline(store_dir: &Path, reporter: &mut Reporter) -> Result<Option<Manifest>, u8> {
    let store = ManifestStore::open(store_dir);
    if let Some(problem) = store.problem() {
        reporter.plain(
            "STC-W-ONLINE-007",
            Severity::Warning,
            &format!(
                "the runtime store `{}` is invalid ({problem}); starting cold",
                store_dir.display()
            ),
        );
        return Ok(None);
    }
    match store.active() {
        Some(active) => Ok(Some(active.manifest.clone())),
        None => Ok(None),
    }
}

fn store_directory(store: Option<&str>, manifest_dir: &Option<String>, root_dir: &Path) -> PathBuf {
    if let Some(store) = store {
        return PathBuf::from(store);
    }
    if let Some(dir) = manifest_dir {
        return root_dir.join(dir);
    }
    // The default keeps the store beside the build artifacts and survives
    // `stc clean` only when the target directory does.
    root_dir.join("target/stc/runtime")
}

fn snapshot(graph: &ProjectGraph) -> BTreeMap<PathBuf, (u128, u64, bool)> {
    let mut paths = Vec::new();
    for package in &graph.packages {
        paths.push(package.manifest_path.clone());
        for source in &package.sources {
            paths.push(source.path.clone());
        }
        for source in &package.test_sources {
            paths.push(source.path.clone());
        }
    }
    paths.push(graph.lockfile_path.clone());
    paths.push(graph.root_package().root_dir.join("stc.renames.toml"));
    let mut snapshot = BTreeMap::new();
    for path in paths {
        let (stamp, length, exists) = match std::fs::metadata(&path) {
            Ok(metadata) => (
                metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos()),
                metadata.len(),
                true,
            ),
            Err(_) => (0, 0, false),
        };
        snapshot.insert(path, (stamp, length, exists));
    }
    snapshot
}

fn build_launcher(out_dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        out_dir.join(format!("{name}.exe"))
    } else {
        out_dir.join(name)
    }
}

fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("stc"))
}

fn build_error(reporter: &mut Reporter, message: &str) -> u8 {
    reporter.plain("STC-E-BUILD-002", Severity::Error, message);
    1
}

fn absolute(path: &Path) -> String {
    if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }
}

fn response_ok(response: &Json) -> bool {
    response_bool(response, "ok")
}

fn response_bool(response: &Json, key: &str) -> bool {
    response
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

fn response_str<'a>(response: &'a Json, key: &str) -> Option<&'a str> {
    response
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Json::as_str)
}

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_token() -> String {
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let seed = format!("{}-{counter}-{nanos}", std::process::id());
    let digest = stc_got::sha256::sha256(seed.as_bytes());
    let mut token = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        token.push_str(&format!("{byte:02x}"));
    }
    token
}

fn endpoint(token: &str) -> String {
    if cfg!(windows) {
        format!(r"\\.\pipe\stc-{}-{token}", std::process::id())
    } else {
        std::env::temp_dir()
            .join(format!("stc-{}-{token}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }
}
