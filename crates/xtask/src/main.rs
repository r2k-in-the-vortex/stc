//! Repository tasks: suite selection for CI and the rule traceability report.
//!
//! `cargo xtask test --suite <name> [--locked]` runs one integration suite,
//! which is one or more package/test-target pairs. `cargo xtask traceability
//! [--check]` maps `STC-*` rule IDs found in `Specification/**/*.md` to
//! evidence in the tests and conformance corpus. The traceability allowlist
//! (`tools/ci/traceability-allowlist.toml`) records rules that are not
//! implemented and IDs that are examples, not rules.

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
        "package" => package(rest),
        "host-audit" => host_audit(rest),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown task `{other}`\n{}", usage())),
    }
}

fn usage() -> &'static str {
    "usage:\n  \
     cargo xtask test --suite ui|conformance|execution|projects|debug|cross-target [--locked] [-- <args>]\n  \
     cargo xtask package [--out <dir>]\n  \
     cargo xtask traceability [--check]\n  \
     cargo xtask host-audit [--check]"
}

/// One test target: a workspace package and an integration test file.
type SuiteTarget = (&'static str, &'static str);

fn suite_targets(suite: &str) -> Option<&'static [SuiteTarget]> {
    match suite {
        "ui" => Some(&[("stc-driver", "ui")]),
        "conformance" => Some(&[("stc-driver", "conformance")]),
        "execution" => Some(&[
            ("stc-driver", "execution"),
            ("stc-driver", "exact_vectors_cli"),
        ]),
        "projects" => Some(&[
            ("stc-driver", "projects"),
            ("stc-driver", "online_cli"),
            ("stc-driver", "manifest_hardening"),
            ("stc-driver", "examples"),
        ]),
        // The full debug matrix: typed access/write/watch, stop/step/stack,
        // protocol/auth/audit, CLI launch/attach/swap, release hook absence,
        // frame sidecars, checkpoint and debug-info generation.
        "debug" => Some(&[
            ("stc-runtime", "debug_access"),
            ("stc-runtime", "debug_write"),
            ("stc-runtime", "debug_stop"),
            ("stc-runtime", "debug_view"),
            ("stc-runtime", "debug_protocol"),
            ("stc-driver", "debug_cli"),
            ("stc-driver", "debug_release"),
            ("stc-driver", "frames"),
            ("stc-codegen", "debug_info"),
            ("stc-codegen", "checkpoints"),
            ("stc-codegen", "frames"),
            ("stc-got", "metadata_v2"),
        ]),
        // M7 cross-target smoke matrix: target-independent release IR compiled
        // with `clang --target`, driver object emission for the matrix, and
        // explicit rejection of cross-target executables/libraries.
        "cross-target" => Some(&[("stc-driver", "cross_target")]),
        _ => None,
    }
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
    let targets = suite_targets(&suite).ok_or_else(|| {
        format!(
            "unknown suite `{suite}` (expected `ui`, `conformance`, `execution`, `projects`, \
             `debug`, or `cross-target`)"
        )
    })?;
    for (package, test) in targets {
        let mut command = Command::new(cargo());
        command
            .current_dir(workspace_root())
            .args(["test", "-p", package, "--test", test]);
        if locked {
            command.arg("--locked");
        }
        if !passthrough.is_empty() {
            command.arg("--").args(&passthrough);
        }
        eprintln!("xtask: cargo test -p {package} --test {test}");
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

/// Files that may contain operating-system conditionals.
///
/// Every entry is a workspace-relative path and a reason. The portable
/// compiler crates and the module/manifest model may only name their single
/// host module; the runtime and driver list every platform-layer file (D-201).
/// The audit also fails on a stale entry so the list cannot drift.
const HOST_AUDIT_ALLOWLIST: &str = "tools/ci/host-allowlist.txt";

/// Crates whose implementation must not branch on the host operating system.
const PORTABLE_CRATES: [&str; 6] = [
    "stc-lexer",
    "stc-parser",
    "stc-sema",
    "stc-codegen",
    "stc-got",
    "stc-manifest",
];

/// Crates that own a platform layer; branches are allowed only in listed files.
const HOST_LAYER_CRATES: [&str; 3] = ["stc-runtime", "stc-driver", "xtask"];

/// One source line that names a host-specific condition.
#[derive(Debug)]
struct HostHit {
    line: usize,
    pattern: String,
}

/// Checks that host-specific code stays inside the allowlisted files.
fn host_audit(args: &[String]) -> Result<(), String> {
    for argument in args {
        match argument.as_str() {
            "--check" => {}
            other => {
                return Err(format!("unknown `cargo xtask host-audit` option `{other}`"));
            }
        }
    }
    let root = workspace_root();
    let allowlist = load_host_allowlist(&root.join(HOST_AUDIT_ALLOWLIST))?;
    let mut hits: BTreeMap<String, Vec<HostHit>> = BTreeMap::new();
    for name in PORTABLE_CRATES.into_iter().chain(HOST_LAYER_CRATES) {
        let directory = root.join("crates").join(name).join("src");
        if !directory.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_rust_files(&directory, &mut files)?;
        for path in files {
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| format!("`{}` is outside the workspace", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
            let file_hits = host_hits(&text);
            if !file_hits.is_empty() {
                hits.insert(relative, file_hits);
            }
        }
    }
    let mut findings = Vec::new();
    for (path, file_hits) in &hits {
        if !allowlist.contains_key(path) {
            for hit in file_hits {
                findings.push(format!(
                    "{path}:{}: `{}` is not allowlisted",
                    hit.line, hit.pattern
                ));
            }
        }
    }
    for path in allowlist.keys() {
        if !hits.contains_key(path) {
            findings.push(format!(
                "{HOST_AUDIT_ALLOWLIST}: `{path}` has no host-specific line; remove the stale entry"
            ));
        }
    }
    if !findings.is_empty() {
        for finding in &findings {
            eprintln!("host-audit: {finding}");
        }
        return Err(format!(
            "{} host-specific source line(s) are outside the allowlist",
            findings.len()
        ));
    }
    let lines: usize = hits.values().map(Vec::len).sum();
    println!(
        "host-audit: {lines} host-specific line(s) in {} allowlisted file(s)",
        hits.len()
    );
    Ok(())
}

fn load_host_allowlist(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let mut entries = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((entry, reason)) = line.split_once('|') else {
            return Err(format!(
                "{}:{}: expected `<path> | <reason>`",
                path.display(),
                index + 1
            ));
        };
        let entry = entry.trim().replace('\\', "/");
        let reason = reason.trim();
        if entry.is_empty() || reason.is_empty() {
            return Err(format!(
                "{}:{}: the path and reason must be non-empty",
                path.display(),
                index + 1
            ));
        }
        entries.insert(entry, reason.to_owned());
    }
    Ok(entries)
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Returns the host-specific lines of one source file.
///
/// A line is a hit when it starts a `cfg` predicate naming an operating
/// system or architecture (including a multi-line predicate), or names a
/// platform module directly. Comment-only lines are ignored.
fn host_hits(text: &str) -> Vec<HostHit> {
    let mut hits = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some(start) = cfg_start(line) {
            // `cfg` predicates can span lines; read until the parentheses
            // balance so `#[cfg(all(\n target_os = ...))]` is not missed.
            let mut block = line[start..].to_owned();
            let mut depth = paren_balance(&block);
            while depth > 0 {
                let Some((_, next)) = lines.next() else {
                    break;
                };
                block.push('\n');
                block.push_str(next);
                depth += paren_balance(next);
            }
            if let Some(predicate) = ["windows", "unix", "target_os", "target_arch"]
                .into_iter()
                .find(|predicate| block.contains(predicate))
            {
                hits.push(HostHit {
                    line: index + 1,
                    pattern: predicate.to_owned(),
                });
            }
            continue;
        }
        if let Some(path) = ["std::os::windows", "std::os::unix", "std::env::consts"]
            .into_iter()
            .find(|path| line.contains(path))
        {
            hits.push(HostHit {
                line: index + 1,
                pattern: path.to_owned(),
            });
        }
    }
    hits
}

fn cfg_start(line: &str) -> Option<usize> {
    line.find("cfg(").or_else(|| line.find("cfg!("))
}

fn paren_balance(text: &str) -> i32 {
    text.chars().fold(0, |depth, character| match character {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    })
}

/// Names of the documentation files shipped in every release package.
const PACKAGE_DOCS: [&str; 5] = [
    "README.md",
    "USER-GUIDE.md",
    "RELEASE-NOTES.md",
    "KNOWN-LIMITATIONS.md",
    "NON-SAFETY.md",
];

/// Builds and verifies a release package (D-189–D-191).
///
/// The package is `stc-<version>-<host>/{bin,lib,docs}` plus `package.json`
/// and a sorted `SHA256SUMS`. The build is always `--locked`. Verification
/// rejects a runtime archive or shared library that still contains debug
/// sections, and checks that the packaged driver reports the expected
/// version.
fn package(args: &[String]) -> Result<(), String> {
    let mut out = PathBuf::from("dist");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                out = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "`--out` needs a directory".to_owned())?,
                );
            }
            other if other.starts_with("--out=") => {
                out = PathBuf::from(&other["--out=".len()..]);
            }
            // The build is always `--locked`; accept the flag for CI symmetry.
            "--locked" => {}
            other => return Err(format!("unknown `cargo xtask package` option `{other}`")),
        }
        index += 1;
    }

    let root = workspace_root();
    let version = env!("CARGO_PKG_VERSION");
    let host = host_triple()?;
    let package_name = format!("stc-{version}-{host}");
    let out = if out.is_absolute() {
        out
    } else {
        root.join(out)
    };
    let package_dir = out.join(&package_name);

    eprintln!("xtask: building stc and the runtime for {host}");
    run_checked(
        &cargo(),
        &[
            "build",
            "--release",
            "--locked",
            "-p",
            "stc-driver",
            "-p",
            "stc-runtime",
        ],
        &root,
    )?;

    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)
            .map_err(|error| format!("cannot clear `{}`: {error}", package_dir.display()))?;
    }
    let bin_dir = package_dir.join("bin");
    let lib_dir = package_dir.join("lib");
    let docs_dir = package_dir.join("docs");
    for directory in [&bin_dir, &lib_dir, &docs_dir] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
    }

    let release_dir = root.join("target/release");
    let driver_name = if cfg!(windows) { "stc.exe" } else { "stc" };
    let driver = release_dir.join(driver_name);
    copy_file(&driver, &bin_dir.join(driver_name))?;
    for name in runtime_names() {
        copy_file(&release_dir.join(name), &lib_dir.join(name))?;
    }
    for name in PACKAGE_DOCS {
        let source = root.join(name);
        if !source.is_file() {
            return Err(format!(
                "release document `{name}` is missing; M7 phase 5 must publish it"
            ));
        }
        copy_file(&source, &docs_dir.join(name))?;
    }
    strip_runtime_archive(&root, &lib_dir)?;

    let artifacts = package_files(&package_dir)?;
    let package_json = out.join(&package_name).join("package.json");
    fs::write(
        &package_json,
        render_package_json(version, &host, &artifacts),
    )
    .map_err(|error| format!("cannot write `{}`: {error}", package_json.display()))?;
    let artifacts = package_files(&package_dir)?;
    write_checksums(&package_dir, &artifacts)?;
    verify_package(&root, &package_dir, &host, version)?;

    println!("package: {}", package_dir.display());
    for artifact in &artifacts {
        println!("  {artifact}");
    }
    println!("  SHA256SUMS");
    Ok(())
}

fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .args(["-vV"])
        .output()
        .map_err(|error| format!("cannot run rustc: {error}"))?;
    if !output.status.success() {
        return Err("`rustc -vV` failed".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "`rustc -vV` did not report a host triple".into())
}

fn runtime_names() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["stc_runtime.lib", "stc_runtime.dll"]
    } else if cfg!(target_os = "macos") {
        vec!["libstc_runtime.a", "libstc_runtime.dylib"]
    } else {
        vec!["libstc_runtime.a", "libstc_runtime.so"]
    }
}

fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::copy(source, target).map(|_| ()).map_err(|error| {
        format!(
            "cannot copy `{}` to `{}`: {error}",
            source.display(),
            target.display()
        )
    })
}

/// Strips debug information from the copied runtime archive and shared
/// library (D-191). Rust standard-library archive members carry CodeView or
/// DWARF even in a release build, so the archive is unpacked, every object is
/// stripped, and the stripped objects replace the originals in place.
fn strip_runtime_archive(root: &Path, lib_dir: &Path) -> Result<(), String> {
    let archive = lib_dir.join(runtime_names()[0]);
    let ar = find_tool(&["llvm-ar", "ar"])?;
    let objcopy = find_tool(&["llvm-objcopy", "objcopy"])?;
    let directory = root.join("target/stc/package-strip");
    if directory.exists() {
        fs::remove_dir_all(&directory)
            .map_err(|error| format!("cannot clear `{}`: {error}", directory.display()))?;
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
    run_checked(&ar, &["x", &archive.to_string_lossy()], &directory)?;
    let mut candidates = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_object = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("o") || extension.eq_ignore_ascii_case("obj")
            });
        if !is_object || !is_native_object(&path) {
            // Import-library members, LLVM bitcode, and other non-object
            // members carry no native sections and are not rewritten.
            continue;
        }
        run_checked(
            &objcopy,
            &["--strip-debug", &path.to_string_lossy()],
            &directory,
        )?;
        candidates.push(path);
    }
    let objects: Vec<String> = candidates
        .into_iter()
        .filter(|path| is_native_object(path))
        .map(|path| {
            path.file_name()
                .ok_or_else(|| format!("archive member `{}` has no name", path.display()))
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect::<Result<_, _>>()?;
    if objects.is_empty() {
        return Err(format!(
            "runtime archive `{}` has no native objects",
            archive.display()
        ));
    }
    // Replace only object members; `r` keeps the duplicate import-library
    // members and their symbol-table entries that a fresh archive would lose.
    // On Windows the member list goes through a response file to stay below
    // the command-line limit; Unix passes the names directly.
    if cfg!(windows) {
        let mut response = String::new();
        response.push_str(&archive.to_string_lossy());
        response.push('\n');
        for object in &objects {
            response.push_str(object);
            response.push('\n');
        }
        fs::write(directory.join("objects.txt"), response)
            .map_err(|error| format!("cannot write member list: {error}"))?;
        run_checked(&ar, &["rD", "@objects.txt"], &directory)?;
    } else {
        let mut args = vec!["rD".to_owned(), archive.to_string_lossy().into_owned()];
        args.extend(objects.iter().cloned());
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        run_checked(&ar, &borrowed, &directory)?;
    }
    for name in &runtime_names()[1..] {
        let library = lib_dir.join(name);
        run_checked(
            &objcopy,
            &["--strip-debug", &library.to_string_lossy()],
            lib_dir,
        )?;
    }
    let _ = fs::remove_dir_all(&directory);
    Ok(())
}

/// Lists package-relative paths (`/`-separated) for every file except
/// `SHA256SUMS`, sorted.
fn package_files(package_dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_package_files(package_dir, package_dir, &mut files)?;
    files.retain(|path| path != "SHA256SUMS");
    files.sort();
    Ok(files)
}

fn collect_package_files(
    root: &Path,
    directory: &Path,
    out: &mut Vec<String>,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_package_files(root, &path, out)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push(relative);
        }
    }
    Ok(())
}

fn render_package_json(version: &str, host: &str, artifacts: &[String]) -> String {
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str("  \"artifacts\": [\n");
    for (index, artifact) in artifacts.iter().enumerate() {
        let comma = if index + 1 == artifacts.len() {
            ""
        } else {
            ","
        };
        json.push_str(&format!("    \"{artifact}\"{comma}\n"));
    }
    json.push_str("  ],\n");
    // The debug backend is host-visible metadata (D-206): Windows MSVC emits
    // object-level CodeView and Linux GNU emits DWARF.
    let debug_contract = if cfg!(windows) {
        "object-codeview"
    } else {
        "dwarf"
    };
    json.push_str(&format!("  \"debug_contract\": \"{debug_contract}\",\n"));
    json.push_str("  \"formats\": {\n");
    json.push_str(&format!("    \"catalog\": {},\n", stc_got::MANIFEST_FORMAT));
    json.push_str("    \"debug_object\": 1,\n");
    json.push_str(&format!(
        "    \"descriptor\": {},\n",
        stc_got::DESCRIPTOR_FORMAT
    ));
    json.push_str("    \"frame_sidecar\": 1,\n");
    json.push_str(&format!(
        "    \"manifest\": {},\n",
        stc_got::MANIFEST_FORMAT
    ));
    json.push_str("    \"metadata_sidecar\": 1,\n");
    json.push_str(&format!("    \"module\": {},\n", stc_got::MODULE_FORMAT));
    json.push_str("    \"state\": 1\n");
    json.push_str("  },\n");
    json.push_str(&format!("  \"host\": \"{host}\",\n"));
    json.push_str("  \"host_abi\": {\n");
    json.push_str(&format!("    \"major\": {},\n", stc_got::HOST_ABI_MAJOR));
    json.push_str(&format!("    \"minor\": {}\n", stc_got::HOST_ABI_MINOR));
    json.push_str("  },\n");
    json.push_str("  \"non_safety\": \"not-a-safety-plc\",\n");
    json.push_str("  \"profile\": \"stc-2013-oop-utf8-v1\",\n");
    json.push_str("  \"schema\": 1,\n");
    json.push_str(&format!("  \"version\": \"{version}\"\n"));
    json.push_str("}\n");
    json
}

fn write_checksums(package_dir: &Path, artifacts: &[String]) -> Result<(), String> {
    let mut sums = String::new();
    for artifact in artifacts {
        let path = package_dir.join(artifact);
        let bytes = fs::read(&path)
            .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
        let hex = stc_got::sha256::sha256(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        sums.push_str(&format!("{hex}  {artifact}\n"));
    }
    let path = package_dir.join("SHA256SUMS");
    fs::write(&path, sums).map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

fn verify_package(
    root: &Path,
    package_dir: &Path,
    host: &str,
    version: &str,
) -> Result<(), String> {
    for (path, expected) in [
        (
            package_dir
                .join("bin")
                .join(if cfg!(windows) { "stc.exe" } else { "stc" }),
            version,
        ),
        (package_dir.join("lib").join(runtime_names()[0]), ""),
    ] {
        if !path.is_file() {
            return Err(format!("package artifact `{}` is missing", path.display()));
        }
        if !expected.is_empty() {
            let output = Command::new(&path)
                .arg("--version")
                .output()
                .map_err(|error| format!("cannot run `{}`: {error}", path.display()))?;
            let text = String::from_utf8_lossy(&output.stdout);
            if !output.status.success() || text.trim() != format!("stc {version}") {
                return Err(format!(
                    "packaged driver reports `{}`, expected `stc {version}`",
                    text.trim()
                ));
            }
        }
    }
    if !host.is_empty()
        && !package_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(host))
    {
        return Err("package directory does not end with the host triple".into());
    }

    let archive = package_dir.join("lib").join(runtime_names()[0]);
    let check_dir = root.join("target/stc/package-check");
    if check_dir.exists() {
        fs::remove_dir_all(&check_dir)
            .map_err(|error| format!("cannot clear `{}`: {error}", check_dir.display()))?;
    }
    fs::create_dir_all(&check_dir)
        .map_err(|error| format!("cannot create `{}`: {error}", check_dir.display()))?;
    let ar = find_tool(&["llvm-ar", "ar"])?;
    run_checked(&ar, &["x", &archive.to_string_lossy()], &check_dir)?;
    let mut objects = Vec::new();
    collect_package_files(&check_dir, &check_dir, &mut objects)?;
    if objects.is_empty() {
        return Err(format!(
            "runtime archive `{}` extracted no objects",
            archive.display()
        ));
    }
    for object in &objects {
        let path = check_dir.join(object);
        if !is_native_object(&path) {
            continue;
        }
        check_no_debug_sections(&path)?;
    }
    for name in &runtime_names()[1..] {
        check_no_debug_sections(&package_dir.join("lib").join(name))?;
    }
    Ok(())
}

fn find_tool(candidates: &[&str]) -> Result<String, String> {
    for candidate in candidates {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Ok((*candidate).to_owned());
        }
    }
    Err(format!(
        "none of the required tools ({}) was found; install the LLVM toolchain",
        candidates.join(", ")
    ))
}

/// Returns whether a file starts with a native object header (ELF, COFF, or
/// Mach-O). Other archive members are not rewritten or section-inspected.
fn is_native_object(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Some(magic) = bytes
        .get(..4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
    else {
        return false;
    };
    if magic == *b"\x7fELF" {
        return true;
    }
    if matches!(
        magic,
        [0xcf, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    ) {
        return true;
    }
    let machine = u16::from_le_bytes([magic[0], magic[1]]);
    matches!(
        machine,
        0x014c
            | 0x0162
            | 0x0166
            | 0x0168
            | 0x01c0
            | 0x01c2
            | 0x01c4
            | 0x01f0
            | 0x01f1
            | 0x0200
            | 0x8664
            | 0xaa64
            | 0xa641
    )
}

fn check_no_debug_sections(file: &Path) -> Result<(), String> {
    let mut text = None;
    let readers: [(&str, &[&str]); 3] = [
        ("llvm-readobj", &["--sections"]),
        ("llvm-objdump", &["--section-headers"]),
        ("readelf", &["-S", "-W"]),
    ];
    for (program, args) in readers {
        let Ok(output) = Command::new(program).args(args).arg(file).output() else {
            continue;
        };
        if output.status.success() {
            text = Some(String::from_utf8_lossy(&output.stdout).into_owned());
            break;
        }
    }
    let text = text.ok_or_else(|| {
        format!(
            "no section reader (llvm-readobj, llvm-objdump, readelf) can inspect `{}`",
            file.display()
        )
    })?;
    let debug: BTreeSet<&str> = text
        .split(|character: char| {
            character.is_whitespace() || matches!(character, ',' | ']' | '[' | '"' | '\'')
        })
        .filter(|token| token.starts_with(".debug") && *token != ".debug_gdb_scripts")
        .collect();
    if !debug.is_empty() {
        return Err(format!(
            "release artifact `{}` contains debug sections: {}",
            file.display(),
            debug.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

fn run_checked(program: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("cannot run `{program}`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program} {}` failed with status {}:\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_audit_flags_operating_system_predicates() {
        let text = "\
#[cfg(windows)]
fn a() {}
#[cfg(all(target_arch = \"aarch64\", target_os = \"linux\"))]
fn b() {}
use std::os::unix::fs::PermissionsExt;
// cfg(windows) in a comment is not a branch
";
        let hits = host_hits(text);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 3);
        assert_eq!(hits[2].line, 5);
    }

    #[test]
    fn host_audit_reads_multiline_predicates() {
        let text = "\
#[cfg(not(any(
    target_arch = \"x86_64\",
    all(target_arch = \"aarch64\", target_os = \"linux\")
)))]
fn a() {}
";
        let hits = host_hits(text);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn host_audit_ignores_unrelated_cfg_and_ports() {
        let text = "\
#[cfg(test)]
mod tests {}
#[cfg(feature = \"x\")]
fn c() {}
let ports: u16 = 80;
";
        assert!(host_hits(text).is_empty());
    }

    #[test]
    fn package_json_names_the_host_debug_contract() {
        let json = render_package_json("1.0.0", "host-triple", &["bin/stc".into()]);
        let expected = if cfg!(windows) {
            "\"debug_contract\": \"object-codeview\""
        } else {
            "\"debug_contract\": \"dwarf\""
        };
        assert!(json.contains(expected), "{json}");
        assert!(json.contains("\"host\": \"host-triple\""), "{json}");
    }
}
