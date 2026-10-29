//! The `stc` command-line driver.
//!
//! M6 phase 3 implements `new`, `check`, `build`, `clean`, `metadata`, and
//! `compliance`; phase 4 adds `run`, phase 6 adds `test`, phase 7 adds `fmt`,
//! and phase 8 adds `rename`, `online download/diff`, and `run --watch`. The
//! driver owns the exit-code contract, human/JSON/short diagnostics, colors,
//! lint levels, and the panic boundary from CLI-and-Manifest §1.3–§1.4.

#![forbid(unsafe_code)]

mod build;
mod clean;
mod cli;
mod compat;
mod compliance;
mod dap;
mod debug;
mod fmt;
mod frames;
mod metadata;
mod new;
mod online;
mod project;
mod rename;
mod report;
mod run;
mod test;
mod watch;

use std::any::Any;
use std::ffi::OsString;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use stc_diag::DiagnosticSink;
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, parse};
use stc_span::{FileId, SourceMap};
use stc_syntax::{dump_cst, dump_typed_ast};

use crate::build::Emit;
use crate::cli::{Command, Invocation};
use crate::project::Mode;
use crate::report::{Log, Reporter, elapsed};

const INTERNAL_ERROR: &str = "STC-I-0001";

fn main() -> ExitCode {
    let suppress_panic_output = std::env::var_os("RUST_BACKTRACE").is_none();
    let previous_hook = std::panic::take_hook();
    if suppress_panic_output {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let result = std::panic::catch_unwind(|| {
        let args: Vec<String> = std::env::args_os()
            .skip(1)
            .map(|argument: OsString| argument.to_string_lossy().into_owned())
            .collect();
        run(&args)
    });
    if suppress_panic_output {
        std::panic::set_hook(previous_hook);
    }
    match result {
        Ok(code) => code,
        Err(payload) => {
            eprintln!(
                "error[{INTERNAL_ERROR}]: internal compiler error: {}",
                panic_message(payload.as_ref())
            );
            eprintln!("  = note: please report this bug with the source file and the command line");
            ExitCode::from(3)
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let invocation = match cli::parse(args) {
        Ok(Some(invocation)) => invocation,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            if let Some(rest) = message.strip_prefix("STC-E-PROJECT-010: ") {
                eprintln!("error[STC-E-PROJECT-010]: {rest}");
            } else {
                eprintln!("stc: {message}");
            }
            eprintln!("try `stc --help` for usage");
            return ExitCode::from(2);
        }
    };
    let log = Log::new(invocation.quiet, invocation.verbosity);
    let mut reporter = Reporter::new(
        invocation.format,
        invocation.color,
        invocation.quiet,
        &invocation.lints,
    );
    let code = match invocation.command {
        Command::Parse => run_parse(&invocation, &mut reporter, log),
        Command::Check | Command::Build => {
            if invocation.paths.is_empty() {
                let mode = if invocation.command == Command::Check {
                    Mode::Check
                } else {
                    Mode::Build
                };
                project::run(
                    mode,
                    invocation.manifest.as_deref(),
                    invocation.locked,
                    invocation.diagnostic_paths,
                    &invocation.build,
                    &invocation.debug,
                    &mut reporter,
                    log,
                )
            } else {
                run_files(&invocation, &mut reporter, log)
            }
        }
        Command::Metadata => project::run(
            Mode::Metadata,
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            &invocation.debug,
            &mut reporter,
            log,
        ),
        Command::New => new::run(&invocation.paths[0], log),
        Command::Clean => clean::run(
            invocation.manifest.as_deref(),
            &invocation.build,
            invocation.clean_test,
            invocation.locked,
            &mut reporter,
            log,
        ),
        Command::Compliance => compliance::run(invocation.format),
        Command::Run if invocation.watch => watch::run(
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            &invocation.debug,
            invocation.online.store.as_deref(),
            invocation.watch_timeout_ns,
            &mut reporter,
            log,
        ),
        Command::Run => project::run(
            Mode::Run,
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            &invocation.debug,
            &mut reporter,
            log,
        ),
        Command::Debug if invocation.debug.attach.is_some() => {
            crate::debug::attach(&invocation.debug, &mut reporter, log)
        }
        Command::Debug => project::run(
            Mode::Debug,
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            &invocation.debug,
            &mut reporter,
            log,
        ),
        Command::Test => test::run(
            invocation.manifest.as_deref(),
            invocation.locked,
            invocation.diagnostic_paths,
            &invocation.build,
            &invocation.test,
            &invocation.paths,
            &mut reporter,
            log,
        ),
        Command::Fmt => fmt::run(
            &invocation.paths,
            invocation.fmt_check,
            invocation.manifest.as_deref(),
            invocation.locked,
            &mut reporter,
            log,
        ),
        Command::OnlineDownload => online::download(
            &invocation.online,
            invocation.manifest.as_deref(),
            invocation.locked,
            &mut reporter,
            log,
        ),
        Command::OnlineDiff => online::diff(
            &invocation.online,
            &invocation.paths[0],
            invocation.manifest.as_deref(),
            invocation.locked,
            &mut reporter,
            log,
        ),
        Command::Rename => rename::run(
            &invocation.paths[0],
            &invocation.paths[1],
            invocation.dry_run,
            invocation.manifest.as_deref(),
            invocation.locked,
            &invocation.build,
            &mut reporter,
            log,
        ),
        // The language server and debug adapter own stdout; they must not go
        // through the diagnostic reporter.
        Command::Lsp => {
            stc_lsp::run_stdio(invocation.manifest.as_deref().map(std::path::Path::new))
        }
        Command::Dap => dap::run(invocation.manifest.as_deref()),
    };
    ExitCode::from(code)
}

fn run_parse(invocation: &Invocation, reporter: &mut Reporter, log: Log) -> u8 {
    let path = &invocation.paths[0];
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("stc: cannot read `{path}`: {error}");
            return 2;
        }
    };
    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let file = FileId::new(0);
    let lex_start = Instant::now();
    let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
    let lex_elapsed = lex_start.elapsed();
    let mapped = sources.add_file(path.clone(), lexed.text.clone());
    debug_assert_eq!(mapped, file);
    let parse_start = Instant::now();
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: invocation.relaxed,
            ..ParseOptions::default()
        },
        &mut diagnostics,
    );
    let parse_elapsed = parse_start.elapsed();
    log.info(format!(
        "lex {}  parse {}",
        duration(lex_elapsed),
        duration(parse_elapsed)
    ));
    reporter.diagnostics(diagnostics.diagnostics(), &sources);
    if diagnostics.suppressed() > 0 {
        reporter.note(format!(
            "note: {} further diagnostic(s) suppressed",
            diagnostics.suppressed()
        ));
    }
    // JSON stays one object per line (D-051); human and short still print the
    // requested CST/AST dump on stdout.
    if !invocation.format.is_json() {
        print!("{}", dump_cst(parsed.tree()));
        if invocation.ast {
            print!("{}", dump_typed_ast(parsed.tree()));
        }
    }
    if diagnostics.has_errors() || reporter.denied() {
        1
    } else {
        0
    }
}

/// The explicit manifest-free bridge: one in-memory package from `<file>...`.
fn run_files(invocation: &Invocation, reporter: &mut Reporter, log: Log) -> u8 {
    let command = invocation.command;
    let mut paths = invocation.paths.clone();
    // File order is not semantic (D-060).
    paths.sort();

    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut parsed_files = Vec::new();
    let mut line_count = 0_usize;
    let mut lex_time = Duration::ZERO;
    let mut parse_time = Duration::ZERO;
    for (index, path) in paths.iter().enumerate() {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("stc: cannot read `{path}`: {error}");
                return 2;
            }
        };
        line_count += bytes.iter().filter(|byte| **byte == b'\n').count();
        let file = FileId::new(u32::try_from(index).expect("too many input files"));
        let lex_start = Instant::now();
        let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
        let lex_elapsed = lex_start.elapsed();
        lex_time += lex_elapsed;
        let mapped = sources.add_file(path.clone(), lexed.text.clone());
        debug_assert_eq!(mapped, file);
        let parse_start = Instant::now();
        let parsed = parse(
            file,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions {
                relaxed_semicolons: invocation.relaxed,
                ..ParseOptions::default()
            },
            &mut diagnostics,
        );
        let parse_elapsed = parse_start.elapsed();
        parse_time += parse_elapsed;
        log.detail(format!(
            "{path} lex {} parse {}",
            duration(lex_elapsed),
            duration(parse_elapsed)
        ));
        parsed_files.push(parsed);
    }
    log.info(format!("sources {} files, {line_count} lines", paths.len()));
    log.info(format!(
        "lex {}  parse {}",
        duration(lex_time),
        duration(parse_time)
    ));

    // D-070: do not analyze recovered trees when syntax errors exist.
    let mut analysis = None;
    if matches!(command, Command::Check | Command::Build) && !diagnostics.has_errors() {
        let analyze_start = Instant::now();
        analysis = Some(stc_sema::analyze(&parsed_files, &mut diagnostics));
        log.info(format!("analyze {}", elapsed(analyze_start)));
    }

    let mut build_result = None;
    if command == Command::Build
        && !diagnostics.has_errors()
        && let Some(analysis) = &analysis
    {
        let mut request = invocation.build.clone();
        let mode = request.mode_override.unwrap_or(build::BuildMode::Debug);
        request.apply_mode(mode);
        if request.emit.is_empty() {
            request.emit.insert(Emit::Ir);
        }
        build_result = Some(build::run(
            &request,
            &analysis.hir,
            &paths,
            &sources,
            &mut diagnostics,
            reporter,
            log,
        ));
    }

    reporter.diagnostics(diagnostics.diagnostics(), &sources);
    if diagnostics.suppressed() > 0 {
        reporter.note(format!(
            "note: {} further diagnostic(s) suppressed",
            diagnostics.suppressed()
        ));
    }
    if let Some(code) = build_result
        && code != 0
    {
        return code;
    }
    if diagnostics.has_errors() || reporter.denied() {
        1
    } else {
        0
    }
}

fn duration(value: Duration) -> String {
    format!("{:.1}ms", value.as_secs_f64() * 1000.0)
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".into()
    }
}
