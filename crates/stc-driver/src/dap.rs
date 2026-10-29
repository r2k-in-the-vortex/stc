//! `stc dap`: the local Debug Adapter Protocol server (WL-1.1 I-3).
//!
//! The driver owns building, host launch, attach, and process ownership; the
//! [`stc_dap`] crate owns DAP framing and translation to `stc-debug/1.1`. This
//! module is the `Launcher` implementation that the adapter calls when a
//! launch or attach request has been configured.
//!
//! Standard output carries DAP messages only. Build diagnostics, phase logs,
//! and the debug endpoint line go to stderr, which the editor surfaces in the
//! debug console.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use stc_dap::{DapOptions, LaunchArguments, LaunchedSession, Launcher, OutputLine, SessionHost};
use stc_got::Json;
use stc_manifest::LoadOptions;
use stc_runtime::DebugClient;
use stc_runtime::control::ControlClient;

use crate::build::{self, BuildMode, Request};
use crate::cli::DebugOptions;
use crate::project::{self, ProjectBuild};
use crate::report::{ColorChoice, DiagnosticPaths, Format, LintOverrides, Log, Reporter};
use crate::run as run_util;

/// Deadline for the host to publish its endpoint file.
const ENDPOINT_DEADLINE: Duration = Duration::from_secs(5);

/// Runs the debug adapter over the process standard streams.
pub(crate) fn run(manifest: Option<&str>) -> u8 {
    let mut launcher = DapLauncher {
        manifest_path: manifest.map(PathBuf::from),
    };
    let options = DapOptions {
        manifest_path: launcher.manifest_path.clone(),
    };
    stc_dap::run_stdio(&mut launcher, options)
}

/// The driver's host launcher.
struct DapLauncher {
    /// `stc dap --manifest-path` fallback for launch configurations that omit
    /// `project`.
    manifest_path: Option<PathBuf>,
}

impl Launcher for DapLauncher {
    fn launch(&mut self, arguments: &LaunchArguments) -> Result<LaunchedSession, String> {
        let project_path = arguments.project_path(self.manifest_path.as_deref());
        let build = if arguments.no_build {
            no_build(&project_path, arguments)?
        } else {
            let mut request = Request::default();
            if arguments.release {
                request.mode_override = Some(BuildMode::Release);
            }
            if let Some(out_dir) = &arguments.out_dir {
                request.out_dir = Some(out_dir.clone());
            }
            let mut reporter = Reporter::new(
                Format::Human,
                ColorChoice::Never,
                false,
                &LintOverrides::default(),
            );
            project::build_for_launch(
                Some(&project_path.to_string_lossy()),
                false,
                DiagnosticPaths::Package,
                &request,
                &mut reporter,
                Log::new(false, 0),
            )
            .map_err(|code| format!("the project build failed (exit code {code})"))?
        };
        spawn_host(&build, arguments)
    }

    fn attach(&mut self, arguments: &LaunchArguments) -> Result<LaunchedSession, String> {
        let Some(endpoint) = arguments.endpoint.as_deref() else {
            return Err("attach requires an `endpoint` file".into());
        };
        let path = absolute_path(Path::new(endpoint));
        let client = DebugClient::connect_file(&path).map_err(|error| {
            format!(
                "cannot connect to the debug endpoint `{}`: {error}",
                path.display()
            )
        })?;
        Ok(LaunchedSession { client, host: None })
    }
}

/// Resolves a `noBuild` launch from existing artifacts.
fn no_build(project_path: &Path, arguments: &LaunchArguments) -> Result<ProjectBuild, String> {
    let graph = stc_manifest::load_project_readonly(project_path, &LoadOptions::default())
        .map_err(|error| {
            error.diagnostics.first().map_or_else(
                || {
                    format!(
                        "cannot load the project `{}`",
                        project_path.to_string_lossy()
                    )
                },
                |diagnostic| {
                    format!(
                        "cannot load the project `{}`: {}: {}",
                        project_path.to_string_lossy(),
                        diagnostic.code,
                        diagnostic.message
                    )
                },
            )
        })?;
    let root = graph.root_package();
    let build_mode = if arguments.release {
        BuildMode::Release
    } else {
        BuildMode::Debug
    };
    let out_dir = match &arguments.out_dir {
        Some(dir) => {
            let dir = Path::new(dir);
            if dir.is_absolute() {
                dir.to_path_buf()
            } else {
                root.root_dir.join(dir)
            }
        }
        None if root.manifest.build.out_dir_explicit => {
            root.root_dir.join(&root.manifest.build.out_dir)
        }
        None => root.root_dir.join(build::default_out_dir(build_mode)),
    };
    let launcher = build::launcher_path(&out_dir, &root.name);
    Ok(ProjectBuild {
        launcher,
        out_dir,
        run: build::RunRequest::default(),
        manifest: root.manifest.clone(),
        root_dir: root.root_dir.clone(),
    })
}

/// Spawns the runtime host with piped output and connects the debug client.
fn spawn_host(
    build: &ProjectBuild,
    arguments: &LaunchArguments,
) -> Result<LaunchedSession, String> {
    let endpoint = build
        .out_dir
        .join(format!("stc-debug-{}.endpoint", crate::debug::random_hex()));
    let control = crate::debug::control_endpoint();
    let nonce = crate::debug::random_hex();
    let debug_options = DebugOptions {
        allow_write: arguments.allow_write,
        allow_force: arguments.allow_force,
        ..DebugOptions::default()
    };
    let mut command = Command::new(&build.launcher);
    command.args(run_util::run_arguments(
        &build.run,
        &build.manifest,
        &build.root_dir,
        false,
    ));
    command.args(crate::debug::server_arguments_for(
        &debug_options,
        &build.manifest,
        &endpoint,
    ));
    command
        .arg("--control")
        .arg(&control)
        .arg("--control-nonce")
        .arg(&nonce);
    // The launch configuration's `args` follow the resolved run options so a
    // later flag overrides a manifest default.
    command.args(&arguments.args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &arguments.cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "cannot start the launcher `{}`: {error}",
            build.launcher.display()
        )
    })?;
    let (sender, receiver) = mpsc::channel();
    spawn_output_reader(child.stdout.take(), "stdout", sender.clone());
    spawn_output_reader(child.stderr.take(), "stderr", sender);
    let deadline = Instant::now() + ENDPOINT_DEADLINE;
    while !endpoint.is_file() {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the target exited before publishing its debug endpoint (exit code {})",
                status.code().unwrap_or(1)
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the debug endpoint was not created within five seconds".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let client = DebugClient::connect_file(&endpoint).map_err(|error| {
        let _ = child.kill();
        let _ = child.wait();
        format!(
            "cannot connect to the debug endpoint `{}`: {error}",
            endpoint.display()
        )
    })?;
    Ok(LaunchedSession {
        client,
        host: Some(Box::new(DriverHost {
            child,
            control,
            nonce,
            output: receiver,
        })),
    })
}

fn spawn_output_reader<R: Read + Send + 'static>(
    stream: Option<R>,
    category: &'static str,
    sender: mpsc::Sender<OutputLine>,
) {
    let Some(stream) = stream else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name(format!("stc-dap-{category}"))
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if sender
                            .send(OutputLine {
                                category,
                                text: line.clone(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("stc-dap: cannot start the {category} reader: {error}");
    }
}

/// A launch-owned host process driven through the control channel.
struct DriverHost {
    child: Child,
    control: String,
    nonce: String,
    output: Receiver<OutputLine>,
}

impl SessionHost for DriverHost {
    fn drain_output(&mut self) -> Vec<OutputLine> {
        self.output.try_iter().collect()
    }

    fn try_exit(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(1)),
            _ => None,
        }
    }

    fn shutdown(&mut self) {
        if let Ok(mut client) =
            ControlClient::connect(&self.control, crate::debug::CONTROL_DEADLINE)
        {
            let _ = client.request(
                &self.nonce,
                Json::object([("op".into(), Json::string("stop"))]),
            );
        }
        let _ = crate::debug::wait_child(&mut self.child);
    }
}

impl Drop for DriverHost {
    fn drop(&mut self) {
        // Never leave a launch-owned host running if the adapter exits without
        // a clean disconnect.
        if self.try_exit().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
