//! Command-line parsing for the driver.
//!
//! The parser recognizes the full M6 command surface. Online-change options
//! select the runtime store, a live control host, an explicit diff baseline,
//! and the rename/watch controls (`--dry-run`, `--watch-timeout`).

use stc_manifest::LintLevel;

use crate::build::{BuildMode, Emit, Request};
use crate::report::{ColorChoice, Format, LintOverrides};

/// A driver command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Lex and parse one file, optionally dumping the CST/AST.
    Parse,
    /// Analyze a project (or the manifest-free file list).
    Check,
    /// Build a project (or the manifest-free file list).
    Build,
    /// Scaffold a new project.
    New,
    /// Build and execute a project.
    Run,
    /// Build and run the test suite.
    Test,
    /// Format sources.
    Fmt,
    /// Remove the configured output directory.
    Clean,
    /// Print the build metadata summary.
    Metadata,
    /// Print the conformance/feature summary.
    Compliance,
    /// Export the active manifest.
    OnlineDownload,
    /// Compare a candidate manifest with the active baseline.
    OnlineDiff,
    /// Source-aware tracked rename.
    Rename,
}

/// `stc test` options.
#[derive(Clone, Debug, Default)]
pub(crate) struct TestOptions {
    /// `--list`: print discovered tests without building.
    pub list: bool,
    /// `--no-run`: build test artifacts without executing them.
    pub no_run: bool,
    /// `--timeout <duration>` override in nanoseconds.
    pub timeout_ns: Option<u64>,
    /// `-- --nocapture`: forward test-host output.
    pub nocapture: bool,
}

/// Online-change command options.
#[derive(Clone, Debug, Default)]
pub(crate) struct OnlineOptions {
    /// `--out`: download envelope output path; stdout when absent.
    pub(crate) out: Option<String>,
    /// `--binary`: optional binary manifest projection output path.
    pub(crate) binary: Option<String>,
    /// `--store`: runtime manifest-store directory.
    pub(crate) store: Option<String>,
    /// `--host`: live control endpoint.
    pub(crate) host: Option<String>,
    /// `--nonce`: live control nonce.
    pub(crate) nonce: Option<String>,
    /// `--baseline`: explicit diff baseline manifest.
    pub(crate) baseline: Option<String>,
}

/// A parsed invocation.
pub(crate) struct Invocation {
    /// Selected command.
    pub(crate) command: Command,
    /// Positional arguments (files, a scaffold path, test filters, or rename keys).
    pub(crate) paths: Vec<String>,
    /// Diagnostic message format.
    pub(crate) format: Format,
    /// Color selection.
    pub(crate) color: ColorChoice,
    /// `-q`.
    pub(crate) quiet: bool,
    /// `-v` count; 0, 1, or 2.
    pub(crate) verbosity: u8,
    /// `--relaxed`.
    pub(crate) relaxed: bool,
    /// `--ast` (parse only).
    pub(crate) ast: bool,
    /// `--manifest-path`.
    pub(crate) manifest: Option<String>,
    /// `--locked`.
    pub(crate) locked: bool,
    /// CLI lint overrides.
    pub(crate) lints: LintOverrides,
    /// `run --watch`.
    pub(crate) watch: bool,
    /// Optional `--watch-timeout` in nanoseconds.
    pub(crate) watch_timeout_ns: Option<u64>,
    /// `rename --dry-run`.
    pub(crate) dry_run: bool,
    /// Online-change options.
    pub(crate) online: OnlineOptions,
    /// `fmt --check`.
    pub(crate) fmt_check: bool,
    /// Build settings and overrides.
    pub(crate) build: Request,
    /// `stc test` options.
    pub(crate) test: TestOptions,
}

/// Parses argv.
///
/// Returns `Ok(None)` when `--help`/`--version` was handled.
///
/// # Errors
///
/// Returns a usage message for unknown commands/options, missing values, and
/// options used with the wrong command.
pub(crate) fn parse(args: &[String]) -> Result<Option<Invocation>, String> {
    if args.is_empty() {
        return Err("missing command".into());
    }
    let mut index = 0;
    let command = match args[index].as_str() {
        "--help" | "-h" => {
            print_help();
            return Ok(None);
        }
        "--version" | "-V" => {
            println!("stc {}", env!("CARGO_PKG_VERSION"));
            return Ok(None);
        }
        "parse" => Command::Parse,
        "check" => Command::Check,
        "build" => Command::Build,
        "new" => Command::New,
        "run" => Command::Run,
        "test" => Command::Test,
        "fmt" => Command::Fmt,
        "clean" => Command::Clean,
        "metadata" => Command::Metadata,
        "compliance" => Command::Compliance,
        "rename" => Command::Rename,
        "online" => {
            let subcommand = args
                .get(1)
                .ok_or_else(|| "`online` needs `download` or `diff`".to_string())?;
            index = 1;
            match subcommand.as_str() {
                "download" => Command::OnlineDownload,
                "diff" => Command::OnlineDiff,
                other => {
                    return Err(format!(
                        "unknown online subcommand `{other}` (expected `download` or `diff`)"
                    ));
                }
            }
        }
        other => return Err(format!("unknown command `{other}`")),
    };
    index += 1;

    let mut format = Format::Human;
    let mut color = ColorChoice::Auto;
    let mut quiet = false;
    let mut verbosity = 0_u8;
    let mut relaxed = false;
    let mut ast = false;
    let mut manifest = None;
    let mut locked = false;
    let mut lints = LintOverrides::default();
    let mut watch = false;
    let mut watch_timeout_ns = None;
    let mut dry_run = false;
    let mut online = OnlineOptions::default();
    let mut fmt_check = false;
    let mut test = TestOptions::default();
    let mut paths = Vec::new();
    let mut build = Request::default();

    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--help" | "-h" => {
                print_help();
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("stc {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--relaxed" => relaxed = true,
            "--ast" => ast = true,
            "--locked" => locked = true,
            "--watch" => watch = true,
            "--watch-timeout" => {
                watch_timeout_ns = Some(parse_cycle_time(&next_value(
                    args,
                    &mut index,
                    "--watch-timeout",
                )?)?);
            }
            other if other.starts_with("--watch-timeout=") => {
                watch_timeout_ns = Some(parse_cycle_time(&other["--watch-timeout=".len()..])?);
            }
            "--dry-run" => dry_run = true,
            "--check" => fmt_check = true,
            "--list" => test.list = true,
            "--no-run" => test.no_run = true,
            "--timeout" => {
                test.timeout_ns = Some(parse_cycle_time(&next_value(
                    args,
                    &mut index,
                    "--timeout",
                )?)?);
            }
            other if other.starts_with("--timeout=") => {
                test.timeout_ns = Some(parse_cycle_time(&other["--timeout=".len()..])?);
            }
            "--" => {
                if command != Command::Test {
                    return Err("`--` is only valid with `stc test`".into());
                }
                for passthrough in &args[index + 1..] {
                    match passthrough.as_str() {
                        "--nocapture" => test.nocapture = true,
                        other => {
                            return Err(format!(
                                "unknown test argument `{other}` (expected `--nocapture`)"
                            ));
                        }
                    }
                }
                break;
            }
            "-q" | "--quiet" => quiet = true,
            "-v" | "--verbose" => verbosity = verbosity.max(1),
            "-vv" => verbosity = 2,
            "--debug" => select_build_mode(&mut build, BuildMode::Debug)?,
            "--release" => select_build_mode(&mut build, BuildMode::Release)?,
            "--debug-server" => {
                if command == Command::Test {
                    return Err(
                        "STC-E-PROJECT-010: `stc test` always builds the hook-free debug form and \
                         rejects `--debug-server`"
                            .into(),
                    );
                }
                return Err(
                    "`--debug-server` is not available until the M6.5 debug session phase".into(),
                );
            }
            "--debug-protocol" => {
                let value = next_value(args, &mut index, "--debug-protocol")?;
                if command == Command::Test {
                    return Err(
                        "STC-E-PROJECT-010: `stc test` always builds the hook-free debug form and \
                         rejects `--debug-protocol`"
                            .into(),
                    );
                }
                return Err(format!(
                    "`--debug-protocol {value}` is not available until the M6.5 debug session phase"
                ));
            }
            other if other.starts_with("--debug-protocol=") => {
                if command == Command::Test {
                    return Err(
                        "STC-E-PROJECT-010: `stc test` always builds the hook-free debug form and \
                         rejects `--debug-protocol`"
                            .into(),
                    );
                }
                return Err(format!(
                    "`{other}` is not available until the M6.5 debug session phase"
                ));
            }
            "--cold" => build.cold = true,
            "--allow-untracked-removal" => build.allow_untracked_removal = true,
            "--deny" => {
                let value = next_value(args, &mut index, "--deny")?;
                if value != "warnings" {
                    return Err(format!("`--deny` expects `warnings`, found `{value}`"));
                }
                lints.set("warnings", LintLevel::Deny)?;
            }
            "-D" | "-W" | "-A" => {
                let flag = argument.as_bytes()[1] as char;
                let value = next_value(args, &mut index, argument)?;
                apply_lint(&mut lints, flag, &value)?;
            }
            other if is_attached_lint(other) => {
                let flag = other.as_bytes()[1] as char;
                apply_lint(&mut lints, flag, &other[2..])?;
            }
            "--message-format" => {
                let value = next_value(args, &mut index, "--message-format")?;
                format = parse_format(&value)?;
            }
            other if other.starts_with("--message-format=") => {
                format = parse_format(&other["--message-format=".len()..])?;
            }
            "--color" => {
                let value = next_value(args, &mut index, "--color")?;
                color = ColorChoice::parse(&value)?;
            }
            other if other.starts_with("--color=") => {
                color = ColorChoice::parse(&other["--color=".len()..])?;
            }
            "--manifest-path" => {
                manifest = Some(next_value(args, &mut index, "--manifest-path")?);
            }
            other if other.starts_with("--manifest-path=") => {
                manifest = Some(other["--manifest-path=".len()..].to_owned());
            }
            "--name" => build.name = Some(next_value(args, &mut index, "--name")?),
            other if other.starts_with("--name=") => {
                build.name = Some(other["--name=".len()..].to_owned());
            }
            "--emit" => {
                build.emit = parse_emit(&next_value(args, &mut index, "--emit")?)?;
            }
            other if other.starts_with("--emit=") => {
                build.emit = parse_emit(&other["--emit=".len()..])?;
            }
            "--out-dir" => {
                build.out_dir = Some(next_value(args, &mut index, "--out-dir")?);
                build.out_dir_explicit = true;
            }
            other if other.starts_with("--out-dir=") => {
                build.out_dir = Some(other["--out-dir=".len()..].to_owned());
                build.out_dir_explicit = true;
            }
            "--out" => online.out = Some(next_value(args, &mut index, "--out")?),
            other if other.starts_with("--out=") => {
                online.out = Some(other["--out=".len()..].to_owned());
            }
            "--binary" => online.binary = Some(next_value(args, &mut index, "--binary")?),
            other if other.starts_with("--binary=") => {
                online.binary = Some(other["--binary=".len()..].to_owned());
            }
            "--store" => online.store = Some(next_value(args, &mut index, "--store")?),
            other if other.starts_with("--store=") => {
                online.store = Some(other["--store=".len()..].to_owned());
            }
            "--host" => online.host = Some(next_value(args, &mut index, "--host")?),
            other if other.starts_with("--host=") => {
                online.host = Some(other["--host=".len()..].to_owned());
            }
            "--nonce" => online.nonce = Some(next_value(args, &mut index, "--nonce")?),
            other if other.starts_with("--nonce=") => {
                online.nonce = Some(other["--nonce=".len()..].to_owned());
            }
            "--baseline" => online.baseline = Some(next_value(args, &mut index, "--baseline")?),
            other if other.starts_with("--baseline=") => {
                online.baseline = Some(other["--baseline=".len()..].to_owned());
            }
            "--target" => build.target = Some(next_value(args, &mut index, "--target")?),
            other if other.starts_with("--target=") => {
                build.target = Some(other["--target=".len()..].to_owned());
            }
            "--clang" => build.clang = Some(next_value(args, &mut index, "--clang")?),
            other if other.starts_with("--clang=") => {
                build.clang = Some(other["--clang=".len()..].to_owned());
            }
            "--runtime" => build.runtime = Some(next_value(args, &mut index, "--runtime")?),
            other if other.starts_with("--runtime=") => {
                build.runtime = Some(other["--runtime=".len()..].to_owned());
            }
            "--got-manifest" => {
                build.got_manifest = Some(next_value(args, &mut index, "--got-manifest")?);
            }
            other if other.starts_with("--got-manifest=") => {
                build.got_manifest = Some(other["--got-manifest=".len()..].to_owned());
            }
            "--rename" => {
                let value = next_value(args, &mut index, "--rename")?;
                build.renames.push(parse_rename(&value)?);
            }
            other if other.starts_with("--rename=") => {
                build
                    .renames
                    .push(parse_rename(&other["--rename=".len()..])?);
            }
            "--cycles" => {
                build.run.cycles = Some(parse_cycles(&next_value(args, &mut index, "--cycles")?)?);
            }
            other if other.starts_with("--cycles=") => {
                build.run.cycles = Some(parse_cycles(&other["--cycles=".len()..])?);
            }
            "--cycle-time" => {
                build.run.cycle_time_ns = Some(parse_cycle_time(&next_value(
                    args,
                    &mut index,
                    "--cycle-time",
                )?)?);
            }
            other if other.starts_with("--cycle-time=") => {
                build.run.cycle_time_ns = Some(parse_cycle_time(&other["--cycle-time=".len()..])?);
            }
            "--io" => {
                build.run.io = Some(parse_io(&next_value(args, &mut index, "--io")?)?);
            }
            other if other.starts_with("--io=") => {
                build.run.io = Some(parse_io(&other["--io=".len()..])?);
            }
            "--io-file" => {
                build.run.io_file = Some(next_value(args, &mut index, "--io-file")?);
            }
            other if other.starts_with("--io-file=") => {
                build.run.io_file = Some(other["--io-file=".len()..].to_owned());
            }
            "--state-file" => {
                build.run.state_file = Some(next_value(args, &mut index, "--state-file")?);
            }
            other if other.starts_with("--state-file=") => {
                build.run.state_file = Some(other["--state-file=".len()..].to_owned());
            }
            "--reset-retain" => build.run.reset_retain = true,
            "--no-retain" => build.run.no_retain = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            other => paths.push(other.to_owned()),
        }
        index += 1;
    }

    build.relaxed = relaxed;
    validate(
        command,
        &paths,
        manifest.as_deref(),
        locked,
        relaxed,
        ast,
        watch,
        watch_timeout_ns,
        dry_run,
        &online,
        fmt_check,
        &build,
        &test,
    )?;

    Ok(Some(Invocation {
        command,
        paths,
        format,
        color,
        quiet,
        verbosity,
        relaxed,
        ast,
        manifest,
        locked,
        lints,
        watch,
        watch_timeout_ns,
        dry_run,
        online,
        fmt_check,
        build,
        test,
    }))
}

fn select_build_mode(request: &mut Request, mode: BuildMode) -> Result<(), String> {
    if let Some(previous) = request.mode_override
        && previous != mode
    {
        return Err(
            "STC-E-PROJECT-010: `--debug` and `--release` are mutually exclusive; select one build \
             mode"
                .into(),
        );
    }
    request.mode_override = Some(mode);
    Ok(())
}

fn is_attached_lint(argument: &str) -> bool {
    argument.len() > 2
        && matches!(argument.as_bytes()[0], b'-')
        && matches!(argument.as_bytes()[1], b'D' | b'W' | b'A')
}

fn apply_lint(lints: &mut LintOverrides, flag: char, value: &str) -> Result<(), String> {
    let level = match flag {
        'D' => LintLevel::Deny,
        'W' => LintLevel::Warn,
        'A' => LintLevel::Allow,
        _ => return Err(format!("unknown lint option `-{flag}`")),
    };
    lints.set(value, level)
}

#[allow(clippy::too_many_arguments)]
fn validate(
    command: Command,
    paths: &[String],
    manifest: Option<&str>,
    locked: bool,
    relaxed: bool,
    ast: bool,
    watch: bool,
    watch_timeout_ns: Option<u64>,
    dry_run: bool,
    online: &OnlineOptions,
    fmt_check: bool,
    build: &Request,
    test: &TestOptions,
) -> Result<(), String> {
    let build_only = build.name.is_some()
        || !build.emit.is_empty()
        || build.target.is_some()
        || build.clang.is_some()
        || build.runtime.is_some()
        || build.got_manifest.is_some()
        || build.cold
        || !build.renames.is_empty()
        || build.allow_untracked_removal;
    if build_only
        && !matches!(
            command,
            Command::Build | Command::Metadata | Command::Run | Command::Test
        )
    {
        return Err(
            "build options are only valid with `stc build`, `stc metadata`, `stc run`, or `stc test`"
                .into(),
        );
    }
    if build.mode_override.is_some()
        && !matches!(
            command,
            Command::Build
                | Command::Metadata
                | Command::Run
                | Command::Test
                | Command::Clean
                | Command::OnlineDownload
                | Command::OnlineDiff
        )
    {
        return Err(
            "`--debug` and `--release` are only valid with `stc build`, `stc metadata`, `stc run`, \
             `stc test`, `stc clean`, or `stc online`"
                .into(),
        );
    }
    if command == Command::Test {
        if build.mode_override == Some(BuildMode::Release) {
            return Err(
                "STC-E-PROJECT-010: `stc test` always builds the hook-free debug form and rejects \
                 `--release`"
                    .into(),
            );
        }
        if build.name.is_some() || !build.emit.is_empty() {
            return Err(
                "`stc test` builds one test artifact per test; `--name`/`--emit` are not valid"
                    .into(),
            );
        }
        if build.got_manifest.is_some()
            || build.cold
            || !build.renames.is_empty()
            || build.allow_untracked_removal
        {
            return Err(
                "`stc test` builds cold test artifacts and never uses a baseline; \
                 `--got-manifest`, `--cold`, `--rename`, and `--allow-untracked-removal` are not valid"
                    .into(),
            );
        }
    }
    if command == Command::Run && (build.name.is_some() || !build.emit.is_empty()) {
        return Err(
            "`stc run` builds `exe,module,metadata`; `--name`/`--emit` are not valid".into(),
        );
    }
    let run_only = build.run.cycles.is_some()
        || build.run.cycle_time_ns.is_some()
        || build.run.state_file.is_some()
        || build.run.reset_retain
        || build.run.no_retain;
    if run_only && command != Command::Run {
        return Err(
            "`--cycles`, `--cycle-time`, `--state-file`, and the retain options are only valid \
             with `stc run`"
                .into(),
        );
    }
    let io_override = build.run.io.is_some() || build.run.io_file.is_some();
    if io_override && !matches!(command, Command::Run | Command::Test) {
        return Err("`--io` and `--io-file` are only valid with `stc run` or `stc test`".into());
    }
    if (test.list || test.no_run || test.timeout_ns.is_some() || test.nocapture)
        && command != Command::Test
    {
        return Err(
            "`--list`, `--no-run`, `--timeout`, and `-- --nocapture` are only valid with `stc test`"
                .into(),
        );
    }
    if build.out_dir.is_some()
        && !matches!(
            command,
            Command::Build | Command::Metadata | Command::Clean | Command::Run | Command::Test
        )
    {
        return Err(
            "`--out-dir` is only valid with `stc build`, `stc metadata`, `stc clean`, `stc run`, or `stc test`"
                .into(),
        );
    }
    if ast && command != Command::Parse {
        return Err("`--ast` is only valid with `stc parse`".into());
    }
    if watch && command != Command::Run {
        return Err("`--watch` is only valid with `stc run`".into());
    }
    if watch_timeout_ns.is_some() && !(command == Command::Run && watch) {
        return Err("`--watch-timeout` requires `stc run --watch`".into());
    }
    if watch && build.cold {
        return Err(
            "`--cold` cannot be combined with `--watch`; the watcher deploys online".into(),
        );
    }
    if dry_run && command != Command::Rename {
        return Err("`--dry-run` is only valid with `stc rename`".into());
    }
    if online.out.is_some() && command != Command::OnlineDownload {
        return Err("`--out` is only valid with `stc online download`".into());
    }
    if online.binary.is_some() && command != Command::OnlineDownload {
        return Err("`--binary` is only valid with `stc online download`".into());
    }
    if online.baseline.is_some() && command != Command::OnlineDiff {
        return Err("`--baseline` is only valid with `stc online diff`".into());
    }
    let online_command = matches!(command, Command::OnlineDownload | Command::OnlineDiff);
    if online.host.is_some() && !online_command {
        return Err("`--host` is only valid with `stc online download/diff`".into());
    }
    if online.nonce.is_some() && !online_command {
        return Err("`--nonce` is only valid with `stc online download/diff`".into());
    }
    if online.store.is_some()
        && !matches!(command, Command::OnlineDownload | Command::OnlineDiff)
        && !(command == Command::Run && watch)
    {
        return Err(
            "`--store` is only valid with `stc online download/diff` or `stc run --watch`".into(),
        );
    }
    if fmt_check && command != Command::Fmt {
        return Err("`--check` is only valid with `stc fmt`".into());
    }
    if relaxed
        && !matches!(
            command,
            Command::Parse | Command::Check | Command::Build | Command::Metadata | Command::Test
        )
    {
        return Err("`--relaxed` is only valid with parse, check, build, metadata, or test".into());
    }
    if manifest.is_some() {
        if !matches!(
            command,
            Command::Check
                | Command::Build
                | Command::Clean
                | Command::Metadata
                | Command::Run
                | Command::Test
                | Command::Fmt
                | Command::OnlineDownload
                | Command::OnlineDiff
                | Command::Rename
        ) {
            return Err("`--manifest-path` is not valid with this command".into());
        }
        if !paths.is_empty() && matches!(command, Command::Check | Command::Build | Command::Fmt) {
            return Err("`--manifest-path` cannot be combined with input files".into());
        }
    }
    if locked
        && !matches!(
            command,
            Command::Check
                | Command::Build
                | Command::Clean
                | Command::Metadata
                | Command::Run
                | Command::Test
                | Command::Fmt
                | Command::OnlineDownload
                | Command::OnlineDiff
                | Command::Rename
        )
    {
        return Err("`--locked` is not valid with this command".into());
    }
    if locked
        && !paths.is_empty()
        && matches!(command, Command::Check | Command::Build | Command::Fmt)
    {
        return Err("`--locked` requires a project manifest".into());
    }

    let path_count = paths.len();
    let allowed = match command {
        Command::Parse | Command::New => path_count == 1,
        Command::Rename => path_count == 2,
        Command::OnlineDiff => path_count == 1,
        // Test positional arguments are substring filters.
        Command::Check | Command::Build | Command::Fmt | Command::Test => true,
        Command::Run
        | Command::Clean
        | Command::Metadata
        | Command::Compliance
        | Command::OnlineDownload => path_count == 0,
    };
    if !allowed {
        return Err(match command {
            Command::Parse => "`stc parse` accepts exactly one input file".into(),
            Command::New => "`stc new` accepts exactly one project path".into(),
            Command::Rename => "`stc rename` accepts `<old> <new>`".into(),
            Command::OnlineDiff => "`stc online diff` accepts one candidate manifest".into(),
            Command::Check | Command::Build | Command::Fmt | Command::Test => {
                "unexpected positional arguments".into()
            }
            _ => format!(
                "`stc {}` does not accept positional arguments",
                command_name(command)
            ),
        });
    }
    Ok(())
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("`{option}` needs a value"))
}

fn parse_rename(value: &str) -> Result<(String, String), String> {
    let Some((old, new)) = value.split_once('=') else {
        return Err(format!("`--rename` expects `old=new`, found `{value}`"));
    };
    if old.is_empty() || new.is_empty() {
        return Err(format!(
            "`--rename` expects non-empty keys, found `{value}`"
        ));
    }
    Ok((old.to_owned(), new.to_owned()))
}

fn parse_cycles(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid cycle count `{value}` (expected a non-negative integer)"))
}

fn parse_cycle_time(value: &str) -> Result<u64, String> {
    stc_manifest::parse_duration(value)
        .map(stc_manifest::DurationNs::as_nanos)
        .ok_or_else(|| {
            format!(
                "invalid cycle time `{value}`; expected an integer and one of `ns`, `us`, `ms`, `s`, `m`, or `h`"
            )
        })
}

fn parse_io(value: &str) -> Result<String, String> {
    match value {
        "null" | "memory" | "file" => Ok(value.to_owned()),
        other => Err(format!(
            "unknown I/O backend `{other}` (expected `null`, `memory`, or `file`)"
        )),
    }
}

fn parse_emit(value: &str) -> Result<std::collections::BTreeSet<Emit>, String> {
    let mut emit = std::collections::BTreeSet::new();
    for part in value.split(',') {
        let kind = match part {
            "ir" => Emit::Ir,
            "obj" => Emit::Obj,
            "module" => Emit::Module,
            "exe" => Emit::Exe,
            "lib" => Emit::Lib,
            "metadata" => Emit::Metadata,
            "" => continue,
            other => {
                return Err(format!(
                    "unknown emit kind `{other}` (expected ir, obj, module, exe, lib, or metadata)"
                ));
            }
        };
        emit.insert(kind);
    }
    if emit.is_empty() {
        return Err("`--emit` needs at least one artifact".into());
    }
    Ok(emit)
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "human" => Ok(Format::Human),
        "json" => Ok(Format::Json),
        "short" => Ok(Format::Short),
        other => Err(format!(
            "unsupported message format `{other}` (expected `human`, `json`, or `short`)"
        )),
    }
}

fn command_name(command: Command) -> &'static str {
    match command {
        Command::Parse => "parse",
        Command::Check => "check",
        Command::Build => "build",
        Command::New => "new",
        Command::Run => "run",
        Command::Test => "test",
        Command::Fmt => "fmt",
        Command::Clean => "clean",
        Command::Metadata => "metadata",
        Command::Compliance => "compliance",
        Command::OnlineDownload => "online download",
        Command::OnlineDiff => "online diff",
        Command::Rename => "rename",
    }
}

fn print_help() {
    println!(
        "stc {} — IEC 61131-3 Structured Text compiler\n\
         \n\
         Usage:\n\
         \x20 stc parse <file> [--ast] [--relaxed] [--message-format human|json|short]\n\
         \x20 stc new <path>\n\
         \x20 stc check [<file>...] [--manifest-path <stc.toml|dir>] [--locked]\n\
         \x20           [--relaxed] [--message-format human|json|short] [-v|-vv] [-q]\n\
         \x20 stc build [<file>...] [--manifest-path <stc.toml|dir>] [--locked]\n\
         \x20           [--debug|--release]\n\
         \x20           [--name <name>] [--emit ir,obj,module,exe,lib,metadata]\n\
         \x20           [--out-dir <dir>] [--target <triple>] [--clang <path>]\n\
         \x20           [--runtime <path>] [--got-manifest <path>] [--cold]\n\
         \x20           [--rename old=new]... [--allow-untracked-removal]\n\
         \x20           [--relaxed] [--message-format human|json|short] [-v|-vv] [-q]\n\
         \x20 stc clean [--manifest-path <stc.toml|dir>] [--out-dir <dir>] [--locked]\n\
         \x20           [--debug|--release]\n\
         \x20 stc metadata [--manifest-path <stc.toml|dir>] [--locked]\n\
         \x20           [--debug|--release] [--message-format human|json] [-v|-vv]\n\
         \x20 stc compliance [--message-format human|json]\n\
         \x20 stc run [--manifest-path <stc.toml|dir>] [--locked] [--cycles <n>]\n\
         \x20         [--debug|--release]\n\
         \x20         [--cycle-time <duration>] [--io null|memory|file]\n\
         \x20         [--io-file <path>] [--state-file <path>] [--reset-retain]\n\
         \x20         [--no-retain] [--target <triple>] [--clang <path>]\n\
         \x20         [--runtime <path>] [-v|-vv] [-q]\n\
         \x20 stc run --watch [--store <dir>] [--watch-timeout <duration>]\n\
         \x20         [--target <triple>] [--clang <path>] [--runtime <path>] [-v|-vv]\n\
         \x20 stc test [<filter>...] [--manifest-path <stc.toml|dir>] [--locked]\n\
         \x20          [--list] [--no-run] [--timeout <duration>]\n\
         \x20          [--io null|memory|file] [--io-file <path>]\n\
         \x20          [--target <triple>] [--clang <path>] [--runtime <path>]\n\
         \x20          [--out-dir <dir>] [--relaxed] [--message-format human|json]\n\
         \x20          [-v|-vv] [-q] [-- --nocapture]\n\
         \x20 stc fmt [<path>...] [--check] [--manifest-path <stc.toml|dir>]\n\
         \x20         [--locked] [--message-format human|json|short] [-v|-vv] [-q]\n\
         \x20 stc online download [--out <path>] [--binary <path>] [--store <dir>]\n\
         \x20         [--host <endpoint>] [--nonce <token>] [--manifest-path <stc.toml|dir>]\n\
         \x20         [--locked] [--debug|--release] [--message-format human|json] [-v|-vv]\n\
         \x20 stc online diff <candidate> [--baseline <path>] [--store <dir>]\n\
         \x20         [--host <endpoint>] [--nonce <token>] [--manifest-path <stc.toml|dir>]\n\
         \x20         [--locked] [--debug|--release]\n\
         \x20         [--message-format human|json|short] [-v|-vv]\n\
         \x20 stc rename <old> <new> [--dry-run] [--manifest-path <stc.toml|dir>] [--locked]\n\
         \x20 stc --version\n\
         \x20 stc --help\n\
         \n\
         Common options: --message-format human|json|short, --color auto|always|never,\n\
         -q, -v, -vv, -D <lint>, -W <lint>, -A <lint>, --deny warnings.\n\
         Lint levels: `warnings` or a documented STC-* diagnostic code.\n\
         \n\
         `parse` lexes and parses one file. `check` lexes, parses, resolves, and\n\
         type-checks one or more files as one in-memory package. `build` also\n\
         emits a loadable ABI 2 module and, when requested, its manifest, a thin\n\
         executable launcher, or the embedding host library through clang.\n\
         Without input files (or with `--manifest-path`), `check`, `build`,\n\
         `clean`, and `metadata` load `stc.toml`, compile the package graph in\n\
         dependency order, and report package-qualified diagnostics. The\n\
         file-based form is the explicit manifest-free bridge. Project builds\n\
         default to `--emit module`, the debug build form, and\n\
         `target/stc/debug`. `--release` selects optimized, stripped,\n\
         hook-free artifacts in `target/stc/release`; `--debug` forces the\n\
         debug form. The CLI flags override `[build].debug`, which overrides\n\
         `[debug].default`. `stc test` always builds the hook-free debug form\n\
         and rejects `--release`, `--debug-server`, and `--debug-protocol`.\n\
         The manifest-free bridge keeps `--emit ir` and a sanitized file stem. `fmt` formats the\n\
         given files, or the root package's sources and test sources when no\n\
         paths are given, with the fixed lossless-CST style; `--check` writes\n\
         nothing and exits 1 when a file would change. `online download`\n\
         exports the committed runtime manifest store as a canonical download\n\
         envelope; `online diff` compares a candidate manifest with the active\n\
         baseline (or `--baseline`) and exits 0 compatible, 1 incompatible,\n\
         2 usage/parse. `rename` rewrites a sema-resolved declaration and its\n\
         references and records the mapping in `stc.renames.toml`.\n\
         Exit codes: 0 success, 1 compile error, 2 usage/manifest/toolchain error,\n\
         3 internal error, 4 runtime trap, 130 interrupted.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(args: &[&str]) -> Invocation {
        let args = args
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();
        parse(&args)
            .unwrap_or_else(|error| panic!("parse failed: {error}"))
            .expect("not a help/version invocation")
    }

    #[test]
    fn online_commands_accept_both_build_modes() {
        assert_eq!(
            invocation(&["online", "download", "--release"])
                .build
                .mode_override,
            Some(BuildMode::Release)
        );
        assert_eq!(
            invocation(&["online", "diff", "candidate.json", "--debug"])
                .build
                .mode_override,
            Some(BuildMode::Debug)
        );
    }

    #[test]
    fn debug_and_release_flags_conflict() {
        let args = ["build", "--debug", "--release"]
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();
        let error = match parse(&args) {
            Ok(_) => panic!("both modes must fail"),
            Err(error) => error,
        };
        assert!(error.starts_with("STC-E-PROJECT-010: "), "{error}");
    }

    #[test]
    fn test_rejects_release_and_debug_session_flags() {
        for args in [
            ["test", "--release"].as_slice(),
            ["test", "--debug-server"].as_slice(),
            ["test", "--debug-protocol", "pipe"].as_slice(),
        ] {
            let args = args
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>();
            let error = match parse(&args) {
                Ok(_) => panic!("`stc test` must reject the flag"),
                Err(error) => error,
            };
            assert!(error.starts_with("STC-E-PROJECT-010: "), "{error}");
        }
    }
}
