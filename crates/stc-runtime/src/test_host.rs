//! Test-host entry point for `stc test` (Testing and Verification §2.5).
//!
//! The test host is a normal executable launcher whose `main` calls
//! [`stc_rt_test_main`]. It loads one test module, runs the requested number
//! of scans on a mock clock, records assertion failures instead of trapping,
//! and prints exactly one structured result line to stdout:
//!
//! ```json
//! {"kind":"stc-test-result","name":"TimerBlink","status":"pass",
//!  "assertions":[],"trap":null,"cycles":25}
//! ```
//!
//! The parent (`stc test`) owns the timeout, process isolation, and the public
//! report shape; this module only reports what one test process observed.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};

use crate::host::{Candidate, Host, LoadMode};
use crate::host_api;
use crate::module_info;
use crate::run::{self, IoBackend, RunLoopError, RunOptions};
use crate::store::ManifestStore;
use crate::{MismatchPolicy, RetainConfig, host_platform};

/// Parsed test-host arguments.
struct TestHostOptions {
    name: String,
    cycles: u64,
    mock_clock_ns: i64,
    io: IoBackend,
    io_file: Option<PathBuf>,
    expect_trap: Option<String>,
    assertions: Option<PathBuf>,
    passed_root: Option<stc_got::GotId>,
    decl_file: Option<String>,
    decl_line: Option<u32>,
    decl_column: Option<u32>,
    module: Option<String>,
}

/// One recorded assertion site from the driver's sidecar.
struct AssertSite {
    message: String,
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}

/// One failure reported in the structured result.
struct Failure {
    message: String,
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}

impl Failure {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            file: None,
            line: None,
            column: None,
        }
    }
}

/// Test-host entry point called by the generated test launcher.
///
/// # Safety
///
/// `argv` must point to `argc` valid, NUL-terminated strings. `module_path`
/// must be null or point to a NUL-terminated string. Both stay valid for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stc_rt_test_main(
    argc: i32,
    argv: *const *const c_char,
    module_path: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the ABI contract requires `argv`/`module_path` to be valid
        // for the duration of the call.
        unsafe { execute_test(argc, argv, module_path) }
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            eprintln!("stc: internal test-host panic");
            3
        }
    }
}

unsafe fn execute_test(argc: i32, argv: *const *const c_char, module_path: *const c_char) -> i32 {
    let args = crate::collect_args(argc, argv);
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("stc: {message}");
            print_result(&ResultLine::internal_error(&message));
            return 2;
        }
    };
    let Some(module_argument) = options.module.clone().or_else(|| {
        if module_path.is_null() {
            None
        } else {
            // SAFETY: the ABI contract requires a NUL-terminated string.
            Some(
                unsafe { CStr::from_ptr(module_path) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }) else {
        let message = "no module path was supplied";
        eprintln!("stc: {message}");
        print_result(&ResultLine::internal_error(message));
        return 2;
    };
    let sites = options
        .assertions
        .as_deref()
        .map(load_assertion_sites)
        .unwrap_or_default();
    let outcome = run_test(&options, &module_argument, &sites);
    print_result(&outcome);
    0
}

fn run_test(
    options: &TestHostOptions,
    module_argument: &str,
    sites: &BTreeMap<u32, AssertSite>,
) -> ResultLine {
    let resolved = crate::resolve_module_path(module_argument);
    let platform = host_platform();
    let manifest = module_info::manifest_path(&resolved);
    let mut host = Host::new(platform.clone(), ManifestStore::memory());
    host_api::clear_last_trap();
    let candidate = match Candidate::open(&platform, &resolved, &manifest) {
        Ok(candidate) => candidate,
        Err(error) => {
            return ResultLine::internal_error(format!(
                "cannot load test module `{}`: {}: {}",
                resolved.display(),
                error.code,
                error.message
            ));
        }
    };
    // Test builds never load or save retained state (Testing §2.5).
    host.configure_retain(RetainConfig {
        enabled: false,
        file: None,
        policy: MismatchPolicy::Error,
        reset: false,
    });
    host.configure_io(Some(options.io), options.io_file.clone());
    if let Err(error) = host.load(candidate, LoadMode::Cold) {
        let failures = host_api::last_trap().map_or_else(
            || {
                vec![Failure::plain(format!(
                    "cannot install test module: {error}"
                ))]
            },
            |trap| {
                vec![Failure::plain(format!(
                    "test module trapped while loading: {}",
                    trap_name(trap.code)
                ))]
            },
        );
        return ResultLine::new(&options.name, Status::Fail, failures, None, 0);
    }

    host_api::begin_assert_recording();
    let run = RunOptions {
        cycles: options.cycles.max(1),
        cycle_time_ns: None,
        io: Some(options.io),
        io_file: options.io_file.clone(),
        state_file: None,
        reset_retain: false,
        no_retain: true,
        mock_clock_ns: Some(options.mock_clock_ns),
        status: false,
        ..RunOptions::default()
    };
    let outcome = run::run_loop(&mut host, &run);
    let records = host_api::end_assert_recording();
    let mut failures: Vec<Failure> = records
        .iter()
        .map(|record| failure_for_site(record.span_id, sites))
        .collect();
    let cycles: u64;
    let mut trap = None;
    let mut trap_record = None;
    match outcome {
        Ok(count) => cycles = count,
        Err(error) => {
            cycles = error.cycles();
            match &error {
                RunLoopError::Cycle {
                    trap: true, error, ..
                } => {
                    trap_record = host_api::last_trap();
                    trap = trap_record.map(|record| trap_name(record.code));
                    if trap.is_none() {
                        failures.push(Failure::plain(format!(
                            "test trapped: {}: {}",
                            error.code, error.message
                        )));
                    }
                }
                RunLoopError::Cycle { error, .. } => failures.push(Failure::plain(format!(
                    "runtime error: {}: {}",
                    error.code, error.message
                ))),
                RunLoopError::Interrupted { .. } => {
                    failures.push(Failure::plain("test was interrupted"));
                }
                RunLoopError::Setup { message } => {
                    failures.push(Failure::plain(format!("runtime setup error: {message}")))
                }
            }
        }
    }

    if let Some(expected) = options.expect_trap.as_deref() {
        let expected = normalize_trap(expected);
        match &trap {
            Some(actual) if normalize_trap(actual) == expected => {
                // A matching expected trap passes; recorded assertions before
                // the trap do not change the outcome.
                return ResultLine::new(&options.name, Status::Pass, Vec::new(), None, cycles);
            }
            Some(actual) => {
                return ResultLine::new(
                    &options.name,
                    Status::Trap,
                    failures,
                    Some(actual.clone()),
                    cycles,
                );
            }
            None => {
                failures.push(declaration_failure(
                    options,
                    &format!("expected trap {expected} but the test completed normally"),
                ));
                return ResultLine::new(&options.name, Status::Fail, failures, None, cycles);
            }
        }
    }

    if let Some(root) = options.passed_root
        && !host
            .root_storage(root)
            .and_then(|bytes| bytes.first())
            .is_some_and(|byte| *byte != 0)
    {
        failures.push(declaration_failure(
            options,
            "PASSED was not TRUE at the end of the test",
        ));
    }
    if let Some(record) = trap_record
        && trap.is_none()
    {
        failures.push(Failure::plain(format!(
            "module trapped at span {}",
            record.span_id
        )));
    }
    // Flush the `file` I/O backend and release module resources. The default
    // memory backend has nothing to flush.
    if let Err(error) = host.shutdown() {
        failures.push(Failure::plain(format!(
            "test host shutdown failed: {}: {}",
            error.code, error.message
        )));
    }
    let status = if trap.is_some() {
        Status::Trap
    } else if failures.is_empty() {
        Status::Pass
    } else {
        Status::Fail
    };
    ResultLine::new(&options.name, status, failures, trap, cycles)
}

fn failure_for_site(span_id: u32, sites: &BTreeMap<u32, AssertSite>) -> Failure {
    match sites.get(&span_id) {
        Some(site) => Failure {
            message: site.message.clone(),
            file: site.file.clone(),
            line: site.line,
            column: site.column,
        },
        None => Failure::plain(format!("assertion failed (span {span_id})")),
    }
}

fn declaration_failure(options: &TestHostOptions, message: &str) -> Failure {
    Failure {
        message: message.to_owned(),
        file: options.decl_file.clone(),
        line: options.decl_line,
        column: options.decl_column,
    }
}

fn normalize_trap(name: &str) -> String {
    let upper = name.trim().to_ascii_uppercase();
    upper.strip_prefix("STC_TRAP_").unwrap_or(&upper).to_owned()
}

fn trap_name(code: i32) -> String {
    format!("STC_TRAP_{}", crate::trap_name(code))
}

fn parse_args(args: &[String]) -> Result<TestHostOptions, String> {
    let mut options = TestHostOptions {
        name: "test".into(),
        cycles: 1,
        mock_clock_ns: 0,
        io: IoBackend::Memory,
        io_file: None,
        expect_trap: None,
        assertions: None,
        passed_root: None,
        decl_file: None,
        decl_line: None,
        decl_column: None,
        module: None,
    };
    let mut index = usize::from(!args.is_empty());
    while index < args.len() {
        let argument = args[index].as_str();
        let mut value = |option: &str| -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("`{option}` requires a value"))
        };
        match argument {
            "--name" => options.name = value("--name")?,
            "--cycles" => {
                let text = value("--cycles")?;
                options.cycles = text
                    .parse::<u64>()
                    .map_err(|_| format!("invalid cycle count `{text}`"))?;
            }
            "--mock-clock" => {
                let text = value("--mock-clock")?;
                let ns = run::parse_duration_ns(&text)
                    .ok_or_else(|| format!("invalid mock clock `{text}`"))?;
                options.mock_clock_ns = i64::try_from(ns).unwrap_or(i64::MAX);
            }
            "--io" => {
                let text = value("--io")?;
                options.io = IoBackend::parse(&text).ok_or_else(|| {
                    format!("unknown I/O backend `{text}` (expected null, memory, or file)")
                })?;
            }
            "--io-file" => options.io_file = Some(PathBuf::from(value("--io-file")?)),
            "--expect-trap" => options.expect_trap = Some(value("--expect-trap")?),
            "--assertions" => options.assertions = Some(PathBuf::from(value("--assertions")?)),
            "--decl-file" => options.decl_file = Some(value("--decl-file")?),
            "--decl-line" => {
                let text = value("--decl-line")?;
                options.decl_line = Some(
                    text.parse::<u32>()
                        .map_err(|_| format!("invalid declaration line `{text}`"))?,
                );
            }
            "--decl-column" => {
                let text = value("--decl-column")?;
                options.decl_column = Some(
                    text.parse::<u32>()
                        .map_err(|_| format!("invalid declaration column `{text}`"))?,
                );
            }
            "--passed-root" => {
                let text = value("--passed-root")?;
                options.passed_root = Some(stc_got::GotId::parse(&text).map_err(|_| {
                    format!("invalid passed root `{text}` (expected a 0x-prefixed GotId)")
                })?);
            }
            "--module" => options.module = Some(value("--module")?),
            other => return Err(format!("unknown test-host option `{other}`")),
        }
        index += 1;
    }
    Ok(options)
}

fn load_assertion_sites(path: &Path) -> BTreeMap<u32, AssertSite> {
    let mut sites = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return sites;
    };
    for line in text.lines() {
        let mut fields = line.splitn(5, '\t');
        let Some(id) = fields.next().and_then(|field| field.parse::<u32>().ok()) else {
            continue;
        };
        let line_number = fields
            .next()
            .and_then(|field| field.parse::<u32>().ok())
            .filter(|value| *value > 0);
        let column = fields
            .next()
            .and_then(|field| field.parse::<u32>().ok())
            .filter(|value| *value > 0);
        let file = fields.next().map(unescape).filter(|file| !file.is_empty());
        let message = fields.next().map(unescape).unwrap_or_default();
        sites.insert(
            id,
            AssertSite {
                message,
                file,
                line: line_number,
                column,
            },
        );
    }
    sites
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Trap,
}

impl Status {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Trap => "trap",
        }
    }
}

struct ResultLine {
    name: String,
    status: &'static str,
    failures: Vec<Failure>,
    trap: Option<String>,
    cycles: u64,
}

impl ResultLine {
    fn new(
        name: &str,
        status: Status,
        failures: Vec<Failure>,
        trap: Option<String>,
        cycles: u64,
    ) -> Self {
        Self {
            name: name.to_owned(),
            status: status.as_str(),
            failures,
            trap,
            cycles,
        }
    }

    fn internal_error(message: impl Into<String>) -> Self {
        Self {
            name: "test".into(),
            status: "fail",
            failures: vec![Failure::plain(format!(
                "internal test-host error: {}",
                message.into()
            ))],
            trap: None,
            cycles: 0,
        }
    }
}

fn print_result(result: &ResultLine) {
    let mut out = String::new();
    out.push_str("{\"kind\":\"stc-test-result\",\"name\":");
    push_json_string(&mut out, &result.name);
    out.push_str(",\"status\":");
    push_json_string(&mut out, result.status);
    out.push_str(",\"assertions\":[");
    for (index, failure) in result.failures.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"message\":");
        push_json_string(&mut out, &failure.message);
        if let Some(file) = &failure.file {
            out.push_str(",\"file\":");
            push_json_string(&mut out, file);
        }
        if let Some(line) = failure.line {
            out.push_str(&format!(",\"line\":{line}"));
        }
        if let Some(column) = failure.column {
            out.push_str(&format!(",\"column\":{column}"));
        }
        out.push('}');
    }
    out.push_str("],\"trap\":");
    match &result.trap {
        Some(trap) => push_json_string(&mut out, trap),
        None => out.push_str("null"),
    }
    out.push_str(&format!(",\"cycles\":{}}}\n", result.cycles));
    print!("{out}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}
