//! The `stc` command-line driver.
//!
//! M4 provides `stc parse <file>` (lex/parse only), `stc check <file>...`
//! (lex, parse, resolve, and type-check), and `stc build <file>...`
//! (check plus IR/object/executable/library/metadata emission). The driver
//! owns the exit-code contract, human/JSON diagnostics, and the panic
//! boundary from CLI-and-Manifest §1.3–§1.4.

#![forbid(unsafe_code)]

mod build;

use std::any::Any;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::process::ExitCode;

use stc_diag::{DiagnosticSink, render_human, render_json};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_span::{FileId, SourceMap};
use stc_syntax::{dump_cst, dump_typed_ast};

use crate::build::Emit;

const INTERNAL_ERROR: &str = "STC-I-0001";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Parse,
    Check,
    Build,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Human,
    Json,
}

struct Invocation {
    command: Command,
    paths: Vec<String>,
    format: Format,
    relaxed: bool,
    ast: bool,
    build: build::Request,
}

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
    let invocation = match parse_args(args) {
        Ok(Some(invocation)) => invocation,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("stc: {message}");
            eprintln!("try `stc --help` for usage");
            return ExitCode::from(2);
        }
    };

    let mut paths = invocation.paths.clone();
    if matches!(invocation.command, Command::Check | Command::Build) {
        // File order is not semantic (D-060).
        paths.sort();
    }

    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut parsed_files = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("stc: cannot read `{path}`: {error}");
                return ExitCode::from(2);
            }
        };
        let file = FileId::new(u32::try_from(index).expect("too many input files"));
        let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
        let mapped = sources.add_file(path.clone(), lexed.text.clone());
        debug_assert_eq!(mapped, file);
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
        parsed_files.push(parsed);
    }

    // D-070: do not analyze recovered trees when syntax errors exist.
    let mut analysis = None;
    if matches!(invocation.command, Command::Check | Command::Build) && !diagnostics.has_errors() {
        analysis = Some(stc_sema::analyze(&parsed_files, &mut diagnostics));
    }

    let mut build_result = None;
    if invocation.command == Command::Build
        && !diagnostics.has_errors()
        && let Some(analysis) = &analysis
    {
        build_result = Some(build::run(
            &invocation.build,
            &analysis.hir,
            &paths,
            &mut diagnostics,
            invocation.format == Format::Json,
        ));
    }

    report(&invocation, &parsed_files, &diagnostics, &sources);

    if let Some(code) = build_result
        && code != 0
    {
        return ExitCode::from(code);
    }
    if diagnostics.has_errors() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn report(
    invocation: &Invocation,
    parsed_files: &[ParsedFile],
    diagnostics: &DiagnosticSink,
    sources: &SourceMap,
) {
    match invocation.format {
        Format::Human => {
            for diagnostic in diagnostics.iter() {
                eprint!("{}", render_human(diagnostic, sources));
            }
            if diagnostics.suppressed() > 0 {
                eprintln!(
                    "note: {} further diagnostic(s) suppressed",
                    diagnostics.suppressed()
                );
            }
            if invocation.command == Command::Parse
                && let Some(parsed) = parsed_files.first()
            {
                print!("{}", dump_cst(parsed.tree()));
                if invocation.ast {
                    print!("{}", dump_typed_ast(parsed.tree()));
                }
            }
        }
        Format::Json => {
            for diagnostic in diagnostics.iter() {
                println!("{}", render_json(diagnostic, sources));
            }
            // The dump is not emitted in JSON mode so stdout stays one JSON
            // object per line (D-051).
        }
    }
}

fn parse_args(args: &[String]) -> Result<Option<Invocation>, String> {
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
        other => return Err(format!("unknown command `{other}`")),
    };
    index += 1;

    let mut format = Format::Human;
    let mut relaxed = false;
    let mut ast = false;
    let mut paths = Vec::new();
    let mut build = build::Request::default();
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
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
            "--message-format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--message-format` needs a value".to_string())?;
                format = parse_format(value)?;
            }
            other if other.starts_with("--message-format=") => {
                format = parse_format(&other["--message-format=".len()..])?;
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
            "--out-dir" => build.out_dir = Some(next_value(args, &mut index, "--out-dir")?),
            other if other.starts_with("--out-dir=") => {
                build.out_dir = Some(other["--out-dir=".len()..].to_owned());
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
            "-v" | "--verbose" => build.verbose = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            other => paths.push(other.to_owned()),
        }
        if command != Command::Build && build_option_present(&build) {
            return Err("build options are only valid with `stc build`".into());
        }
        index += 1;
    }

    if paths.is_empty() {
        return Err("missing input file".into());
    }
    if command == Command::Parse && paths.len() != 1 {
        return Err("`stc parse` accepts exactly one input file".into());
    }
    if command != Command::Parse && ast {
        return Err("`--ast` is only valid with `stc parse`".into());
    }
    if command == Command::Build && build.emit.is_empty() {
        build.emit.insert(Emit::Ir);
    }
    Ok(Some(Invocation {
        command,
        paths,
        format,
        relaxed,
        ast,
        build,
    }))
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("`{option}` needs a value"))
}

fn build_option_present(build: &build::Request) -> bool {
    build.name.is_some()
        || !build.emit.is_empty()
        || build.out_dir.is_some()
        || build.target.is_some()
        || build.clang.is_some()
        || build.runtime.is_some()
        || build.verbose
}

fn parse_emit(value: &str) -> Result<BTreeSet<Emit>, String> {
    let mut emit = BTreeSet::new();
    for part in value.split(',') {
        let kind = match part {
            "ir" => Emit::Ir,
            "obj" => Emit::Obj,
            "exe" => Emit::Exe,
            "lib" => Emit::Lib,
            "metadata" => Emit::Metadata,
            "" => continue,
            other => {
                return Err(format!(
                    "unknown emit kind `{other}` (expected ir, obj, exe, lib, or metadata)"
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
        other => Err(format!(
            "unsupported message format `{other}` (expected `human` or `json`)"
        )),
    }
}

fn print_help() {
    println!(
        "stc {} — IEC 61131-3 Structured Text compiler\n\
         \n\
         Usage:\n\
         \x20 stc parse <file> [--ast] [--relaxed] [--message-format human|json]\n\
         \x20 stc check <file>... [--relaxed] [--message-format human|json]\n\
         \x20 stc build <file>... [--name <name>] [--emit ir,obj,exe,lib,metadata]\n\
         \x20             [--out-dir <dir>] [--target <triple>] [--clang <path>]\n\
         \x20             [--runtime <path>] [--relaxed] [--message-format human|json] [-v]\n\
         \x20 stc --version\n\
         \x20 stc --help\n\
         \n\
         `parse` lexes and parses one file. `check` lexes, parses, resolves, and\n\
         type-checks one or more files in one in-memory package. `build` also\n\
         emits IR and, when requested, objects, executables, or static libraries\n\
         through clang.\n\
         Exit codes: 0 success, 1 compile error, 2 usage/toolchain error, 3 internal error.",
        env!("CARGO_PKG_VERSION")
    );
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
