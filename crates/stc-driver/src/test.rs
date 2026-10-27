//! `stc test`: discovery, test-build mode, and the process-isolated runner
//! (M6 phase 6, Testing and Verification §2).
//!
//! The driver parses the production sources (and the root package's
//! `[test].sources`), discovers `{stc:test}` POUs, and builds one artifact per
//! test into its own directory. Each artifact is a normal ABI 2 module with a
//! synthetic `STC_TEST` configuration and a test-host launcher whose
//! `main` calls the runtime's `stc_rt_test_main`. Tests run sequentially in
//! separate processes; the runner enforces the timeout and parses the
//! structured result line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stc_diag::{DiagnosticSink, Severity};
use stc_hir::{CalleeHir, ConstValue, ExprKindHir, HirCrate, StdFnKind};
use stc_lexer::tokenize_bytes;
use stc_manifest::SyntaxMode;
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{PackageSession, ProjectSession};
use stc_span::{FileId, SourceMap, Span};
use stc_test::{
    SourceFileRef, TestCase, TestStatus, discover, filter_tests, human_failure_lines,
    human_result_line, json_result, summary_line, wrapper_source,
};

use crate::build::{self, Emit, Request};
use crate::cli::TestOptions;
use crate::report::{Log, Reporter};

/// One parsed source file in `FileId` order.
struct SourceEntry {
    package: usize,
    is_test: bool,
    /// Path used for analysis diagnostics and metadata: `<package>/<relative>`.
    display: String,
    /// Package-relative path with forward slashes.
    relative: String,
    /// Path reported in test results (root files use the plain relative path).
    report_path: String,
    file_id: FileId,
    bytes: Vec<u8>,
    parsed: ParsedFile,
}

/// A built test artifact ready to run.
struct BuiltTest {
    test: TestCase,
    exe: PathBuf,
    args: Vec<String>,
}

/// Runs `stc test`.
pub(crate) fn run(
    manifest: Option<&str>,
    locked: bool,
    request: &Request,
    options: &TestOptions,
    filters: &[String],
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph =
        match stc_manifest::load_project(manifest_input, &stc_manifest::LoadOptions { locked }) {
            Ok(graph) => graph,
            Err(error) => {
                reporter.diagnostics(&error.diagnostics, &error.sources);
                return 2;
            }
        };
    let root = graph.root_package();
    let root_index = graph.root.0;
    reporter.merge_manifest_lints(&root.manifest.lints);
    reporter.diagnostics(&graph.warnings, &graph.sources);
    log.info(format!("manifest {manifest_input}"));
    log.info(format!("profile {}", stc_manifest::PROFILE_ID));

    let relaxed = request.relaxed || root.manifest.build.syntax == SyntaxMode::Relaxed;
    let mut sources = SourceMap::new();
    let mut parse_diagnostics = DiagnosticSink::new();
    let mut entries: Vec<SourceEntry> = Vec::new();
    let mut read_error = false;
    let mut lex_time = Duration::ZERO;
    let mut parse_time = Duration::ZERO;
    let mut line_count = 0_usize;

    for (index, package) in graph.packages.iter().enumerate() {
        let mut package_sources: Vec<(&stc_manifest::SourcePath, bool)> = package
            .sources
            .iter()
            .map(|source| (source, false))
            .collect();
        if package.is_root {
            package_sources.extend(package.test_sources.iter().map(|source| (source, true)));
        }
        for (source, is_test) in package_sources {
            let display = format!("{}/{}", package.name, source.relative);
            let Some(file) = u32::try_from(entries.len()).ok().map(FileId::new) else {
                reporter.plain(
                    "STC-E-BUILD-002",
                    Severity::Error,
                    "too many source files for the span table",
                );
                return 1;
            };
            let bytes = match std::fs::read(&source.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let mapped = sources.add_file(display, String::new());
                    debug_assert_eq!(mapped, file);
                    parse_diagnostics.error(
                        "STC-E-BUILD-002",
                        format!("cannot read `{}`: {error}", source.path.to_string_lossy()),
                        Span::new(mapped, 0, 0),
                    );
                    read_error = true;
                    continue;
                }
            };
            line_count += bytes.iter().filter(|byte| **byte == b'\n').count();
            let lex_start = Instant::now();
            let lexed = tokenize_bytes(file, &bytes, &mut parse_diagnostics);
            lex_time += lex_start.elapsed();
            let mapped = sources.add_file(display.clone(), lexed.text.clone());
            debug_assert_eq!(mapped, file);
            let parse_start = Instant::now();
            let parsed = parse(
                file,
                lexed.text.as_str(),
                &lexed.stream,
                ParseOptions {
                    relaxed_semicolons: relaxed,
                    ..ParseOptions::default()
                },
                &mut parse_diagnostics,
            );
            parse_time += parse_start.elapsed();
            let report_path = if package.is_root {
                source.relative.clone()
            } else {
                display.clone()
            };
            entries.push(SourceEntry {
                package: index,
                is_test,
                display,
                relative: source.relative.clone(),
                report_path,
                file_id: file,
                bytes,
                parsed,
            });
        }
    }
    log.info(format!(
        "sources {} files, {line_count} lines",
        entries.len()
    ));
    log.info(format!(
        "lex {}  parse {}",
        duration(lex_time),
        duration(parse_time)
    ));
    if parse_diagnostics.has_errors() || read_error {
        report_sink(&parse_diagnostics, &sources, reporter, log);
        return 1;
    }

    // Discovery sees the current package's production and test sources.
    // Manifest `[[test.case]]` spans refer to the manifest source map; merge
    // those files into the compilation source map so one reporter can render
    // both kinds of diagnostic.
    let mut manifest_file_ids = BTreeMap::new();
    for index in 0..graph.sources.len() {
        let id = FileId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let Some(file) = graph.sources.file(id) else {
            continue;
        };
        let mapped = sources.add_file(file.path().to_owned(), file.text().to_owned());
        manifest_file_ids.insert(index, mapped);
    }
    let mut test_settings = root.manifest.test.clone();
    for case in &mut test_settings.cases {
        if let Some(mapped) = manifest_file_ids.get(&case.span.file.index()) {
            case.span = Span::new(*mapped, case.span.start, case.span.end);
        }
    }
    let discovery_refs: Vec<SourceFileRef<'_>> = entries
        .iter()
        .filter(|entry| entry.package == root_index)
        .map(|entry| SourceFileRef {
            relative: &entry.report_path,
            parsed: &entry.parsed,
        })
        .collect();
    let discovery = discover(&discovery_refs, &test_settings, options.timeout_ns);
    if !discovery.diagnostics.is_empty() {
        reporter.diagnostics(&discovery.diagnostics, &sources);
        return 1;
    }
    let tests = filter_tests(&discovery.tests, filters);
    if options.list {
        for test in &tests {
            if reporter.is_json() {
                println!("{{\"name\":{}}}", json_string(&test.name));
            } else {
                println!("{}", test.name);
            }
        }
        return u8::from(reporter.denied());
    }
    if tests.is_empty() {
        log.status("0 tests discovered");
        return u8::from(reporter.denied());
    }

    // Build the per-package file vectors once; each test appends its wrapper.
    let mut package_files: Vec<Vec<ParsedFile>> = vec![Vec::new(); graph.packages.len()];
    for entry in &entries {
        package_files[entry.package].push(entry.parsed.clone());
    }
    let test_files: Vec<ParsedFile> = entries
        .iter()
        .filter(|entry| entry.is_test)
        .map(|entry| entry.parsed.clone())
        .collect();

    // Catch semantic errors once before building N artifacts.
    {
        let mut diagnostics = DiagnosticSink::new();
        let session = session_for(
            &graph,
            &package_files,
            &test_files,
            None,
            root.manifest.project.stdlib,
        );
        let _analysis = stc_sema::analyze_project(&session, &mut diagnostics);
        // The per-test analyses repeat these warnings; report them once.
        reporter.diagnostics(diagnostics.diagnostics(), &sources);
        if diagnostics.has_errors() {
            return 1;
        }
    }

    let artifact_root = request
        .out_dir
        .as_ref()
        .map_or_else(|| root.root_dir.join("target/stc/test"), PathBuf::from);
    let effective_io = request
        .run
        .io
        .clone()
        .unwrap_or_else(|| root.manifest.test.effective_io().as_str().to_owned());
    let io_file = request
        .run
        .io_file
        .as_ref()
        .map(|file| absolute(Path::new(file)))
        .or_else(|| {
            root.manifest
                .io
                .file
                .as_ref()
                .map(|file| root.root_dir.join(file))
        });
    let mut built = Vec::new();
    let mut build_exit = 0_u8;
    for test in &tests {
        let artifact = artifact_name(&root.name, &test.name);
        let wrapper = wrapper_source(test);
        let wrapper_file = sources.add_virtual_file(format!("stc-test-{artifact}"), &wrapper);
        let mut wrapper_diagnostics = DiagnosticSink::new();
        let wrapper_lexed =
            tokenize_bytes(wrapper_file, wrapper.as_bytes(), &mut wrapper_diagnostics);
        let wrapper_parsed = parse(
            wrapper_file,
            wrapper_lexed.text.as_str(),
            &wrapper_lexed.stream,
            ParseOptions {
                relaxed_semicolons: relaxed,
                ..ParseOptions::default()
            },
            &mut wrapper_diagnostics,
        );
        if wrapper_diagnostics.has_errors() {
            reporter.diagnostics(wrapper_diagnostics.diagnostics(), &sources);
            build_exit = build_exit.max(1);
            continue;
        }
        let mut files_for_test = package_files.clone();
        files_for_test[root_index].push(wrapper_parsed);
        let mut diagnostics = DiagnosticSink::new();
        let session = session_for(
            &graph,
            &files_for_test,
            &test_files,
            Some(stc_test::discovery::TEST_CONFIGURATION),
            root.manifest.project.stdlib,
        );
        let analysis = stc_sema::analyze_project(&session, &mut diagnostics);
        if diagnostics.has_errors() {
            reporter.diagnostics(diagnostics.diagnostics(), &sources);
            build_exit = build_exit.max(1);
            continue;
        }

        let mut test_request = request.clone();
        test_request.emit = [Emit::Module, Emit::Metadata, Emit::Exe]
            .into_iter()
            .collect();
        test_request.test_build = true;
        // `stc test` always builds the hook-free debug form, regardless of
        // `[build].debug`, `[debug].default`, or `--debug`.
        test_request.apply_mode(crate::build::BuildMode::Debug);
        test_request.out_dir_explicit = false;
        test_request.cold = true;
        test_request.got_manifest = None;
        test_request.renames.clear();
        test_request.allow_untracked_removal = false;
        test_request.relaxed = relaxed;
        test_request.opt_level = root.manifest.build.opt_level;
        test_request.opt_level_explicit = root.manifest.build.opt_level_explicit;
        test_request.lib_type = root.manifest.build.lib_type;
        test_request.manifest_target = root.manifest.build.target.clone();
        test_request.manifest_clang = root.manifest.build.clang.clone();
        test_request.manifest_runtime = root.manifest.build.runtime.clone();
        test_request.project = build::ProjectSettings::from_manifest(&root.manifest);
        test_request.project.retain_enabled = false;
        test_request.project.io_backend.clone_from(&effective_io);
        let out_dir = artifact_root.join(&artifact);
        // A previous run can leave a module with an older content-derived
        // build ID; start each test artifact directory clean.
        if let Err(error) = std::fs::remove_dir_all(&out_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot remove `{}`: {error}", out_dir.display()),
            );
            build_exit = build_exit.max(1);
            continue;
        }
        test_request.out_dir = Some(out_dir.to_string_lossy().into_owned());

        // Source paths must cover every FileId up to the wrapper so metadata's
        // span-file index stays valid; earlier wrappers are unused placeholders.
        let mut source_files: Vec<String> =
            entries.iter().map(|entry| entry.display.clone()).collect();
        let mut source_paths: Vec<String> =
            entries.iter().map(|entry| entry.relative.clone()).collect();
        while source_files.len() <= wrapper_file.index() {
            let id = FileId::new(u32::try_from(source_files.len()).unwrap_or(u32::MAX));
            source_files.push(
                sources
                    .file_path(id)
                    .map_or_else(|| "<test>".into(), str::to_owned),
            );
            source_paths.push(format!("{artifact}.st"));
        }
        source_paths[wrapper_file.index()] = format!("{artifact}.st");
        let mut fingerprint_sources: Vec<(String, String, Vec<u8>)> = entries
            .iter()
            .map(|entry| {
                (
                    graph.packages[entry.package].name.clone(),
                    entry.relative.clone(),
                    entry.bytes.clone(),
                )
            })
            .collect();
        fingerprint_sources.push((
            root.name.clone(),
            format!("{artifact}.st"),
            wrapper.as_bytes().to_vec(),
        ));
        let fingerprint =
            match stc_codegen::module::project_source_fingerprint(&fingerprint_sources) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    reporter.plain(error.code, Severity::Error, &error.message);
                    build_exit = build_exit.max(1);
                    continue;
                }
            };

        let generated = match build::generate(
            &test_request,
            &analysis.hir,
            &artifact,
            &source_files,
            Some(&source_paths),
            &sources,
            fingerprint,
            &mut diagnostics,
            reporter,
            log,
        ) {
            Ok(generated) => generated,
            Err(_) => {
                reporter.diagnostics(diagnostics.diagnostics(), &sources);
                build_exit = build_exit.max(1);
                continue;
            }
        };
        let code = build::emit(&test_request, &artifact, &generated, reporter, log);
        report_sink(&diagnostics, &sources, reporter, log);
        if code != 0 {
            build_exit = build_exit.max(code);
            continue;
        }
        let passed_root = if test.has_passed {
            match passed_root(&generated) {
                Some(root) => Some(root),
                None => {
                    reporter.plain(
                        "STC-E-CODEGEN-002",
                        Severity::Error,
                        "the test wrapper's `PASSED` output has no GOT root",
                    );
                    build_exit = build_exit.max(1);
                    continue;
                }
            }
        } else {
            None
        };
        let assertion_path = generated.out_dir.join(format!("{artifact}.assertions"));
        if let Err(error) = write_assertion_sites(
            &assertion_path,
            &analysis.hir,
            &generated,
            &sources,
            wrapper_file,
            test,
            &entries,
        ) {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot write `{}`: {error}", assertion_path.display()),
            );
            build_exit = build_exit.max(1);
            continue;
        }
        let exe = build::launcher_path(&generated.out_dir, &artifact);
        let module = generated.out_dir.join(&generated.module_file);
        let declaration = sources.location(test.span);
        let mut args = vec![
            "--name".to_owned(),
            test.name.clone(),
            "--cycles".to_owned(),
            test.cycles.to_string(),
            "--mock-clock".to_owned(),
            format!("{}ns", root.manifest.test.mock_clock.as_nanos()),
            "--io".to_owned(),
            effective_io.clone(),
            "--assertions".to_owned(),
            assertion_path.to_string_lossy().into_owned(),
            "--decl-file".to_owned(),
            test.file.clone(),
            "--decl-line".to_owned(),
            declaration.line.to_string(),
            "--decl-column".to_owned(),
            declaration.column.to_string(),
            "--module".to_owned(),
            module.to_string_lossy().into_owned(),
        ];
        if let Some(file) = &io_file {
            args.push("--io-file".into());
            args.push(file.to_string_lossy().into_owned());
        }
        if let Some(trap) = &test.expect_trap {
            args.push("--expect-trap".into());
            args.push(trap.clone());
        }
        if let Some(root) = passed_root {
            args.push("--passed-root".into());
            args.push(root);
        }
        built.push(BuiltTest {
            test: test.clone(),
            exe,
            args,
        });
    }
    if build_exit != 0 {
        return build_exit;
    }
    if options.no_run {
        log.status(format!("built {} test artifact(s)", built.len()));
        return u8::from(reporter.denied());
    }
    let code = run_tests(&built, options, reporter);
    if code == 0 && reporter.denied() {
        1
    } else {
        code
    }
}

/// Runs the built test processes sequentially.
fn run_tests(built: &[BuiltTest], options: &TestOptions, reporter: &Reporter) -> u8 {
    let mut passed = 0;
    let mut failed = 0;
    let mut timed_out = 0;
    let mut trapped = 0;
    for item in built {
        let timeout = Duration::from_nanos(item.test.timeout_ns.max(1));
        let outcome = stc_test::run_process(&item.exe, &item.args, timeout);
        if options.nocapture {
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
        }
        match outcome.status {
            TestStatus::Pass => passed += 1,
            TestStatus::Fail => failed += 1,
            TestStatus::Timeout => timed_out += 1,
            TestStatus::Trap => trapped += 1,
        }
        if reporter.is_json() {
            println!("{}", json_result(&item.test.name, &outcome));
        } else if !reporter.is_quiet() || outcome.status != TestStatus::Pass {
            println!("{}", human_result_line(&item.test.name, &outcome));
            for line in human_failure_lines(&outcome) {
                println!("{line}");
            }
        }
    }
    if !reporter.is_json() && !reporter.is_quiet() {
        println!("{}", summary_line(passed, failed, timed_out, trapped));
    }
    if trapped > 0 {
        4
    } else if failed > 0 || timed_out > 0 {
        1
    } else {
        0
    }
}

/// Builds the merged session for one analysis pass.
fn session_for<'a>(
    graph: &'a stc_manifest::ProjectGraph,
    package_files: &'a [Vec<ParsedFile>],
    test_files: &'a [ParsedFile],
    configuration: Option<&str>,
    stdlib: bool,
) -> ProjectSession<'a> {
    let packages: Vec<PackageSession<'a>> = graph
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| PackageSession {
            name: package.name.clone(),
            root_namespace: package.namespace.clone(),
            dependencies: package
                .dependencies
                .iter()
                .map(|dependency| dependency.package.0)
                .collect(),
            files: &package_files[index],
            is_root: package.is_root,
        })
        .collect();
    ProjectSession {
        packages,
        configuration: configuration.map(str::to_owned),
        stdlib,
        test_build: true,
        test_files,
    }
}

/// Finds the GOT identity of the wrapper FB instance's `PASSED` output.
///
/// FB fields are `PROTECTED` by default, so the wrapper cannot read `PASSED`
/// in source; the host reads the leaf root directly instead.
fn passed_root(generated: &build::Generated) -> Option<String> {
    let manifest = &generated.output.got.manifest;
    let expected = format!("{}.PASSED", stc_test::discovery::TEST_FB);
    manifest
        .roots
        .iter()
        .find(|root| root.field_path.eq_ignore_ascii_case(&expected))
        .map(|root| root.got_id.to_hex())
}

/// Writes the assertion-site sidecar the test host uses to name failures.
fn write_assertion_sites(
    path: &Path,
    hir: &HirCrate,
    generated: &build::Generated,
    sources: &SourceMap,
    wrapper_file: FileId,
    test: &TestCase,
    entries: &[SourceEntry],
) -> Result<(), std::io::Error> {
    let report_paths: BTreeMap<FileId, &str> = entries
        .iter()
        .map(|entry| (entry.file_id, entry.report_path.as_str()))
        .collect();
    let mut text = String::new();
    for expr in &hir.exprs {
        let ExprKindHir::Call {
            callee: CalleeHir::StdFunction(id),
            args,
        } = &expr.kind
        else {
            continue;
        };
        if hir.std_fns[id.index()].kind != StdFnKind::Assert {
            continue;
        }
        let Some(span_id) = generated
            .output
            .spans
            .iter()
            .find(|entry| {
                entry.file == expr.span.file
                    && entry.start == expr.span.start
                    && entry.end == expr.span.end
            })
            .map(|entry| entry.id)
        else {
            continue;
        };
        let message = assertion_message(hir, args, sources, expr.span);
        let (file, line, column) = if expr.span.file == wrapper_file {
            let location = sources.location(test.span);
            (test.file.clone(), location.line, location.column)
        } else {
            let location = sources.location(expr.span);
            let file = report_paths
                .get(&expr.span.file)
                .map_or_else(|| String::from("<unknown>"), |path| (*path).to_owned());
            (file, location.line, location.column)
        };
        text.push_str(&format!(
            "{span_id}\t{line}\t{column}\t{}\t{}\n",
            escape_field(&file),
            escape_field(&message)
        ));
    }
    std::fs::write(path, text)
}

fn assertion_message(
    hir: &HirCrate,
    args: &[stc_hir::ArgHir],
    sources: &SourceMap,
    call_span: Span,
) -> String {
    let Some(value) = args.get(1).and_then(|arg| arg.value) else {
        return "assertion failed".into();
    };
    if let ExprKindHir::Literal(literal) = &hir.expr(value).kind
        && let ConstValue::String(units) = &literal.value
    {
        return units
            .iter()
            .filter_map(|unit| char::from_u32(*unit))
            .collect();
    }
    let text = sources.text(hir.expr(value).span);
    let text = if text.is_empty() {
        sources.text(call_span)
    } else {
        text
    };
    if text.is_empty() {
        "assertion failed".into()
    } else {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn escape_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            character => out.push(character),
        }
    }
    out
}

fn artifact_name(package: &str, test: &str) -> String {
    let mut out = String::new();
    for character in format!("{package}-{test}").chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            out.push(character);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "test".into() } else { out }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
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
    out
}

fn duration(value: Duration) -> String {
    format!("{:.1}ms", value.as_secs_f64() * 1000.0)
}

fn report_sink(sink: &DiagnosticSink, sources: &SourceMap, reporter: &mut Reporter, log: Log) {
    reporter.diagnostics(sink.diagnostics(), sources);
    if sink.suppressed() > 0 {
        log.status(format!(
            "note: {} further diagnostic(s) suppressed",
            sink.suppressed()
        ));
    }
}
