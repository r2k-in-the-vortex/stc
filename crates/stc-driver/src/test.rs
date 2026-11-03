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

use std::collections::{BTreeMap, BTreeSet};
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
    CoverageData, CoverageKind, CoveragePoint, CoverageRun, CoverageTest, SourceFileRef, TestCase,
    TestStatus, aggregate, discover, filter_tests, human_failure_lines, human_result_line,
    json_result, render_human, render_json, render_lcov, summary_line, wrapper_source,
};

use crate::build::{self, Emit, Request};
use crate::cli::TestOptions;
use crate::report::{DiagnosticPaths, Log, Reporter, absolute_path};

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

/// One built test artifact's coverage instrumentation.
struct CoverageArtifact {
    /// Raw sidecar the test host writes after the run.
    raw_path: PathBuf,
    /// Local site-ID to canonical point mapping; `None` marks an omitted site.
    sites: Vec<Option<CoveragePoint>>,
}

/// A built test artifact ready to run.
struct BuiltTest {
    test: TestCase,
    exe: PathBuf,
    args: Vec<String>,
    /// Directory holding the launcher, module, and sidecars.
    out_dir: PathBuf,
    /// Coverage instrumentation, when the run has coverage enabled.
    coverage: Option<CoverageArtifact>,
}

/// Parsed and discovered project state shared by `stc test` and the per-test
/// debug build (`stc dap` launch with `test`, WL-1.1 F-17).
struct TestProject {
    graph: stc_manifest::ProjectGraph,
    entries: Vec<SourceEntry>,
    sources: SourceMap,
    package_files: Vec<Vec<ParsedFile>>,
    test_files: Vec<ParsedFile>,
    tests: Vec<TestCase>,
    /// Production source files eligible for statement coverage.
    coverage_files: BTreeSet<FileId>,
    relaxed: bool,
    root_index: usize,
    root_name: String,
    root_dir: PathBuf,
    manifest: stc_manifest::Manifest,
    stdlib: bool,
    mock_clock_ns: u64,
}

/// Build-wide settings resolved after the manifest and profile are known.
struct TestBuildConfig {
    project_settings: build::ProjectSettings,
    effective_io: String,
    io_file: Option<PathBuf>,
}

/// One selected test built as a debug harness: checkpoint hooks, debug
/// information, a frame sidecar, and test-host arguments.
pub(crate) struct DebugTestBuild {
    /// Built test-host launcher.
    pub(crate) launcher: PathBuf,
    /// Artifact directory.
    pub(crate) out_dir: PathBuf,
    /// Test-host arguments, without debug-server arguments.
    pub(crate) args: Vec<String>,
    /// Root-package manifest.
    pub(crate) manifest: stc_manifest::Manifest,
}

impl TestProject {
    /// Loads, parses, and discovers tests without analyzing or configuring the
    /// profile; `--list` and an empty selection return before those phases.
    #[allow(clippy::too_many_arguments)]
    fn load(
        manifest: Option<&str>,
        locked: bool,
        timeout_ns: Option<u64>,
        diagnostic_paths: DiagnosticPaths,
        request: &Request,
        reporter: &mut Reporter,
        log: Log,
    ) -> Result<Self, u8> {
        let manifest_input = manifest.unwrap_or("stc.toml");
        let graph =
            match stc_manifest::load_project(manifest_input, &stc_manifest::LoadOptions { locked })
            {
                Ok(graph) => graph,
                Err(error) => {
                    reporter.diagnostics(&error.diagnostics, &error.sources);
                    return Err(2);
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
                let diagnostic_display = match diagnostic_paths {
                    DiagnosticPaths::Package => display.clone(),
                    DiagnosticPaths::Absolute => absolute_path(&source.path),
                };
                let Some(file) = u32::try_from(entries.len()).ok().map(FileId::new) else {
                    reporter.plain(
                        "STC-E-BUILD-002",
                        Severity::Error,
                        "too many source files for the span table",
                    );
                    return Err(1);
                };
                let bytes = match std::fs::read(&source.path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let mapped = sources.add_file(diagnostic_display, String::new());
                        debug_assert_eq!(mapped, file);
                        parse_diagnostics.error(
                            "STC-E-BUILD-002",
                            format!("cannot read `{}`: {}", source.path.to_string_lossy(), error),
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
                let mapped = sources.add_file(diagnostic_display.clone(), lexed.text.clone());
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
            return Err(1);
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
        let discovery = discover(&discovery_refs, &test_settings, timeout_ns);
        if !discovery.diagnostics.is_empty() {
            reporter.diagnostics(&discovery.diagnostics, &sources);
            return Err(1);
        }

        // Build the per-package file vectors once; each test appends its
        // wrapper.
        let mut package_files: Vec<Vec<ParsedFile>> = vec![Vec::new(); graph.packages.len()];
        for entry in &entries {
            package_files[entry.package].push(entry.parsed.clone());
        }
        let test_files: Vec<ParsedFile> = entries
            .iter()
            .filter(|entry| entry.is_test)
            .map(|entry| entry.parsed.clone())
            .collect();
        // Production sources of every package are instrumented; `[test].sources`
        // and the synthetic wrapper are excluded by construction.
        let coverage_files: BTreeSet<FileId> = entries
            .iter()
            .filter(|entry| !entry.is_test)
            .map(|entry| entry.file_id)
            .collect();
        let root_name = root.name.clone();
        let root_dir = root.root_dir.clone();
        let manifest = root.manifest.clone();
        let stdlib = root.manifest.project.stdlib;
        let mock_clock_ns = root.manifest.test.mock_clock.as_nanos();
        Ok(Self {
            graph,
            entries,
            sources,
            package_files,
            test_files,
            tests: discovery.tests,
            coverage_files,
            relaxed,
            root_index,
            root_name,
            root_dir,
            manifest,
            stdlib,
            mock_clock_ns,
        })
    }

    /// Reports semantic errors once before building any artifact. Warnings are
    /// reported here and not repeated by the per-test analyses.
    fn check_semantics(&self, reporter: &mut Reporter) -> u8 {
        let mut diagnostics = DiagnosticSink::new();
        let session = session_for(
            &self.graph,
            &self.package_files,
            &self.test_files,
            None,
            self.stdlib,
        );
        let _analysis = stc_sema::analyze_project(&session, &mut diagnostics);
        reporter.diagnostics(diagnostics.diagnostics(), &self.sources);
        u8::from(diagnostics.has_errors())
    }

    /// Embeds the machine profile and resolves I/O settings for test runs and
    /// debug test builds.
    fn configure(
        &mut self,
        request: &Request,
        reporter: &mut Reporter,
        log: Log,
    ) -> Result<TestBuildConfig, u8> {
        let mut profile_diagnostics = DiagnosticSink::new();
        let mut project_settings = build::ProjectSettings::from_manifest(&self.manifest);
        if !project_settings.embed_profile(
            &self.root_dir,
            &self.manifest,
            &mut self.sources,
            &mut profile_diagnostics,
        ) {
            report_sink(&profile_diagnostics, &self.sources, reporter, log);
            return Err(crate::project::diagnostic_exit(&profile_diagnostics).max(1));
        }
        let effective_io = request
            .run
            .io
            .clone()
            .unwrap_or_else(|| self.manifest.test.effective_io().as_str().to_owned());
        let io_file = request
            .run
            .io_file
            .as_ref()
            .map(|file| absolute(Path::new(file)))
            .or_else(|| {
                self.manifest
                    .io
                    .file
                    .as_ref()
                    .map(|file| self.root_dir.join(file))
            });
        Ok(TestBuildConfig {
            project_settings,
            effective_io,
            io_file,
        })
    }

    /// Builds one test artifact under `artifact_root`.
    ///
    /// `test_debug` keeps checkpoint hooks, debug information, and the frame
    /// sidecar; ordinary test builds leave it false and stay hook-free.
    /// `coverage_files` selects statement coverage for this artifact.
    #[allow(clippy::too_many_arguments)]
    fn build(
        &mut self,
        config: &TestBuildConfig,
        test: &TestCase,
        request: &Request,
        artifact_root: &Path,
        coverage_files: Option<&BTreeSet<FileId>>,
        test_debug: bool,
        reporter: &mut Reporter,
        log: Log,
    ) -> Result<BuiltTest, u8> {
        let artifact = artifact_name(&self.root_name, &test.name);
        let wrapper = wrapper_source(test);
        let wrapper_file = self
            .sources
            .add_virtual_file(format!("stc-test-{artifact}"), &wrapper);
        let mut wrapper_diagnostics = DiagnosticSink::new();
        let wrapper_lexed =
            tokenize_bytes(wrapper_file, wrapper.as_bytes(), &mut wrapper_diagnostics);
        let wrapper_parsed = parse(
            wrapper_file,
            wrapper_lexed.text.as_str(),
            &wrapper_lexed.stream,
            ParseOptions {
                relaxed_semicolons: self.relaxed,
                ..ParseOptions::default()
            },
            &mut wrapper_diagnostics,
        );
        if wrapper_diagnostics.has_errors() {
            reporter.diagnostics(wrapper_diagnostics.diagnostics(), &self.sources);
            return Err(1);
        }
        let mut files_for_test = self.package_files.clone();
        files_for_test[self.root_index].push(wrapper_parsed);
        let mut diagnostics = DiagnosticSink::new();
        let session = session_for(
            &self.graph,
            &files_for_test,
            &self.test_files,
            Some(stc_test::discovery::TEST_CONFIGURATION),
            self.stdlib,
        );
        let analysis = stc_sema::analyze_project(&session, &mut diagnostics);
        if diagnostics.has_errors() {
            reporter.diagnostics(diagnostics.diagnostics(), &self.sources);
            return Err(1);
        }

        let mut test_request = request.clone();
        test_request.emit = [Emit::Module, Emit::Metadata, Emit::Exe]
            .into_iter()
            .collect();
        test_request.test_build = true;
        test_request.test_debug = test_debug;
        // `stc test` always builds the debug form, regardless of
        // `[build].debug`, `[debug].default`, or `--debug`. A debug harness
        // additionally keeps checkpoint hooks.
        test_request.apply_mode(crate::build::BuildMode::Debug);
        test_request.out_dir_explicit = false;
        test_request.cold = true;
        test_request.got_manifest = None;
        test_request.renames.clear();
        test_request.allow_untracked_removal = false;
        test_request.relaxed = self.relaxed;
        test_request.opt_level = self.manifest.build.opt_level;
        test_request.opt_level_explicit = self.manifest.build.opt_level_explicit;
        test_request.lib_type = self.manifest.build.lib_type;
        test_request.manifest_target = self.manifest.build.target.clone();
        test_request.manifest_clang = self.manifest.build.clang.clone();
        test_request.manifest_runtime = self.manifest.build.runtime.clone();
        test_request.project = config.project_settings.clone();
        test_request.project.retain_enabled = false;
        test_request
            .project
            .io_backend
            .clone_from(&config.effective_io);
        // Non-coverage test builds stay byte-identical: the request carries no
        // coverage options and codegen receives `None`.
        test_request.coverage = coverage_files.map(|files| stc_codegen::CoverageOptions {
            files: files.clone(),
        });
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
            return Err(1);
        }
        test_request.out_dir = Some(out_dir.to_string_lossy().into_owned());

        // Source paths must cover every FileId up to the wrapper so metadata's
        // span-file index stays valid; earlier wrappers are unused placeholders.
        let mut source_files: Vec<String> = self
            .entries
            .iter()
            .map(|entry| entry.display.clone())
            .collect();
        let mut source_paths: Vec<String> = self
            .entries
            .iter()
            .map(|entry| entry.relative.clone())
            .collect();
        while source_files.len() <= wrapper_file.index() {
            let id = FileId::new(u32::try_from(source_files.len()).unwrap_or(u32::MAX));
            source_files.push(
                self.sources
                    .file_path(id)
                    .map_or_else(|| "<test>".into(), str::to_owned),
            );
            source_paths.push(format!("{artifact}.st"));
        }
        source_paths[wrapper_file.index()] = format!("{artifact}.st");
        let mut fingerprint_sources: Vec<(String, String, Vec<u8>)> = self
            .entries
            .iter()
            .map(|entry| {
                (
                    self.graph.packages[entry.package].name.clone(),
                    entry.relative.clone(),
                    entry.bytes.clone(),
                )
            })
            .collect();
        fingerprint_sources.push((
            self.root_name.clone(),
            format!("{artifact}.st"),
            wrapper.as_bytes().to_vec(),
        ));
        let fingerprint =
            match stc_codegen::module::project_source_fingerprint(&fingerprint_sources) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    reporter.plain(error.code, Severity::Error, &error.message);
                    return Err(1);
                }
            };

        let generated = match build::generate(
            &test_request,
            &analysis.hir,
            &artifact,
            &source_files,
            Some(&source_paths),
            &self.sources,
            fingerprint,
            &mut diagnostics,
            reporter,
            log,
        ) {
            Ok(generated) => generated,
            Err(_) => {
                reporter.diagnostics(diagnostics.diagnostics(), &self.sources);
                return Err(1);
            }
        };
        let code = build::emit(&test_request, &artifact, &generated, reporter, log);
        report_sink(&diagnostics, &self.sources, reporter, log);
        if code != 0 {
            return Err(code);
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
                    return Err(1);
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
            &self.sources,
            wrapper_file,
            test,
            &self.entries,
        ) {
            reporter.plain(
                "STC-E-BUILD-002",
                Severity::Error,
                &format!("cannot write `{}`: {error}", assertion_path.display()),
            );
            return Err(1);
        }
        let coverage = if coverage_files.is_some() {
            let report_paths: BTreeMap<FileId, &str> = self
                .entries
                .iter()
                .map(|entry| (entry.file_id, entry.report_path.as_str()))
                .collect();
            let (points, omitted) = coverage_points(
                &generated.output.coverage_sites,
                &report_paths,
                &self.sources,
            );
            if omitted > 0 {
                reporter.plain(
                    "STC-W-COVERAGE-002",
                    Severity::Warning,
                    &format!(
                        "{omitted} coverage site(s) in test `{}` cannot be attributed to a \
                         reportable source file; omitted",
                        test.name
                    ),
                );
            }
            Some(CoverageArtifact {
                raw_path: generated.out_dir.join(format!("{artifact}.coverage")),
                sites: points,
            })
        } else {
            None
        };
        let exe = build::launcher_path(&generated.out_dir, &artifact);
        let module = generated.out_dir.join(&generated.module_file);
        let declaration = self.sources.location(test.span);
        let mut args = vec![
            "--name".to_owned(),
            test.name.clone(),
            "--cycles".to_owned(),
            test.cycles.to_string(),
            "--mock-clock".to_owned(),
            format!("{}ns", self.mock_clock_ns),
            "--io".to_owned(),
            config.effective_io.clone(),
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
        if let Some(file) = &config.io_file {
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
        if let Some(coverage) = &coverage {
            args.push("--coverage".into());
            args.push(coverage.raw_path.to_string_lossy().into_owned());
            args.push("--coverage-sites".into());
            args.push(coverage.sites.len().to_string());
        }
        Ok(BuiltTest {
            test: test.clone(),
            exe,
            args,
            out_dir: generated.out_dir.clone(),
            coverage,
        })
    }
}

/// Builds one selected test as a debug harness for `stc dap`.
///
/// The artifact lands under `<artifact root>/debug/<test>` so it cannot
/// overwrite the hook-free `stc test` artifact. `request.outDir` replaces the
/// default `target/stc/test` artifact root and still gains the `debug`
/// component.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_debug_test(
    manifest: Option<&str>,
    request: &Request,
    test_name: &str,
    diagnostic_paths: DiagnosticPaths,
    reporter: &mut Reporter,
    log: Log,
) -> Result<DebugTestBuild, String> {
    let mut project = TestProject::load(
        manifest,
        false,
        None,
        diagnostic_paths,
        request,
        reporter,
        log,
    )
    .map_err(|code| format!("the test project could not be loaded (exit code {code})"))?;
    let Some(test) = project
        .tests
        .iter()
        .find(|test| test.name == test_name)
        .cloned()
    else {
        return Err(format!(
            "no test named `{test_name}` was discovered; run `stc test --list` to list tests"
        ));
    };
    let code = project.check_semantics(reporter);
    if code != 0 {
        return Err(format!("the test build failed (exit code {code})"));
    }
    let config = project
        .configure(request, reporter, log)
        .map_err(|code| format!("the test build failed (exit code {code})"))?;
    let base = request
        .out_dir
        .as_ref()
        .map_or_else(|| project.root_dir.join("target/stc/test"), PathBuf::from);
    let artifact_root = absolute(&base.join("debug"));
    let built = project
        .build(
            &config,
            &test,
            request,
            &artifact_root,
            None,
            true,
            reporter,
            log,
        )
        .map_err(|code| format!("the test build failed (exit code {code})"))?;
    Ok(DebugTestBuild {
        launcher: built.exe,
        out_dir: built.out_dir,
        args: built.args,
        manifest: project.manifest.clone(),
    })
}

/// Runs `stc test`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    manifest: Option<&str>,
    locked: bool,
    diagnostic_paths: DiagnosticPaths,
    request: &Request,
    options: &TestOptions,
    filters: &[String],
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let mut project = match TestProject::load(
        manifest,
        locked,
        options.timeout_ns,
        diagnostic_paths,
        request,
        reporter,
        log,
    ) {
        Ok(project) => project,
        Err(code) => return code,
    };

    // `--coverage`/`--no-coverage` override `[test] coverage`; `--list` ignores
    // the manifest default and returns before any coverage work.
    let effective_coverage = options.coverage.unwrap_or(project.manifest.test.coverage);
    if options.coverage_out.is_some() && !effective_coverage {
        reporter.plain(
            "STC-E-PROJECT-010",
            Severity::Error,
            "`--coverage-out` requires coverage to be enabled (`--coverage` or `[test] coverage = \
             true`)",
        );
        return 2;
    }
    let tests = filter_tests(&project.tests, filters, options.exact);
    if options.list {
        // Root-package tests report plain package-relative paths; a dependency
        // that ever contributes a test would report `<package>/<relative>`.
        // Resolve the owning package from the parsed source entries.
        let mut packages: BTreeMap<&str, &str> = BTreeMap::new();
        for entry in &project.entries {
            packages
                .entry(entry.report_path.as_str())
                .or_insert_with(|| project.graph.packages[entry.package].name.as_str());
        }
        for test in &tests {
            if reporter.is_json() {
                let location = project.sources.location(test.span);
                let package = packages
                    .get(test.file.as_str())
                    .copied()
                    .unwrap_or(project.root_name.as_str());
                println!(
                    "{{\"name\":{},\"package\":{},\"file\":{},\"line\":{},\"column\":{}}}",
                    json_string(&test.name),
                    json_string(package),
                    json_string(&test.file),
                    location.line,
                    location.column
                );
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

    // Catch semantic errors once before building N artifacts.
    if project.check_semantics(reporter) != 0 {
        return 1;
    }
    let config = match project.configure(request, reporter, log) {
        Ok(config) => config,
        Err(code) => return code,
    };
    let artifact_root = request
        .out_dir
        .as_ref()
        .map_or_else(|| project.root_dir.join("target/stc/test"), PathBuf::from);

    let mut built = Vec::new();
    let mut build_exit = 0_u8;
    let coverage_files = effective_coverage.then(|| project.coverage_files.clone());
    for test in &tests {
        match project.build(
            &config,
            test,
            request,
            &artifact_root,
            coverage_files.as_ref(),
            false,
            reporter,
            log,
        ) {
            Ok(item) => built.push(item),
            Err(code) => build_exit = build_exit.max(code),
        }
    }
    if build_exit != 0 {
        return build_exit;
    }
    if options.no_run {
        if effective_coverage {
            log.status(
                "coverage enabled: instrumented artifacts built; no coverage report is written \
                 with `--no-run`",
            );
        }
        log.status(format!("built {} test artifact(s)", built.len()));
        return u8::from(reporter.denied());
    }
    let (mut code, coverage_tests) = run_tests(&built, options, reporter);
    if effective_coverage {
        let coverage_dir = options.coverage_out.as_ref().map_or_else(
            || project.root_dir.join("target/stc/coverage"),
            PathBuf::from,
        );
        let report = aggregate(&CoverageRun {
            package: project.root_name.clone(),
            filters: filters.to_vec(),
            tests: coverage_tests,
        });
        if let Err(message) = write_coverage_reports(&coverage_dir, &report) {
            reporter.plain("STC-E-BUILD-002", Severity::Error, &message);
            code = 1;
        } else if reporter.is_json() {
            log.status(format!(
                "coverage report {}",
                crate::report::display_path(&coverage_dir.join("coverage.json"))
            ));
            log.status(format!(
                "coverage report {}",
                crate::report::display_path(&coverage_dir.join("lcov.info"))
            ));
        } else if !reporter.is_quiet() {
            print!("{}", render_human(&report));
        }
    }
    if code == 0 && reporter.denied() {
        1
    } else {
        code
    }
}

/// Writes `coverage.json` and `lcov.info`, creating the directory.
fn write_coverage_reports(dir: &Path, report: &stc_test::CoverageReport) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create `{}`: {error}", dir.display()))?;
    let json_path = dir.join("coverage.json");
    std::fs::write(&json_path, render_json(report))
        .map_err(|error| format!("cannot write `{}`: {error}", json_path.display()))?;
    let info_path = dir.join("lcov.info");
    std::fs::write(&info_path, render_lcov(report))
        .map_err(|error| format!("cannot write `{}`: {error}", info_path.display()))?;
    Ok(())
}

/// Runs the built test processes sequentially.
fn run_tests(
    built: &[BuiltTest],
    options: &TestOptions,
    reporter: &mut Reporter,
) -> (u8, Vec<CoverageTest>) {
    let mut passed = 0;
    let mut failed = 0;
    let mut timed_out = 0;
    let mut trapped = 0;
    let mut coverage_tests = Vec::new();
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
        if let Some(artifact) = &item.coverage {
            let data = if outcome.status == TestStatus::Timeout {
                // A timed-out process never reports data; its sidecar is not read.
                CoverageData::Timeout
            } else {
                match load_coverage_data(&artifact.raw_path, artifact.sites.len()) {
                    Ok(counts) => CoverageData::Reported { counts },
                    Err(message) => {
                        reporter.plain(
                            "STC-W-COVERAGE-001",
                            Severity::Warning,
                            &format!(
                                "test `{}` has no usable coverage data: {message}; excluded from \
                                 the aggregate",
                                item.test.name
                            ),
                        );
                        CoverageData::Missing
                    }
                }
            };
            coverage_tests.push(CoverageTest {
                name: item.test.name.clone(),
                status: outcome.status,
                sites: artifact.sites.clone(),
                data,
            });
        }
    }
    if !reporter.is_json() && !reporter.is_quiet() {
        println!("{}", summary_line(passed, failed, timed_out, trapped));
    }
    let code = if trapped > 0 {
        4
    } else if failed > 0 || timed_out > 0 {
        1
    } else {
        0
    };
    (code, coverage_tests)
}

/// Maps artifact-local coverage sites to canonical report points.
///
/// `report_paths` holds the reportable files (all non-test entries); a site in
/// any other file is omitted and counted. Omitted sites are reported with
/// `STC-W-COVERAGE-002`.
fn coverage_points(
    sites: &[stc_codegen::CoverageSite],
    report_paths: &BTreeMap<FileId, &str>,
    sources: &SourceMap,
) -> (Vec<Option<CoveragePoint>>, usize) {
    let mut points = Vec::with_capacity(sites.len());
    let mut omitted = 0_usize;
    for site in sites {
        let point = report_paths.get(&site.span.file).map(|path| {
            let location = sources.location(site.span);
            CoveragePoint {
                file: (*path).to_owned(),
                start: site.span.start,
                end: site.span.end,
                line: location.line,
                column: location.column,
                kind: match site.kind {
                    stc_codegen::CoverageKind::Statement => CoverageKind::Statement,
                },
            }
        });
        if point.is_none() {
            omitted += 1;
        }
        points.push(point);
    }
    (points, omitted)
}

/// Reads and validates one raw coverage sidecar against the artifact site table.
fn load_coverage_data(path: &Path, sites: usize) -> Result<Vec<(u32, u64)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let parsed = stc_test::parse_raw_coverage(&text)
        .map_err(|error| format!("malformed `{}`: {error}", path.display()))?;
    if parsed.sites as usize != sites {
        return Err(format!(
            "`{}` declares {} site(s), the artifact has {sites}",
            path.display(),
            parsed.sites
        ));
    }
    if let Some((id, _)) = parsed.counts.iter().find(|(id, _)| *id as usize >= sites) {
        return Err(format!(
            "`{}` site ID {id} is out of range for {sites} site(s)",
            path.display()
        ));
    }
    Ok(parsed.counts)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sidecar(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "stc-load-coverage-{}-{name}.coverage",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("write sidecar");
        path
    }

    #[test]
    fn coverage_sites_outside_reportable_files_are_omitted() {
        // A site without a reportable file is dropped from the report; this is
        // the `STC-W-COVERAGE-002` case.
        let mut sources = SourceMap::new();
        let reportable = sources.add_file("src/main.st", "PROGRAM Main\nEND_PROGRAM\n");
        let excluded = sources.add_file("tests/main.st", "PROGRAM Main\nEND_PROGRAM\n");
        let report_paths = BTreeMap::from([(reportable, "src/main.st")]);
        let sites = [
            stc_codegen::CoverageSite {
                id: 0,
                span: Span::new(reportable, 0, 12),
                kind: stc_codegen::CoverageKind::Statement,
            },
            stc_codegen::CoverageSite {
                id: 1,
                span: Span::new(excluded, 0, 12),
                kind: stc_codegen::CoverageKind::Statement,
            },
        ];
        let (points, omitted) = coverage_points(&sites, &report_paths, &sources);
        assert_eq!(omitted, 1);
        assert_eq!(points.len(), 2);
        let point = points[0].as_ref().expect("reportable site");
        assert_eq!(point.file, "src/main.st");
        assert_eq!((point.start, point.end), (0, 12));
        assert_eq!((point.line, point.column), (1, 1));
        assert!(points[1].is_none());
    }

    #[test]
    fn accepts_a_sidecar_matching_the_artifact_site_table() {
        let path = sidecar("valid", "stc-coverage-raw/1 2\n0 3\n1 1\n");
        let counts = load_coverage_data(&path, 2).expect("valid sidecar");
        assert_eq!(counts, vec![(0, 3), (1, 1)]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_missing_malformed_and_mismatched_sidecars() {
        let missing = std::env::temp_dir().join(format!(
            "stc-load-coverage-{}-missing.coverage",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);
        assert!(
            load_coverage_data(&missing, 1)
                .unwrap_err()
                .contains("cannot read")
        );

        let malformed = sidecar("malformed", "not a sidecar\n");
        assert!(
            load_coverage_data(&malformed, 1)
                .unwrap_err()
                .contains("malformed")
        );
        let _ = std::fs::remove_file(malformed);

        let mismatched = sidecar("mismatched", "stc-coverage-raw/1 1\n0 1\n");
        let error = load_coverage_data(&mismatched, 2).unwrap_err();
        assert!(
            error.contains("declares 1 site(s), the artifact has 2"),
            "{error}"
        );
        let _ = std::fs::remove_file(mismatched);

        let out_of_range = sidecar("range", "stc-coverage-raw/1 1\n1 1\n");
        assert!(
            load_coverage_data(&out_of_range, 1)
                .unwrap_err()
                .contains("out of range")
        );
        let _ = std::fs::remove_file(out_of_range);
    }
}
