//! `stc rename`: the CLI front end for the shared `stc-ide` rename engine
//! (CLI §3.1, D-253).
//!
//! The command keeps the project loading, parsing, and test-build surface it
//! has always used, but it no longer collects edits itself. It builds a
//! [`stc_ide::RenameContext`] from the same analysis, resolves the old dotted
//! key through the shared index, and applies the engine's returned source,
//! manifest, profile, and access edits. The engine's canonical history entries
//! are written only after every file write succeeds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stc_diag::{Diagnostic, DiagnosticSink, Severity};
use stc_hir::{DefKind, HirCrate};
use stc_ide::{
    DeclarationIndex, RenameAnalyzer, RenameContext, RenameEdit, RenameError, RenameFile,
    RenameOnline, RenameProfile, RenameResult, RenameTarget, RenameWarning,
};
use stc_lexer::tokenize_bytes;
use stc_manifest::{
    LoadOptions, Manifest, Package, Profile, ProjectGraph, RenameEntry, append_renames_file,
    load_project_profile, load_project_readonly, load_rename_history, merge_renames, parse_profile,
};
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, PackageSession, ProjectSession};
use stc_span::{FileId, SourceMap};

use crate::build::Request;
use crate::report::{Log, Reporter};

/// One parsed project source.
struct Entry {
    path: PathBuf,
    display: String,
    file: FileId,
    parsed: ParsedFile,
}

/// The selected machine profile and the source map that owns its text.
struct ProfileInput {
    path: PathBuf,
    sources: SourceMap,
    file: FileId,
    profile: Option<Profile>,
    error: Option<String>,
}

/// One validated file replacement.
struct WritePlan {
    path: PathBuf,
    display: String,
    text: String,
}

/// Runs `stc rename`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    old: &str,
    new: &str,
    dry_run: bool,
    manifest: Option<&str>,
    locked: bool,
    request: &Request,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let Some(old_segments) = parse_key(old) else {
        return usage(
            reporter,
            &format!("invalid rename source `{old}` (expected a dotted identifier path)"),
        );
    };
    let Some(new_segments) = parse_key(new) else {
        return usage(
            reporter,
            &format!("invalid rename target `{new}` (expected a dotted identifier path)"),
        );
    };
    if old_segments.len() != new_segments.len()
        || old_segments[..old_segments.len() - 1]
            .iter()
            .zip(&new_segments)
            .any(|(old, new)| !old.eq_ignore_ascii_case(new))
    {
        return usage(
            reporter,
            &format!(
                "`stc rename` changes one name in place; `{old}` -> `{new}` moves the declaration \
                 (use build `--rename {old}={new}` for a tracked move)"
            ),
        );
    }
    let old_name = old_segments.last().expect("non-empty key");
    let new_name = new_segments.last().expect("non-empty key");
    if !stc_lexer::is_identifier(new_name) {
        return usage(
            reporter,
            &format!("`{new_name}` is not a valid identifier or is a reserved keyword"),
        );
    }
    if old_name.eq_ignore_ascii_case(new_name) && old_name != new_name {
        // A spelling-only change is a rename of the same symbol.
    } else if old_name.eq_ignore_ascii_case(new_name) {
        return usage(reporter, "the old and new names are the same");
    }

    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match load_project_readonly(manifest_input, &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return 2;
        }
    };
    let root = graph.root_package();
    reporter.merge_manifest_lints(&root.manifest.lints);
    reporter.diagnostics(&graph.warnings, &graph.sources);

    let relaxed =
        request.relaxed || root.manifest.build.syntax == stc_manifest::SyntaxMode::Relaxed;
    let mut sources = SourceMap::new();
    let mut diagnostics = DiagnosticSink::new();
    let mut entries = Vec::new();
    let mut ranges = Vec::new();
    for package in &graph.packages {
        let start = entries.len();
        let mut package_sources: Vec<&stc_manifest::SourcePath> = package.sources.iter().collect();
        if package.is_root {
            package_sources.extend(package.test_sources.iter());
        }
        for source in package_sources {
            let display = format!("{}/{}", package.name, source.relative);
            let file = FileId::new(u32::try_from(entries.len()).expect("too many sources"));
            let bytes = match std::fs::read(&source.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let mapped = sources.add_file(display, String::new());
                    diagnostics.error(
                        "STC-E-BUILD-002",
                        format!("cannot read `{}`: {error}", source.path.to_string_lossy()),
                        stc_span::Span::new(mapped, 0, 0),
                    );
                    continue;
                }
            };
            let lexed = tokenize_bytes(file, &bytes, &mut diagnostics);
            let mapped = sources.add_file(display.clone(), lexed.text.clone());
            debug_assert_eq!(mapped, file);
            let parsed = parse(
                file,
                lexed.text.as_str(),
                &lexed.stream,
                ParseOptions {
                    relaxed_semicolons: relaxed,
                    ..ParseOptions::default()
                },
                &mut diagnostics,
            );
            entries.push(Entry {
                path: source.path.clone(),
                display,
                file,
                parsed,
            });
        }
        ranges.push((start, entries.len()));
    }
    reporter.diagnostics(diagnostics.diagnostics(), &sources);
    if diagnostics.has_errors() {
        return 1;
    }

    let parsed_files: Vec<ParsedFile> = entries.iter().map(|entry| entry.parsed.clone()).collect();
    let root_range = ranges[graph.root.0];
    let test_indexes: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            *index >= root_range.0
                && *index < root_range.1
                && root
                    .test_sources
                    .iter()
                    .any(|source| source.path == entry.path)
        })
        .map(|(index, _)| index)
        .collect();

    let (analysis, analysis_diagnostics) = analyze_sources(
        &graph,
        &ranges,
        &test_indexes,
        &parsed_files,
        &root.manifest,
    );
    reporter.diagnostics(&analysis_diagnostics, &sources);
    if analysis_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return 1;
    }

    let declarations = DeclarationIndex::build(&analysis.hir, &parsed_files);
    let profile_input = load_profile_input(root);
    let profile = profile_input.as_ref().map(|input| RenameProfile {
        path: input.path.clone(),
        text: input
            .sources
            .file(input.file)
            .map_or("", stc_span::SourceFile::text),
        sources: &input.sources,
        file: input.file,
        profile: input.profile.as_ref(),
        error: input.error.clone(),
    });
    let graph_ref = &graph;
    let analyzer: Box<RenameAnalyzer<'_>> = Box::new(move |parsed, manifest| {
        analyze_sources(graph_ref, &ranges, &test_indexes, parsed, manifest)
    });
    let context = RenameContext {
        hir: &analysis.hir,
        resolutions: &analysis.resolutions,
        parsed: &parsed_files,
        sources: &sources,
        declarations: &declarations,
        relaxed,
        root_package: graph.root.0,
        root_namespace: &root.namespace,
        manifest: &root.manifest,
        manifest_sources: &graph.sources,
        manifest_file: graph.root_manifest_file,
        profile,
        baseline_diagnostics: analysis_diagnostics.as_slice(),
        analyzer,
    };

    let Some(target) = context.resolve_path(&old_segments) else {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            &format!("`{old}` does not name a declaration in the current package"),
        );
        return 2;
    };
    let result = match context.session(target).rename(new_name) {
        Ok(result) => result,
        Err(RenameError::SameName) => {
            return usage(reporter, "the old and new names are the same");
        }
        Err(error) => {
            reporter.plain("STC-E-PROJECT-013", Severity::Error, &error.to_string());
            return 2;
        }
    };

    for warning in &result.warnings {
        print_warning(reporter, warning);
    }

    let decision = history_decision(&analysis.hir, target, old_name, new_name, &result);
    if dry_run {
        print_edits(
            &entries,
            &graph,
            profile_input.as_ref(),
            &result,
            old,
            new,
            &decision,
        );
        return 0;
    }

    if let Err(message) = apply_rename_edits(
        &result.edits,
        &entries,
        &graph,
        root,
        profile_input.as_ref(),
    ) {
        reporter.plain("STC-E-PROJECT-013", Severity::Error, &message);
        return 2;
    }

    let history_path = root.root_dir.join("stc.renames.toml");
    if result.history.is_empty() {
        log.status(format!(
            "renamed {old} -> {new} in {} edit(s); {decision}",
            result.edits.len()
        ));
    } else {
        let entries: Vec<RenameEntry> = result
            .history
            .iter()
            .map(|entry| RenameEntry::new(entry.old.clone(), entry.new.clone()))
            .collect();
        match append_renames_file(&history_path, &entries) {
            Ok(count) => log.status(format!(
                "renamed {old} -> {new} in {} edit(s); recorded {count} entr{} in {}",
                result.edits.len(),
                if count == 1 { "y" } else { "ies" },
                history_path.display()
            )),
            Err(error) => {
                reporter.plain(error.code, Severity::Error, &error.message);
                return 2;
            }
        }
    }
    0
}

fn usage(reporter: &mut Reporter, message: &str) -> u8 {
    reporter.plain("STC-E-BUILD-004", Severity::Error, message);
    2
}

/// Splits and validates a dotted rename key; `None` for empty or invalid
/// segments.
fn parse_key(text: &str) -> Option<Vec<String>> {
    if text.is_empty() {
        return None;
    }
    let segments: Vec<String> = text.split('.').map(str::to_owned).collect();
    if segments
        .iter()
        .any(|segment| !stc_lexer::is_identifier(segment))
    {
        return None;
    }
    Some(segments)
}

/// Loads and parses the selected machine profile without failing the rename.
fn load_profile_input(root: &Package) -> Option<ProfileInput> {
    let manifest = &root.manifest;
    let selected = manifest.io.config.as_deref()?;
    let mut sources = SourceMap::new();
    match load_project_profile(&root.root_dir, manifest) {
        Ok(Some(loaded)) => {
            let file = sources.add_file(loaded.selected.clone(), loaded.text.clone());
            match parse_profile(&loaded.text, file) {
                Ok(profile) => Some(ProfileInput {
                    path: loaded.path,
                    sources,
                    file,
                    profile: Some(profile),
                    error: None,
                }),
                Err(diagnostics) => Some(ProfileInput {
                    path: loaded.path,
                    sources,
                    file,
                    profile: None,
                    error: Some(profile_error(&diagnostics)),
                }),
            }
        }
        Ok(None) => None,
        Err(error) => {
            let file = sources.add_file(selected.to_owned(), String::new());
            Some(ProfileInput {
                path: PathBuf::new(),
                sources,
                file,
                profile: None,
                error: Some(error.message),
            })
        }
    }
}

fn profile_error(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return "the selected machine profile is invalid".to_owned();
    }
    diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Re-runs semantic analysis over the parsed sources and a manifest model.
fn analyze_sources(
    graph: &ProjectGraph,
    ranges: &[(usize, usize)],
    test_indexes: &[usize],
    parsed: &[ParsedFile],
    manifest: &Manifest,
) -> (Analysis, Vec<Diagnostic>) {
    let packages: Vec<PackageSession<'_>> = graph
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let (start, end) = ranges[index];
            PackageSession {
                name: package.name.clone(),
                root_namespace: package.namespace.clone(),
                dependencies: package
                    .dependencies
                    .iter()
                    .map(|dependency| dependency.package.0)
                    .collect(),
                files: &parsed[start..end],
                is_root: package.is_root,
            }
        })
        .collect();
    let test_files: Vec<ParsedFile> = test_indexes
        .iter()
        .map(|index| parsed[*index].clone())
        .collect();
    let session = ProjectSession {
        packages,
        configuration: manifest.project.configuration.clone(),
        stdlib: manifest.project.stdlib,
        test_build: true,
        test_files: &test_files,
    };
    let mut diagnostics = DiagnosticSink::new();
    let analysis = stc_sema::analyze_project(&session, &mut diagnostics);
    (analysis, diagnostics.into_diagnostics())
}

fn print_warning(reporter: &mut Reporter, warning: &RenameWarning) {
    let RenameWarning::ProfileUnavailable(message) = warning;
    reporter.plain(
        "STC-E-PROJECT-009",
        Severity::Warning,
        &format!("selected machine profile unavailable; profile edits skipped: {message}"),
    );
}

fn history_decision(
    hir: &HirCrate,
    target: RenameTarget,
    old_name: &str,
    new_name: &str,
    result: &RenameResult,
) -> String {
    if !result.history.is_empty() {
        return result
            .history
            .iter()
            .map(|entry| format!("{} -> {}", entry.old, entry.new))
            .collect::<Vec<_>>()
            .join(", ");
    }
    if old_name.eq_ignore_ascii_case(new_name) && old_name != new_name {
        return "no history entry (case-only)".to_owned();
    }
    if matches!(target, RenameTarget::Param(_) | RenameTarget::Local(_)) {
        return "no history entry (no GOT identity)".to_owned();
    }
    if is_configuration(hir, target) {
        return if matches!(result.online, RenameOnline::Cold(_)) {
            "no history entry (configuration; the next online download may be cold)".to_owned()
        } else {
            "no history entry (configuration)".to_owned()
        };
    }
    "no history entry".to_owned()
}

fn is_configuration(hir: &HirCrate, target: RenameTarget) -> bool {
    matches!(
        target,
        RenameTarget::Def(def)
            if matches!(&hir.defs[def.index()].kind, DefKind::Configuration { .. })
    )
}

fn print_edits(
    entries: &[Entry],
    graph: &ProjectGraph,
    profile: Option<&ProfileInput>,
    result: &RenameResult,
    old: &str,
    new: &str,
    decision: &str,
) {
    for edit in &result.edits {
        let Some((text, display)) = edit_text(edit, entries, graph, profile) else {
            continue;
        };
        let (line, column) = line_column(text, edit.start as usize);
        println!("{display}:{line}:{column}: {old} -> {new}");
    }
    println!("history: {decision}");
}

fn edit_text<'a>(
    edit: &RenameEdit,
    entries: &'a [Entry],
    graph: &'a ProjectGraph,
    profile: Option<&'a ProfileInput>,
) -> Option<(&'a str, String)> {
    match edit.file {
        RenameFile::Source(file) => {
            let entry = entries.iter().find(|entry| entry.file == file)?;
            Some((entry.parsed.source(), entry.display.clone()))
        }
        RenameFile::Manifest => {
            let file = graph.sources.file(graph.root_manifest_file)?;
            Some((
                file.text(),
                graph
                    .sources
                    .file_path(graph.root_manifest_file)
                    .unwrap_or("stc.toml")
                    .to_owned(),
            ))
        }
        RenameFile::Profile => {
            let input = profile?;
            let file = input.sources.file(input.file)?;
            Some((file.text(), input.path.display().to_string()))
        }
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rfind('\n')
        .map_or(offset, |index| offset - index - 1)
        + 1;
    (line, column)
}

fn apply_rename_edits(
    edits: &[RenameEdit],
    entries: &[Entry],
    graph: &ProjectGraph,
    root: &Package,
    profile: Option<&ProfileInput>,
) -> Result<(), String> {
    let mut source_edits: BTreeMap<usize, Vec<&RenameEdit>> = BTreeMap::new();
    let mut manifest_edits = Vec::new();
    let mut profile_edits = Vec::new();
    for edit in edits {
        match edit.file {
            RenameFile::Source(file) => source_edits.entry(file.index()).or_default().push(edit),
            RenameFile::Manifest => manifest_edits.push(edit),
            RenameFile::Profile => profile_edits.push(edit),
        }
    }

    let mut plans = Vec::new();
    for (index, group) in source_edits {
        let entry = entries
            .iter()
            .find(|entry| entry.file.index() == index)
            .ok_or_else(|| format!("rename references an unknown source file {index}"))?;
        let text = edited_text(entry.parsed.source(), &group, &entry.display)?;
        plans.push(WritePlan {
            path: entry.path.clone(),
            display: entry.display.clone(),
            text,
        });
    }
    if !manifest_edits.is_empty() {
        let file = graph
            .sources
            .file(graph.root_manifest_file)
            .ok_or_else(|| "the root manifest text is unavailable".to_owned())?;
        let text = edited_text(file.text(), &manifest_edits, "stc.toml")?;
        plans.push(WritePlan {
            path: root.manifest_path.clone(),
            display: graph
                .sources
                .file_path(graph.root_manifest_file)
                .unwrap_or("stc.toml")
                .to_owned(),
            text,
        });
    }
    if !profile_edits.is_empty() {
        let input = profile.ok_or_else(|| {
            "rename edits the selected profile but the profile is unavailable".to_owned()
        })?;
        if input.path.as_os_str().is_empty() {
            return Err("rename edits the selected profile but its path is unavailable".to_owned());
        }
        let file = input
            .sources
            .file(input.file)
            .ok_or_else(|| "the selected profile text is unavailable".to_owned())?;
        let text = edited_text(
            file.text(),
            &profile_edits,
            &input.path.display().to_string(),
        )?;
        plans.push(WritePlan {
            path: input.path.clone(),
            display: input.path.display().to_string(),
            text,
        });
    }

    let mut changed = Vec::new();
    for plan in plans {
        if let Err(error) = std::fs::write(&plan.path, &plan.text) {
            let changed = if changed.is_empty() {
                "no files changed".to_owned()
            } else {
                format!("changed: {}", changed.join(", "))
            };
            return Err(format!(
                "cannot write `{}`: {error}; {changed}",
                plan.path.display()
            ));
        }
        changed.push(plan.display);
    }
    Ok(())
}

fn edited_text(text: &str, edits: &[&RenameEdit], display: &str) -> Result<String, String> {
    let mut sorted = edits.to_vec();
    sorted.sort_by(|left, right| {
        right
            .start
            .cmp(&left.start)
            .then_with(|| right.end.cmp(&left.end))
    });
    let mut output = text.to_owned();
    let mut limit = text.len();
    for edit in sorted {
        let start = usize::try_from(edit.start)
            .map_err(|_| format!("rename edit start is outside `{display}`"))?;
        let end = usize::try_from(edit.end)
            .map_err(|_| format!("rename edit end is outside `{display}`"))?;
        if start >= end || end > limit || end > text.len() {
            return Err(format!(
                "rename edit range {start}..{end} is outside `{display}`"
            ));
        }
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return Err(format!(
                "rename edit range is not on a UTF-8 boundary in `{display}`"
            ));
        }
        if edit.new_text.is_empty() {
            return Err(format!(
                "rename edit in `{display}` has an empty replacement"
            ));
        }
        output.replace_range(start..end, &edit.new_text);
        limit = start;
    }
    Ok(output)
}

/// Reads a rename history and merges it with CLI `--rename` entries.
///
/// A missing history is empty. Errors are returned for the caller to report.
///
/// # Errors
///
/// Returns the history or merge diagnostic.
pub(crate) fn merged_project_renames(
    root_dir: &Path,
    cli: &[(String, String)],
) -> Result<Vec<(String, String)>, stc_manifest::RenameHistoryError> {
    let history = load_rename_history(&root_dir.join("stc.renames.toml"))?;
    merge_renames(&history.entries, cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_require_identifier_segments() {
        assert_eq!(
            parse_key("Main.Speed"),
            Some(vec!["Main".into(), "Speed".into()])
        );
        assert!(parse_key("").is_none());
        assert!(parse_key("Main.").is_none());
        assert!(parse_key("Main.IF").is_none());
        assert!(parse_key("Main.1x").is_none());
    }

    #[test]
    fn edits_are_validated_before_application() {
        let edit = |start, end| RenameEdit {
            file: RenameFile::Manifest,
            start,
            end,
            new_text: "X".into(),
        };
        assert_eq!(
            edited_text("abcdef", &[&edit(1, 3)], "stc.toml").expect("valid"),
            "aXdef"
        );
        assert!(edited_text("abcdef", &[&edit(2, 5), &edit(1, 3)], "stc.toml").is_err());
        assert!(edited_text("abcdef", &[&edit(1, 9)], "stc.toml").is_err());
    }
}
