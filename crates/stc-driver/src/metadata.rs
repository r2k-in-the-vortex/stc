//! `stc metadata`: the human and JSON build summary (CLI-and-Manifest §1.6).
//!
//! The summary is derived from a code generation projection; it reports the
//! selected configuration, ABI, tasks, exports, retain/I/O settings, and the
//! artifact names. The complete sidecar remains `<name>.stc-meta.json`.

use stc_codegen::Json;

use crate::build::{self, Generated, Request};

/// Prints the metadata summary.
pub(crate) fn print(generated: &Generated, request: &Request, json: bool) {
    if json {
        println!("{}", summary(generated, request).render());
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
    println!(
        "  retain           {}, {}, on-mismatch {}",
        if metadata.retain.enabled {
            "enabled"
        } else {
            "disabled"
        },
        metadata.retain.file,
        metadata.retain.on_mismatch
    );
    println!(
        "  io               backend {}, {} bindings, {} devices, {} var-access",
        metadata.io.backend,
        metadata.io.bindings.len(),
        metadata.io.devices.len(),
        metadata.io.var_access.len()
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
    println!("  build mode       {}", build_mode(request));
    println!("  debug info       {}", debug_info(generated));
    println!("  debug hooks      {}", debug_hooks(generated));
}

fn build_mode(request: &Request) -> &'static str {
    if request.release {
        build::BuildMode::Release.as_str()
    } else {
        build::BuildMode::Debug.as_str()
    }
}

fn debug_info(generated: &Generated) -> &'static str {
    generated
        .output
        .metadata
        .debug
        .as_ref()
        .map_or("none", |debug| debug.info)
}

fn debug_hooks(generated: &Generated) -> bool {
    generated
        .output
        .metadata
        .debug
        .as_ref()
        .is_some_and(|debug| debug.hooks)
}

fn summary(generated: &Generated, request: &Request) -> Json {
    let metadata = &generated.output.metadata;
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
                ("enabled".into(), Json::Bool(metadata.retain.enabled)),
                ("file".into(), Json::String(metadata.retain.file.clone())),
                (
                    "on_mismatch".into(),
                    Json::String(metadata.retain.on_mismatch.clone()),
                ),
            ]),
        ),
        (
            "io".into(),
            Json::object([
                ("backend".into(), Json::String(metadata.io.backend.clone())),
                (
                    "bindings".into(),
                    Json::Number(i128::try_from(metadata.io.bindings.len()).unwrap_or(i128::MAX)),
                ),
                (
                    "devices".into(),
                    Json::Number(i128::try_from(metadata.io.devices.len()).unwrap_or(i128::MAX)),
                ),
                (
                    "var_access".into(),
                    Json::Number(i128::try_from(metadata.io.var_access.len()).unwrap_or(i128::MAX)),
                ),
            ]),
        ),
        ("artifacts".into(), Json::Array(artifacts)),
        (
            "build_mode".into(),
            Json::String(build_mode(request).into()),
        ),
        (
            "debug_info".into(),
            Json::String(debug_info(generated).into()),
        ),
        ("debug_hooks".into(), Json::Bool(debug_hooks(generated))),
    ])
}
