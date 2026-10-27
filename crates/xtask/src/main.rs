//! Repository tasks: suite selection for CI and the rule traceability report.
//!
//! `cargo xtask test --suite <name> [--locked]` runs one driver integration
//! suite. `cargo xtask traceability [--check]` maps `STC-*` rule IDs found in
//! `Specification/**/*.md` to evidence in the tests and conformance corpus.
//! The traceability allowlist (`tools/ci/traceability-allowlist.toml`) records
//! rules that are not implemented and IDs that are examples, not rules.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Rules that are not evidence-bearing rules.
const ALLOWLIST: &str = "tools/ci/traceability-allowlist.toml";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(|| usage().to_owned())?;
    match command.as_str() {
        "test" => test(rest),
        "traceability" => traceability(rest),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown task `{other}`\n{}", usage())),
    }
}

fn usage() -> &'static str {
    "usage:\n  \
     cargo xtask test --suite ui|conformance|execution|projects [--locked] [-- <args>]\n  \
     cargo xtask traceability [--check]"
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn test(args: &[String]) -> Result<(), String> {
    let mut suite = None;
    let mut locked = false;
    let mut passthrough = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--suite" => {
                index += 1;
                suite = Some(
                    args.get(index)
                        .ok_or_else(|| "`--suite` needs a value".to_owned())?
                        .clone(),
                );
            }
            "--locked" => locked = true,
            "--" => {
                passthrough.extend_from_slice(&args[index + 1..]);
                break;
            }
            other if other.starts_with("--suite=") => {
                suite = Some(other["--suite=".len()..].to_owned());
            }
            other => return Err(format!("unknown `cargo xtask test` option `{other}`")),
        }
        index += 1;
    }
    let suite = suite.ok_or_else(|| "`cargo xtask test` needs `--suite <name>`".to_owned())?;
    let tests: &[&str] = match suite.as_str() {
        "ui" => &["ui"],
        "conformance" => &["conformance"],
        "execution" => &["execution", "exact_vectors_cli"],
        "projects" => &["projects", "online_cli"],
        other => {
            return Err(format!(
                "unknown suite `{other}` (expected `ui`, `conformance`, `execution`, or `projects`)"
            ));
        }
    };
    for test in tests {
        let mut command = Command::new(cargo());
        command
            .current_dir(workspace_root())
            .args(["test", "-p", "stc-driver", "--test", test]);
        if locked {
            command.arg("--locked");
        }
        if !passthrough.is_empty() {
            command.arg("--").args(&passthrough);
        }
        eprintln!("xtask: cargo test -p stc-driver --test {test}");
        let status = command
            .status()
            .map_err(|error| format!("cannot run cargo: {error}"))?;
        if !status.success() {
            return Err(format!(
                "suite `{suite}` failed in test target `{test}` ({status})"
            ));
        }
    }
    Ok(())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn traceability(args: &[String]) -> Result<(), String> {
    let mut check = false;
    for argument in args {
        match argument.as_str() {
            "--check" => check = true,
            "--help" | "-h" => {
                println!("usage: cargo xtask traceability [--check]");
                return Ok(());
            }
            other => {
                return Err(format!(
                    "unknown `cargo xtask traceability` option `{other}`"
                ));
            }
        }
    }
    let root = workspace_root();
    let rules = spec_rules(&root)?;
    let allow = read_allowlist(&root)?;
    let evidence = evidence(&root)?;

    let mut not_implemented = Vec::new();
    let mut examples = Vec::new();
    let mut unknown_allow = Vec::new();
    let mut stale_allow = Vec::new();
    let mut implemented = BTreeSet::new();
    for rule in &rules {
        match allow.get(rule) {
            None => {
                implemented.insert(rule.clone());
            }
            Some(entry) => match entry.status.as_str() {
                "not-implemented" => {
                    not_implemented.push(rule.clone());
                    if evidence.contains_key(rule) {
                        stale_allow.push(rule.clone());
                    }
                }
                "example" => {
                    examples.push(rule.clone());
                    if evidence.contains_key(rule) {
                        stale_allow.push(rule.clone());
                    }
                }
                other => {
                    return Err(format!(
                        "allowlist entry `{rule}` has unknown status `{other}` \
                         (expected `not-implemented` or `example`)"
                    ));
                }
            },
        }
    }
    for rule in allow.keys() {
        if !rules.contains(rule) {
            unknown_allow.push(rule.clone());
        }
    }

    let covered: Vec<&String> = implemented
        .iter()
        .filter(|rule| evidence.contains_key(*rule))
        .collect();
    let uncovered: Vec<&String> = implemented
        .iter()
        .filter(|rule| !evidence.contains_key(*rule))
        .collect();

    println!(
        "traceability: {} spec rules; {} implemented, {} covered, {} uncovered; \
         {} not implemented; {} example IDs",
        rules.len(),
        implemented.len(),
        covered.len(),
        uncovered.len(),
        not_implemented.len(),
        examples.len()
    );
    if !not_implemented.is_empty() {
        println!("\nnot implemented (allowlisted):");
        for rule in &not_implemented {
            println!("  {rule}  {}", allow[rule].reason);
        }
    }
    if !examples.is_empty() {
        println!("\nexample identifiers (allowlisted, not rules):");
        for rule in &examples {
            println!("  {rule}  {}", allow[rule].reason);
        }
    }
    if !uncovered.is_empty() {
        println!("\nuncovered implemented rules:");
        for rule in &uncovered {
            println!("  {rule}");
        }
    }
    if !unknown_allow.is_empty() {
        println!("\nallowlist entries that are not spec rules:");
        for rule in &unknown_allow {
            println!("  {rule}");
        }
    }
    if !stale_allow.is_empty() {
        println!("\nallowlisted rules that now have test evidence:");
        for rule in &stale_allow {
            println!("  {rule}");
        }
    }

    if check && !(uncovered.is_empty() && unknown_allow.is_empty() && stale_allow.is_empty()) {
        return Err("traceability check failed".into());
    }
    Ok(())
}

#[derive(Debug)]
struct AllowEntry {
    status: String,
    reason: String,
}

fn read_allowlist(root: &Path) -> Result<BTreeMap<String, AllowEntry>, String> {
    let path = root.join(ALLOWLIST);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error| format!("cannot parse `{}`: {error}", path.display()))?;
    let mut entries = BTreeMap::new();
    let Some(tables) = document
        .get("rule")
        .and_then(toml_edit::Item::as_array_of_tables)
    else {
        return Err(format!("`{}` has no `[[rule]]` entries", path.display()));
    };
    for table in tables {
        let id = table
            .get("id")
            .and_then(toml_edit::Item::as_str)
            .ok_or_else(|| format!("`{}` entry without an `id`", path.display()))?;
        let status = table
            .get("status")
            .and_then(toml_edit::Item::as_str)
            .ok_or_else(|| format!("allowlist entry `{id}` has no `status`"))?;
        let reason = table
            .get("reason")
            .and_then(toml_edit::Item::as_str)
            .ok_or_else(|| format!("allowlist entry `{id}` has no `reason`"))?;
        if entries
            .insert(
                id.to_owned(),
                AllowEntry {
                    status: status.to_owned(),
                    reason: reason.to_owned(),
                },
            )
            .is_some()
        {
            return Err(format!("duplicate allowlist entry `{id}`"));
        }
    }
    Ok(entries)
}

fn spec_rules(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut files = Vec::new();
    collect_files(&root.join("Specification"), &mut files)?;
    let mut rules = BTreeSet::new();
    for file in files {
        if file.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let text = read_text(&file);
        rules.extend(ids_in(&text).into_iter().map(str::to_owned));
    }
    Ok(rules)
}

/// Collects test evidence: integration tests, unit-test sections, and the
/// `tests/` corpus. Implementation constants and code lists are excluded, so a
/// rule is not "covered" merely because the compiler defines its code.
fn evidence(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut files = Vec::new();
    let mut source_files = Vec::new();
    let crates = root.join("crates");
    for entry in fs::read_dir(&crates)
        .map_err(|error| format!("cannot read `{}`: {error}", crates.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let crate_dir = entry.path();
        if !crate_dir.is_dir() || crate_dir.file_name().is_some_and(|name| name == "xtask") {
            continue;
        }
        let tests = crate_dir.join("tests");
        if tests.is_dir() {
            collect_files(&tests, &mut files)?;
        }
        let src = crate_dir.join("src");
        if src.is_dir() {
            collect_files(&src, &mut files)?;
            collect_files(&src, &mut source_files)?;
        }
    }
    let tests = root.join("tests");
    if tests.is_dir() {
        collect_files(&tests, &mut files)?;
    }

    // Tests often assert on a shared constant (`codes::ABI_MISMATCH`) instead
    // of spelling the code. Resolve those constants to their rule IDs.
    let constants = code_constants(&source_files);
    let mut evidence: BTreeMap<String, PathBuf> = BTreeMap::new();
    for file in files {
        let Some(extension) = file.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        if !matches!(
            extension,
            "rs" | "st" | "toml" | "json" | "stderr" | "c" | "h"
        ) {
            continue;
        }
        let text = read_text(&file);
        let section = if file.components().any(|part| part.as_os_str() == "src") {
            unit_test_sections(&text)
        } else {
            text
        };
        let mut ids: BTreeSet<String> = ids_in(&section).into_iter().map(str::to_owned).collect();
        for token in identifier_tokens(&section) {
            if let Some(id) = constants.get(token) {
                ids.insert(id.clone());
            }
        }
        for id in ids {
            evidence.entry(id).or_insert_with(|| file.clone());
        }
    }
    Ok(evidence)
}

/// Maps Rust constant names to the diagnostic code they hold. Only uppercase
/// constants are considered, and duplicate names are dropped as ambiguous.
fn code_constants(files: &[PathBuf]) -> BTreeMap<String, String> {
    let mut constants: BTreeMap<String, Option<String>> = BTreeMap::new();
    for file in files {
        if file.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let text = read_text(file);
        for line in text.lines() {
            let line = line.trim();
            let Some(index) = line.find("const ") else {
                continue;
            };
            let rest = &line[index + "const ".len()..];
            let Some((name, value)) = rest.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            {
                continue;
            }
            let Some((_, after)) = value.split_once('"') else {
                continue;
            };
            let Some((id, _)) = after.split_once('"') else {
                continue;
            };
            if !is_rule_id(id) {
                continue;
            }
            let slot = constants
                .entry(name.to_owned())
                .or_insert_with(|| Some(id.to_owned()));
            if slot.as_deref() != Some(id) {
                *slot = None;
            }
        }
    }
    constants
        .into_iter()
        .filter_map(|(name, id)| id.map(|id| (name, id)))
        .collect()
}

fn identifier_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| {
            !token.is_empty()
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

/// Returns the content after each `#[cfg(test)]` marker, joined with newlines.
/// Unit test modules are conventionally at the end of a source file.
fn unit_test_sections(text: &str) -> String {
    let mut sections = String::new();
    for (index, _) in text.match_indices("#[cfg(test)]") {
        sections.push_str(&text[index..]);
        sections.push('\n');
    }
    sections
}

fn collect_files(directory: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !directory.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(
            name.as_ref(),
            "target" | ".git" | "snapshots" | "vectors" | "corpus" | "artifacts" | ".lake"
        ) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn read_text(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > 4 * 1024 * 1024 => String::new(),
        _ => fs::read_to_string(path).unwrap_or_default(),
    }
}

/// Extracts `STC-<AREA>-...-<NNN>` identifiers from free text.
fn ids_in(text: &str) -> Vec<&str> {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
    })
    .filter(|token| is_rule_id(token))
    .collect()
}

fn is_rule_id(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("STC-") else {
        return false;
    };
    let Some((body, last)) = rest.rsplit_once('-') else {
        return false;
    };
    if last.len() != 3 || !last.bytes().all(|byte| byte.is_ascii_digit()) || body.is_empty() {
        return false;
    }
    body.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    })
}
