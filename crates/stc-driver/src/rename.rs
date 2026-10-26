//! `stc rename`: source-aware tracked rename and the `stc.renames.toml`
//! history (CLI §3.1, D-156).
//!
//! The command analyzes the project with the test build surface, resolves the
//! qualified key to a declaration in the root package, collects every sema
//! resolution that names it (plus qualified path prefixes for POU/type/
//! namespace targets), rewrites only identifier tokens, and appends the
//! mapping to the committed history after every file write succeeds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stc_diag::{DiagnosticSink, Severity};
use stc_hir::{DefId, DefKind, FieldId, HirCrate, LocalId, ParamId};
use stc_lexer::{TokenKind, tokenize_bytes};
use stc_manifest::{
    LoadOptions, append_rename, load_project_readonly, merge_renames, render_rename_history,
};
use stc_parser::{ParseOptions, ParsedFile, parse};
use stc_sema::{Analysis, PackageSession, ProjectSession, Res};
use stc_span::{FileId, SourceMap, Span};
use stc_syntax::SyntaxKind;
use stc_syntax::ast::{AstNode, Name};

use crate::build::Request;
use crate::report::{Log, Reporter};

/// One parsed project source.
struct Entry {
    path: PathBuf,
    display: String,
    file: FileId,
    parsed: ParsedFile,
}

/// The declaration selected by the old key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Def(DefId),
    Field(FieldId),
    Param(ParamId),
    Local(LocalId),
    EnumValue(DefId, i128),
}

/// One non-overlapping source edit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edit {
    file: FileId,
    start: usize,
    end: usize,
    replacement: String,
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
    if !valid_identifier(new_name) {
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
                        Span::new(mapped, 0, 0),
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
    let packages: Vec<PackageSession<'_>> = graph
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
            files: &parsed_files[ranges[index].0..ranges[index].1],
            is_root: package.is_root,
        })
        .collect();
    let test_files: Vec<ParsedFile> = entries
        .iter()
        .filter(|entry| entry.file.index() >= ranges[graph.root.0].0)
        .filter(|entry| {
            root.test_sources
                .iter()
                .any(|source| source.path == entry.path)
        })
        .map(|entry| entry.parsed.clone())
        .collect();
    let session = ProjectSession {
        packages,
        configuration: root.manifest.project.configuration.clone(),
        stdlib: root.manifest.project.stdlib,
        test_build: true,
        test_files: &test_files,
    };
    let mut analysis_diagnostics = DiagnosticSink::new();
    let analysis = stc_sema::analyze_project(&session, &mut analysis_diagnostics);
    reporter.diagnostics(analysis_diagnostics.diagnostics(), &sources);
    if analysis_diagnostics.has_errors() {
        return 1;
    }

    let index = DeclarationIndex::build(&analysis.hir, graph.root.0, &root.namespace);
    let Some(target) = index.resolve(&old_segments) else {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            &format!("`{old}` does not name a declaration in the current package"),
        );
        return 2;
    };
    if let Some(existing) = index.resolve(&new_segments)
        && existing != target
    {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            &format!("`{new}` already names a declaration in the current package"),
        );
        return 2;
    }

    let mut edits = Vec::new();
    if !index.declaration_edit(
        target,
        &analysis.hir,
        &entries,
        new_name,
        &mut edits,
        reporter,
    ) {
        return 2;
    }
    collect_reference_edits(&analysis, target, old_name, new_name, &entries, &mut edits);
    if let Target::Def(def) = target {
        let kind = &analysis.hir.defs[def.index()].kind;
        if index.is_path_prefix_kind(kind) {
            collect_prefix_edits(
                &entries,
                &index.path_of(target).expect("def has a path"),
                new_name,
                &mut edits,
            );
        }
    }
    edits.sort();
    edits.dedup();
    if edits.is_empty() {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            &format!("`{old}` has no source occurrences to rewrite"),
        );
        return 2;
    }
    if edits_overlap(&edits) {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            "rename produced overlapping source edits",
        );
        return 2;
    }

    if dry_run {
        print_edits(&entries, &sources, &edits, old, new);
        return 0;
    }

    if let Err(message) = apply_edits(&entries, &edits) {
        reporter.plain("STC-E-PROJECT-013", Severity::Error, &message);
        return 2;
    }

    let history_path = root.root_dir.join("stc.renames.toml");
    let mut history = match stc_manifest::load_rename_history(&history_path) {
        Ok(history) => history,
        Err(error) => {
            reporter.plain(error.code, Severity::Error, &error.message);
            return 2;
        }
    };
    if let Err(error) = append_rename(&mut history, old, new) {
        reporter.plain(error.code, Severity::Error, &error.message);
        return 2;
    }
    if let Err(error) = std::fs::write(&history_path, render_rename_history(&history)) {
        reporter.plain(
            "STC-E-PROJECT-013",
            Severity::Error,
            &format!("cannot write `{}`: {error}", history_path.display()),
        );
        return 2;
    }
    log.status(format!(
        "renamed {old} -> {new} in {} edit(s); recorded {}",
        edits.len(),
        history_path.display()
    ));
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
    if segments.iter().any(|segment| !valid_identifier(segment)) {
        return None;
    }
    Some(segments)
}

/// Returns whether `text` lexes as one identifier.
fn valid_identifier(text: &str) -> bool {
    let mut diagnostics = DiagnosticSink::new();
    let lexed = tokenize_bytes(FileId::new(0), text.as_bytes(), &mut diagnostics);
    if diagnostics.has_errors() || lexed.stream.tokens.len() != 2 {
        return false;
    }
    matches!(lexed.stream.tokens[0].kind, TokenKind::Ident(_))
        && lexed.stream.tokens[1].kind.is_eof()
}

/// Declaration lookup for the root package.
struct DeclarationIndex {
    paths: Vec<(Vec<String>, Target)>,
    by_key: BTreeMap<String, Vec<Target>>,
    package_name: String,
    namespace: String,
}

impl DeclarationIndex {
    fn build(hir: &HirCrate, root_package: usize, namespace: &str) -> Self {
        let mut index = Self {
            paths: Vec::new(),
            by_key: BTreeMap::new(),
            package_name: namespace.to_owned(),
            namespace: namespace.to_owned(),
        };
        for (def_index, def) in hir.defs.iter().enumerate() {
            if def.package.index() != root_package {
                continue;
            }
            let Ok(def_id) = u32::try_from(def_index).map(DefId::new) else {
                continue;
            };
            let Some(path) = def_path(hir, def_id, namespace) else {
                continue;
            };
            let target = Target::Def(def_id);
            index.add(path.clone(), target);
            for field in def.field_ids() {
                let field_data = hir.field(*field);
                let mut field_path = path.clone();
                field_path.push(hir.interner.text(field_data.name).to_string());
                index.add(field_path, Target::Field(*field));
            }
            for method in def.method_ids() {
                let method_data = &hir.defs[method.index()];
                let Some(name) = method_data.name else {
                    continue;
                };
                let mut method_path = path.clone();
                method_path.push(hir.interner.text(name).to_string());
                index.add(method_path, Target::Def(*method));
            }
            if let DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. } = &def.kind {
                for value in values {
                    let mut value_path = path.clone();
                    value_path.push(hir.interner.text(value.name).to_string());
                    index.add(value_path, Target::EnumValue(def_id, value.value));
                }
            }
        }
        for (param_index, param) in hir.params.iter().enumerate() {
            if param.owner.index() >= hir.defs.len()
                || hir.defs[param.owner.index()].package.index() != root_package
            {
                continue;
            }
            let Ok(param_id) = u32::try_from(param_index).map(ParamId::new) else {
                continue;
            };
            if let Some(mut path) = def_path(hir, param.owner, namespace) {
                path.push(hir.interner.text(param.name).to_string());
                index.add(path, Target::Param(param_id));
            }
        }
        for (local_index, local) in hir.locals.iter().enumerate() {
            if local.owner.index() >= hir.defs.len()
                || hir.defs[local.owner.index()].package.index() != root_package
            {
                continue;
            }
            let Ok(local_id) = u32::try_from(local_index).map(LocalId::new) else {
                continue;
            };
            if let Some(mut path) = def_path(hir, local.owner, namespace) {
                path.push(hir.interner.text(local.name).to_string());
                index.add(path, Target::Local(local_id));
            }
        }
        index
    }

    fn add(&mut self, path: Vec<String>, target: Target) {
        let key = normalize_path(&path);
        self.by_key.entry(key).or_default().push(target);
        self.paths.push((path, target));
    }

    fn resolve(&self, segments: &[String]) -> Option<Target> {
        let mut candidates = Vec::new();
        if let Some(target) = self.lookup(segments) {
            candidates.push(target);
        }
        if let Some(first) = segments.first()
            && (first.eq_ignore_ascii_case(&self.package_name)
                || first.eq_ignore_ascii_case(&self.namespace))
            && segments.len() > 1
            && let Some(target) = self.lookup(&segments[1..])
        {
            candidates.push(target);
        }
        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [] => None,
            [target] => Some(*target),
            _ => None,
        }
    }

    fn lookup(&self, segments: &[String]) -> Option<Target> {
        let key = normalize_path(segments);
        let targets = self.by_key.get(&key)?;
        (targets.len() == 1).then_some(targets[0])
    }

    fn path_of(&self, target: Target) -> Option<Vec<String>> {
        self.paths
            .iter()
            .find(|(_, candidate)| *candidate == target)
            .map(|(path, _)| path.clone())
    }

    fn is_path_prefix_kind(&self, kind: &DefKind) -> bool {
        matches!(
            kind,
            DefKind::Namespace { .. }
                | DefKind::Function { .. }
                | DefKind::FunctionBlock { .. }
                | DefKind::Program { .. }
                | DefKind::Class { .. }
                | DefKind::Interface { .. }
                | DefKind::Method { .. }
                | DefKind::Struct { .. }
                | DefKind::Enum { .. }
                | DefKind::NamedValue { .. }
                | DefKind::Subrange { .. }
                | DefKind::Alias { .. }
        )
    }

    fn declaration_edit(
        &self,
        target: Target,
        hir: &HirCrate,
        entries: &[Entry],
        new_name: &str,
        edits: &mut Vec<Edit>,
        reporter: &mut Reporter,
    ) -> bool {
        let (file, span, name) = match target {
            Target::Def(def) => {
                let data = &hir.defs[def.index()];
                let Some(name) = data.name else {
                    reporter.plain(
                        "STC-E-PROJECT-013",
                        Severity::Error,
                        "the declaration has no name",
                    );
                    return false;
                };
                (
                    data.span.file,
                    data.span,
                    hir.interner.text(name).to_string(),
                )
            }
            Target::Field(field) => {
                let data = hir.field(field);
                (
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_string(),
                )
            }
            Target::Param(param) => {
                let data = &hir.params[param.index()];
                (
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_string(),
                )
            }
            Target::Local(local) => {
                let data = &hir.locals[local.index()];
                (
                    data.span.file,
                    data.span,
                    hir.interner.text(data.name).to_string(),
                )
            }
            Target::EnumValue(def, value) => {
                let (DefKind::Enum { values, .. } | DefKind::NamedValue { values, .. }) =
                    &hir.defs[def.index()].kind
                else {
                    return false;
                };
                let Some(value) = values.iter().find(|candidate| candidate.value == value) else {
                    return false;
                };
                (
                    value.span.file,
                    value.span,
                    hir.interner.text(value.name).to_string(),
                )
            }
        };
        let Some(entry) = entries.iter().find(|entry| entry.file == file) else {
            return false;
        };
        let Some(token) = name_token(&entry.parsed, span, &name) else {
            reporter.plain(
                "STC-E-PROJECT-013",
                Severity::Error,
                &format!("cannot locate the declaration token for `{name}`"),
            );
            return false;
        };
        edits.push(Edit {
            file,
            start: token.0,
            end: token.1,
            replacement: new_name.to_owned(),
        });
        true
    }
}

/// Builds the source path of a definition in the root package.
fn def_path(hir: &HirCrate, def: DefId, namespace: &str) -> Option<Vec<String>> {
    let mut path = Vec::new();
    let mut current = Some(def);
    while let Some(id) = current {
        let data = &hir.defs[id.index()];
        let is_root_namespace =
            data.parent.is_none() && matches!(data.kind, DefKind::Namespace { .. });
        if is_root_namespace {
            break;
        }
        if let Some(name) = data.name {
            path.push(hir.interner.text(name).to_string());
        }
        current = data.parent;
    }
    if path
        .last()
        .is_some_and(|last| last.eq_ignore_ascii_case(namespace))
    {
        path.pop();
    }
    path.reverse();
    (!path.is_empty()).then_some(path)
}

fn normalize_path(path: &[String]) -> String {
    path.iter()
        .map(|segment| segment.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(".")
}

/// Finds the identifier token for a declaration span and name.
fn name_token(parsed: &ParsedFile, span: Span, name: &str) -> Option<(usize, usize)> {
    let mut tokens = Vec::new();
    collect_tokens(&parsed.tree().root(), &mut tokens);
    tokens
        .into_iter()
        .filter(|token| {
            token.kind() == SyntaxKind::Ident
                && token.span().start >= span.start
                && token.span().end <= span.end
                && token.text().eq_ignore_ascii_case(name)
        })
        .map(|token| (token.span().start as usize, token.span().end as usize))
        .next()
}

fn collect_tokens(node: &stc_syntax::SyntaxNode, out: &mut Vec<stc_syntax::SyntaxToken>) {
    for child in node.children() {
        match child {
            stc_syntax::SyntaxElement::Node(node) => collect_tokens(&node, out),
            stc_syntax::SyntaxElement::Token(token) => out.push(token),
            stc_syntax::SyntaxElement::Trivia(_) => {}
        }
    }
}

fn collect_reference_edits(
    analysis: &Analysis,
    target: Target,
    old_name: &str,
    new_name: &str,
    entries: &[Entry],
    edits: &mut Vec<Edit>,
) {
    for ((file, node_id), resolution) in &analysis.resolutions {
        if !matches_target(analysis, resolution, target) {
            continue;
        }
        let Some(entry) = entries.iter().find(|entry| entry.file == *file) else {
            continue;
        };
        let tree = entry.parsed.tree();
        let node = tree.node(*node_id);
        let mut tokens = Vec::new();
        collect_tokens(&node, &mut tokens);
        let mut matching: Vec<_> = tokens
            .into_iter()
            .filter(|token| {
                token.kind() == SyntaxKind::Ident && token.text().eq_ignore_ascii_case(old_name)
            })
            .collect();
        let Some(token) = matching.pop() else {
            continue;
        };
        edits.push(Edit {
            file: *file,
            start: token.span().start as usize,
            end: token.span().end as usize,
            replacement: new_name.to_owned(),
        });
    }
}

fn matches_target(analysis: &Analysis, resolution: &Res, target: Target) -> bool {
    match (resolution, target) {
        (Res::Def(def), Target::Def(expected)) => *def == expected,
        (Res::Field(field), Target::Field(expected)) => *field == expected,
        (Res::Param(param), Target::Param(expected)) => *param == expected,
        (Res::Local(local), Target::Local(expected)) => *local == expected,
        (Res::EnumValue(def, value), Target::EnumValue(expected_def, expected_value)) => {
            *def == expected_def && *value == expected_value
        }
        (Res::Type(ty), Target::Def(expected)) => {
            matches!(analysis.hir.types.get(*ty), stc_hir::Ty::Named(def) if *def == expected)
        }
        _ => false,
    }
}

/// Rewrites qualified path prefixes for POU/type/namespace targets.
fn collect_prefix_edits(
    entries: &[Entry],
    target_path: &[String],
    new_name: &str,
    edits: &mut Vec<Edit>,
) {
    for entry in entries {
        let mut nodes = Vec::new();
        collect_nodes(&entry.parsed.tree().root(), &mut nodes);
        for node in nodes {
            if node.kind() != SyntaxKind::PathExpr && node.kind() != SyntaxKind::Path {
                continue;
            }
            let names: Vec<Name> = node.child_nodes().iter().filter_map(Name::cast).collect();
            if names.len() < target_path.len() {
                continue;
            }
            for start in 0..=names.len() - target_path.len() {
                let window = &names[start..start + target_path.len()];
                if !window
                    .iter()
                    .zip(target_path)
                    .all(|(name, expected)| name.text().eq_ignore_ascii_case(expected))
                {
                    continue;
                }
                let name = &window[target_path.len() - 1];
                let Some(token) = name.token() else {
                    continue;
                };
                edits.push(Edit {
                    file: entry.file,
                    start: token.span().start as usize,
                    end: token.span().end as usize,
                    replacement: new_name.to_owned(),
                });
            }
        }
    }
}

fn collect_nodes(node: &stc_syntax::SyntaxNode, out: &mut Vec<stc_syntax::SyntaxNode>) {
    out.push(node.clone());
    for child in node.child_nodes() {
        collect_nodes(&child, out);
    }
}

fn edits_overlap(edits: &[Edit]) -> bool {
    for pair in edits.windows(2) {
        let (left, right) = (&pair[0], &pair[1]);
        if left.file == right.file && right.start < left.end {
            return true;
        }
    }
    false
}

fn print_edits(
    entries: &[Entry],
    sources: &SourceMap,
    edits: &[Edit],
    old_key: &str,
    new_key: &str,
) {
    for edit in edits {
        let Some(entry) = entries.iter().find(|entry| entry.file == edit.file) else {
            continue;
        };
        let (line, column) = line_column(entry.parsed.source(), edit.start);
        let path = sources
            .file_path(edit.file)
            .unwrap_or(entry.display.as_str());
        println!("{path}:{line}:{column}: {old_key} -> {new_key}");
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

fn apply_edits(entries: &[Entry], edits: &[Edit]) -> Result<(), String> {
    let mut by_file: BTreeMap<FileId, Vec<&Edit>> = BTreeMap::new();
    for edit in edits {
        by_file.entry(edit.file).or_default().push(edit);
    }
    let mut updates = Vec::new();
    for (file, mut file_edits) in by_file {
        let Some(entry) = entries.iter().find(|entry| entry.file == file) else {
            continue;
        };
        file_edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
        let mut text = entry.parsed.source().to_owned();
        for edit in file_edits {
            let Some(slice) = text.get(edit.start..edit.end) else {
                return Err(format!(
                    "edit range {}..{} is outside `{}`",
                    edit.start, edit.end, entry.display
                ));
            };
            if slice.is_empty() {
                return Err(format!("empty edit range in `{}`", entry.display));
            }
            text.replace_range(edit.start..edit.end, &edit.replacement);
        }
        updates.push((entry.path.clone(), text));
    }
    for (path, text) in updates {
        std::fs::write(&path, text)
            .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
    }
    Ok(())
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
    let history = stc_manifest::load_rename_history(&root_dir.join("stc.renames.toml"))?;
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
    fn paths_normalize_case_insensitively() {
        assert_eq!(
            normalize_path(&["Main".into(), "speed".into()]),
            "MAIN.SPEED"
        );
    }
}
