//! `stc metadata`: the human and JSON build summary (CLI-and-Manifest §1.6).
//!
//! The summary is derived from a code generation projection; it reports the
//! selected configuration, ABI, tasks, exports, retain/I/O settings, and the
//! artifact names. The complete sidecar remains `<name>.stc-meta.json`.

use stc_codegen::Json;
use stc_manifest::Manifest;

use crate::build::{self, Generated, Request};

/// Prints the metadata summary.
pub(crate) fn print(generated: &Generated, manifest: &Manifest, request: &Request, json: bool) {
    if json {
        println!("{}", summary(generated, manifest, request).render());
        return;
    }
    let metadata = &generated.output.metadata;
    println!("stc metadata");
    println!("  profile          {}", metadata.profile);
    println!("  abi version      {}", metadata.abi_version);
    println!("  artifact         {}", metadata.artifact);
    println!("  build id         {}", metadata.build_id);
    println!(
        "  configuration    {}",
        metadata.configuration.as_deref().unwrap_or("<none>")
    );
    println!("  target           {}", metadata.target);
    println!("  tasks            {}", metadata.tasks.len());
    for task in &metadata.tasks {
        let interval = task
            .interval_ns
            .map_or_else(|| "every scan".to_owned(), |value| format!("{value}ns"));
        let priority = task
            .priority
            .map_or_else(|| "none".to_owned(), |value| value.to_string());
        let single = if task.single == Some(true) {
            " single"
        } else {
            ""
        };
        println!(
            "    {} resource {} interval {} priority {}{single}",
            task.name, task.resource, interval, priority
        );
    }
    println!("  exports          {}", metadata.exports.len());
    for export in &metadata.exports {
        println!("    {} -> {}", export.symbol, export.function);
    }
    let state_file = state_file(manifest, metadata.artifact.as_str());
    println!(
        "  retain           {}, {}, on-mismatch {}",
        if manifest.retain.enabled {
            "enabled"
        } else {
            "disabled"
        },
        state_file,
        manifest.retain.on_mismatch.as_str()
    );
    // Project bindings are resolved into module metadata in phase 5; until
    // then count the declared bindings alongside the pragma bindings.
    let bindings = metadata.io.bindings.len() + manifest.io.bindings.len();
    println!(
        "  io               backend {}, {bindings} bindings, {} devices",
        manifest.io.backend.as_str(),
        manifest.io.devices.len()
    );
    println!("  artifacts        {}", request.emit.len());
    for kind in &request.emit {
        let file = build::artifact_file(
            *kind,
            &metadata.artifact,
            &generated.module_file,
            request.lib_type,
        );
        println!("    {:<8} {file}", build::emit_kind(*kind));
    }
}

fn summary(generated: &Generated, manifest: &Manifest, request: &Request) -> Json {
    let metadata = &generated.output.metadata;
    let state_file = state_file(manifest, metadata.artifact.as_str());
    let tasks = metadata
        .tasks
        .iter()
        .map(|task| {
            Json::object([
                ("name".into(), Json::String(task.name.clone())),
                ("resource".into(), Json::String(task.resource.clone())),
                (
                    "interval_ns".into(),
                    task.interval_ns.map_or(Json::Null, Json::Number),
                ),
                (
                    "priority".into(),
                    task.priority.map_or(Json::Null, Json::Number),
                ),
                ("single".into(), Json::Bool(task.single.unwrap_or(false))),
            ])
        })
        .collect();
    let exports = metadata
        .exports
        .iter()
        .map(|export| {
            Json::object([
                ("symbol".into(), Json::String(export.symbol.clone())),
                ("function".into(), Json::String(export.function.clone())),
            ])
        })
        .collect();
    let artifacts = request
        .emit
        .iter()
        .map(|kind| {
            Json::object([
                ("kind".into(), Json::String(build::emit_kind(*kind).into())),
                (
                    "file".into(),
                    Json::String(build::artifact_file(
                        *kind,
                        &metadata.artifact,
                        &generated.module_file,
                        request.lib_type,
                    )),
                ),
            ])
        })
        .collect();
    let bindings = metadata.io.bindings.len() + manifest.io.bindings.len();
    Json::object([
        ("profile".into(), Json::String(metadata.profile.into())),
        (
            "abi_version".into(),
            Json::Number(i128::from(metadata.abi_version)),
        ),
        ("artifact".into(), Json::String(metadata.artifact.clone())),
        ("build_id".into(), Json::String(metadata.build_id.clone())),
        (
            "configuration".into(),
            metadata
                .configuration
                .clone()
                .map_or(Json::Null, Json::String),
        ),
        ("target".into(), Json::String(metadata.target.clone())),
        ("tasks".into(), Json::Array(tasks)),
        ("exports".into(), Json::Array(exports)),
        (
            "retain".into(),
            Json::object([
                ("enabled".into(), Json::Bool(manifest.retain.enabled)),
                ("file".into(), Json::String(state_file)),
                (
                    "on_mismatch".into(),
                    Json::String(manifest.retain.on_mismatch.as_str().into()),
                ),
            ]),
        ),
        (
            "io".into(),
            Json::object([
                (
                    "backend".into(),
                    Json::String(manifest.io.backend.as_str().into()),
                ),
                (
                    "bindings".into(),
                    Json::Number(i128::try_from(bindings).unwrap_or(i128::MAX)),
                ),
                (
                    "devices".into(),
                    Json::Number(i128::try_from(manifest.io.devices.len()).unwrap_or(i128::MAX)),
                ),
            ]),
        ),
        ("artifacts".into(), Json::Array(artifacts)),
    ])
}

fn state_file(manifest: &Manifest, artifact: &str) -> String {
    manifest
        .retain
        .file
        .clone()
        .unwrap_or_else(|| format!("{artifact}.stcstate"))
}
