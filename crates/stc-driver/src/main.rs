//! The `stc` command-line driver (M2 skeleton).
//!
//! M2 provides `stc parse` and `stc check`. Both lex and parse one file;
//! `check` is parse-only until semantic analysis arrives in M3. The driver
//! owns the exit-code contract, human/JSON diagnostics, and the panic boundary
//! from CLI-and-Manifest §1.3–§1.4.

#![forbid(unsafe_code)]

use std::any::Any;
use std::ffi::OsString;
use std::process::ExitCode;

use stc_diag::{DiagnosticSink, render_human, render_json};
use stc_lexer::tokenize_bytes;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_span::FileId;
use stc_span::SourceMap;
use stc_syntax::{dump_cst, dump_typed_ast};

const FILE: FileId = FileId::new(0);
const INTERNAL_ERROR: &str = "STC-I-0001";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Parse,
    Check,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Human,
    Json,
}

struct Invocation {
    command: Command,
    path: String,
    format: Format,
    relaxed: bool,
    ast: bool,
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

    let bytes = match std::fs::read(&invocation.path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("stc: cannot read `{}`: {error}", invocation.path);
            return ExitCode::from(2);
        }
    };

    let mut diagnostics = DiagnosticSink::new();
    let lexed = tokenize_bytes(FILE, &bytes, &mut diagnostics);
    let mut sources = SourceMap::new();
    let file = sources.add_file(invocation.path.clone(), lexed.text.clone());
    debug_assert_eq!(file, FILE);

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

    report(&invocation, &parsed, &diagnostics, &sources);

    if diagnostics.has_errors() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn report(
    invocation: &Invocation,
    parsed: &ParsedFile,
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
            if invocation.command == Command::Parse {
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
        other => return Err(format!("unknown command `{other}`")),
    };
    index += 1;

    let mut format = Format::Human;
    let mut relaxed = false;
    let mut ast = false;
    let mut path: Option<String> = None;
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
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            other => {
                if path.is_some() {
                    return Err(format!("unexpected argument `{other}`"));
                }
                path = Some(other.to_owned());
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| "missing input file".to_string())?;
    if command == Command::Check && ast {
        return Err("`--ast` is only valid with `stc parse`".into());
    }
    Ok(Some(Invocation {
        command,
        path,
        format,
        relaxed,
        ast,
    }))
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
         \x20 stc check <file> [--relaxed] [--message-format human|json]\n\
         \x20 stc --version\n\
         \x20 stc --help\n\
         \n\
         M2 commands lex and parse one file. `check` is parse-only until M3.\n\
         Exit codes: 0 success, 1 compile error, 2 usage error, 3 internal error.",
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
