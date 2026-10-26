//! `stc online download|diff`: export and compare the runtime baseline
//! (Online Change §7.4, CLI §1.5).
//!
//! Download reads the committed manifest from the runtime store or a live
//! control host, validates it, and writes the canonical download envelope.
//! Diff compares a candidate manifest with the explicit baseline or the
//! store's active manifest through the runtime's manifest-only entry point;
//! it never executes a module.

use std::path::{Path, PathBuf};

use stc_diag::Severity;
use stc_got::{Json, Manifest, RootKind, SlotKind};
use stc_manifest::LoadOptions;
use stc_runtime::control::ControlClient;
use stc_runtime::store::{
    Deployment, Download, LayoutState, ManifestStore, RuntimeLayout, RuntimeLayoutEntry,
};

use crate::cli::OnlineOptions;
use crate::compat::CompatibilityReport;
use crate::report::{Log, Reporter};

/// Runs `stc online download`.
pub(crate) fn download(
    options: &OnlineOptions,
    manifest: Option<&str>,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let envelope = if let Some(endpoint) = &options.host {
        let Some(nonce) = control_nonce(options, reporter) else {
            return 2;
        };
        match live_download(endpoint, &nonce, reporter) {
            Ok(download) => download,
            Err(code) => return code,
        }
    } else {
        let Some(store_dir) = store_directory(options, manifest, locked, reporter, log) else {
            return 2;
        };
        match store_download(&store_dir, reporter) {
            Ok(download) => download,
            Err(code) => return code,
        }
    };

    if let Err(error) = write_envelope(options, &envelope, reporter, log) {
        reporter.plain("STC-E-BUILD-002", Severity::Error, &error);
        return 1;
    }
    0
}

/// Runs `stc online diff`.
pub(crate) fn diff(
    options: &OnlineOptions,
    candidate_path: &str,
    manifest: Option<&str>,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> u8 {
    let candidate = match read_manifest(Path::new(candidate_path)) {
        Ok(manifest) => manifest,
        Err(message) => {
            reporter.plain("STC-E-MANIFEST-001", Severity::Error, &message);
            return 2;
        }
    };

    let baseline = if let Some(path) = &options.baseline {
        match read_manifest(Path::new(path)) {
            Ok(manifest) => manifest,
            Err(message) => {
                reporter.plain("STC-E-MANIFEST-001", Severity::Error, &message);
                return 2;
            }
        }
    } else if let Some(endpoint) = &options.host {
        let Some(nonce) = control_nonce(options, reporter) else {
            return 2;
        };
        match live_download(endpoint, &nonce, reporter) {
            Ok(download) => download.manifest().clone(),
            Err(code) => return code,
        }
    } else {
        let Some(store_dir) = store_directory(options, manifest, locked, reporter, log) else {
            return 2;
        };
        let store = ManifestStore::open(&store_dir);
        if let Some(problem) = store.problem() {
            reporter.plain(
                "STC-E-ONLINE-009",
                Severity::Error,
                &format!(
                    "the runtime store `{}` is invalid: {problem}",
                    store_dir.display()
                ),
            );
            return 2;
        }
        let Some(active) = store.active() else {
            reporter.plain(
                "STC-E-ONLINE-009",
                Severity::Error,
                &format!(
                    "the runtime store `{}` has no active baseline",
                    store_dir.display()
                ),
            );
            return 2;
        };
        active.manifest.clone()
    };

    let comparison = match stc_runtime::compare_manifests(&candidate, Some(&baseline)) {
        Ok(comparison) => comparison,
        Err(error) => {
            reporter.plain(error.code, Severity::Error, &error.message);
            return 2;
        }
    };
    let report = CompatibilityReport::new(&candidate, Some(&baseline));
    report.print(reporter, log);
    if comparison.decision == Some(stc_runtime::PlanDecision::Online)
        && comparison.error.is_none()
        && report.is_compatible()
    {
        0
    } else {
        1
    }
}

fn control_nonce(options: &OnlineOptions, reporter: &mut Reporter) -> Option<String> {
    let nonce = options
        .nonce
        .clone()
        .or_else(|| std::env::var("STC_CONTROL_NONCE").ok())
        .filter(|nonce| !nonce.is_empty());
    if nonce.is_none() {
        reporter.plain(
            "STC-E-ONLINE-007",
            Severity::Error,
            "a live control request needs `--nonce <token>` or `STC_CONTROL_NONCE`",
        );
    }
    nonce
}

fn live_download(endpoint: &str, nonce: &str, reporter: &mut Reporter) -> Result<Download, u8> {
    let mut client = match ControlClient::connect(endpoint, std::time::Duration::from_secs(5)) {
        Ok(client) => client,
        Err(message) => {
            reporter.plain("STC-E-ONLINE-007", Severity::Error, &message);
            return Err(1);
        }
    };
    let response = match client.request(
        nonce,
        Json::object([("op".into(), Json::string("download"))]),
    ) {
        Ok(response) => response,
        Err(message) => {
            reporter.plain("STC-E-ONLINE-007", Severity::Error, &message);
            return Err(1);
        }
    };
    if !response_ok(&response) {
        reporter.plain(
            "STC-E-ONLINE-007",
            Severity::Error,
            &response_error(&response),
        );
        return Err(1);
    }
    let Some(text) = response
        .as_object()
        .and_then(|object| object.get("download"))
        .and_then(Json::as_str)
    else {
        reporter.plain(
            "STC-E-ONLINE-007",
            Severity::Error,
            "the control response has no download envelope",
        );
        return Err(1);
    };
    Download::from_json_text(text).map_err(|error| {
        reporter.plain(
            "STC-E-ONLINE-009",
            Severity::Error,
            &format!("the host download is invalid: {}", error.message),
        );
        1
    })
}

fn store_directory(
    options: &OnlineOptions,
    manifest: Option<&str>,
    locked: bool,
    reporter: &mut Reporter,
    log: Log,
) -> Option<PathBuf> {
    if let Some(store) = &options.store {
        return Some(PathBuf::from(store));
    }
    let manifest_input = manifest.unwrap_or("stc.toml");
    let graph = match stc_manifest::load_project_readonly(manifest_input, &LoadOptions { locked }) {
        Ok(graph) => graph,
        Err(error) => {
            reporter.diagnostics(&error.diagnostics, &error.sources);
            return None;
        }
    };
    let root = graph.root_package();
    reporter.merge_manifest_lints(&root.manifest.lints);
    reporter.diagnostics(&graph.warnings, &graph.sources);
    let Some(dir) = root.manifest.runtime.manifest_dir.as_deref() else {
        reporter.plain(
            "STC-E-ONLINE-009",
            Severity::Error,
            "no runtime store was selected; pass `--store <dir>` or set `[runtime].manifest-dir`",
        );
        return None;
    };
    log.info(format!("store {}", root.root_dir.join(dir).display()));
    Some(root.root_dir.join(dir))
}

fn store_download(store_dir: &Path, reporter: &mut Reporter) -> Result<Download, u8> {
    let store = ManifestStore::open(store_dir);
    if let Some(problem) = store.problem() {
        reporter.plain(
            "STC-E-ONLINE-009",
            Severity::Error,
            &format!(
                "the runtime store `{}` is invalid: {problem}",
                store_dir.display()
            ),
        );
        return Err(1);
    }
    let Some(active) = store.active() else {
        reporter.plain(
            "STC-E-ONLINE-009",
            Severity::Error,
            &format!(
                "the runtime store `{}` has no active baseline",
                store_dir.display()
            ),
        );
        return Err(1);
    };
    let deployment = Deployment {
        module: active.manifest.header.module_name.clone(),
        build_id: active.build_id,
        manifest_digest: active.digest,
        parent_build_id: active.manifest.header.parent_build_id,
    };
    let layout = store_layout(&active.manifest);
    store.download(layout, deployment).map_err(|error| {
        reporter.plain("STC-E-ONLINE-009", Severity::Error, &error.message);
        1
    })
}

/// Builds a value-free layout summary from the committed manifest.
fn store_layout(manifest: &Manifest) -> RuntimeLayout {
    let mut entries: Vec<RuntimeLayoutEntry> = manifest
        .roots
        .iter()
        .filter(|root| root.root_kind != RootKind::Direct)
        .map(|root| RuntimeLayoutEntry {
            got_id: root.got_id,
            key: root.key.clone(),
            kind: root.root_kind,
            state: LayoutState::Active,
            slot: manifest
                .slots
                .iter()
                .find(|slot| slot.got_id == root.got_id && slot.kind == SlotKind::Data)
                .map(|slot| slot.slot),
            size: root.size,
            generation: 0,
            storage: None,
            cause: None,
        })
        .collect();
    entries.sort_by_key(|entry| entry.got_id);
    RuntimeLayout { entries }
}

fn write_envelope(
    options: &OnlineOptions,
    download: &Download,
    reporter: &Reporter,
    log: Log,
) -> Result<(), String> {
    let text = download.to_json_text();
    if let Some(path) = &options.out {
        std::fs::write(path, &text).map_err(|error| format!("cannot write `{path}`: {error}"))?;
        log.status(format!(
            "downloaded build {} to {path}",
            download.deployment.build_id.to_hex()
        ));
    } else {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
    }
    if let Some(path) = &options.binary {
        std::fs::write(path, download.manifest_binary())
            .map_err(|error| format!("cannot write `{path}`: {error}"))?;
        log.status(format!("wrote binary manifest {path}"));
    }
    if !reporter.is_json() {
        log.status(format!(
            "deployment {} module {} digest {} ({} layout entries)",
            download.deployment.build_id.to_hex(),
            download.deployment.module,
            download.deployment.manifest_digest.to_hex(),
            download.layout.entries.len()
        ));
    }
    Ok(())
}

/// Reads a `.stc-got.json` or `.stc-got.bin` manifest and validates it.
pub(crate) fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let manifest = if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        let text =
            String::from_utf8(bytes).map_err(|_| format!("`{}` is not UTF-8", path.display()))?;
        Manifest::decode_json(&text)
    } else {
        Manifest::decode_binary(&bytes)
    }
    .map_err(|error| {
        format!(
            "`{}` is not a valid GOT manifest: {}",
            path.display(),
            error.message
        )
    })?;
    manifest.validate().map_err(|error| {
        format!(
            "`{}` is not a valid GOT manifest: {}",
            path.display(),
            error.message
        )
    })?;
    Ok(manifest)
}

fn response_ok(response: &Json) -> bool {
    response
        .as_object()
        .and_then(|object| object.get("ok"))
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

fn response_error(response: &Json) -> String {
    let Some(object) = response.as_object() else {
        return "the control response is not an object".into();
    };
    let code = object
        .get("code")
        .and_then(Json::as_str)
        .unwrap_or("STC-E-ONLINE-007");
    let message = object
        .get("message")
        .and_then(Json::as_str)
        .unwrap_or("the control request failed");
    format!("{code}: {message}")
}
