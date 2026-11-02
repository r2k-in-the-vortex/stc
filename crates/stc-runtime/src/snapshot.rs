//! Test-only global snapshot surface for the P0.6 differential harness.
//!
//! The launcher's opt-in `--snapshot <path>` writes one deterministic JSON
//! document after the run loop finishes. It reads every root in the active
//! manifest, in manifest order, through [`Host::debug_read`] and records the
//! resolved key, identity, canonical type, root kind, retention, and value,
//! together with the active generation's build and completed cycles. The
//! manifest includes global roots, program-instance fields, task guards,
//! `AT` direct variables, and hidden object-handle/record roots; consumers
//! select the roots they need by key.
//!
//! The document is:
//!
//! ```json
//! {
//!   "format": "stc-runtime-snapshot",
//!   "formatVersion": "0.1.0",
//!   "generation": 1,
//!   "build": "0x...",
//!   "cycles": 1,
//!   "roots": [
//!     {
//!       "key": "_ROOT::G",
//!       "gotId": "0x...",
//!       "rootKind": "global",
//!       "type": "DINT",
//!       "retention": "none",
//!       "protected": false,
//!       "atomic": true,
//!       "consistent": true,
//!       "value": "105"
//!     }
//!   ]
//! }
//! ```
//!
//! # Test-only restriction
//!
//! `--snapshot` is a differential-testing surface, not a production debug
//! service. [`Host::debug_read`] is the synchronous embedding/test read path:
//! it reads protected data (`RETAIN` roots and `AT` direct variables) without
//! the audited `observe` transaction the debug protocol requires. The snapshot
//! therefore includes protected roots, marks each one with `protected: true`,
//! and is only reachable by passing the flag to a compiled artifact. It never
//! writes, forces, or retargets a root, and it does not change the run loop's
//! production semantics.
//!
//! Manifest order is the compiler's canonical root order, so equal runs over
//! equal artifacts produce byte-identical output. A root whose read
//! fails, or a host without an active generation, fails the snapshot instead
//! of emitting a partial document.

use std::path::Path;

use stc_got::Json;
use stc_got::digest::GotId;
use stc_got::model::RootRecord;

use crate::debug::{DebugRead, DebugReadRequest, DebugTarget, debug_value_json};
use crate::host::Host;

/// The snapshot format family.
pub(crate) const SNAPSHOT_FORMAT: &str = "stc-runtime-snapshot";

/// The exact snapshot version emitted by this runtime.
pub(crate) const SNAPSHOT_FORMAT_VERSION: &str = "0.1.0";

/// One manifest root paired with its completed read.
struct ObservedRoot<'a> {
    root: &'a RootRecord,
    read: DebugRead,
}

/// Writes the active generation's global snapshot to `path`.
pub(crate) fn write_snapshot(path: &Path, host: &Host) -> Result<(), String> {
    let document = snapshot_json(host)?;
    write_document(path, &document)
}

/// Writes one rendered document, without creating missing directories.
fn write_document(path: &Path, document: &Json) -> Result<(), String> {
    std::fs::write(path, document.render())
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

/// Builds the deterministic snapshot document for the active generation.
pub(crate) fn snapshot_json(host: &Host) -> Result<Json, String> {
    let manifest = host
        .active_manifest()
        .ok_or_else(|| "no active generation is available for a snapshot".to_owned())?;
    let generation = host
        .active_generation()
        .ok_or_else(|| "no active generation is available for a snapshot".to_owned())?;
    let status = host.status();
    let build = status
        .active_build_id
        .ok_or_else(|| "no active build is available for a snapshot".to_owned())?;
    let mut observed = Vec::with_capacity(manifest.roots.len());
    for root in &manifest.roots {
        let read = host
            .debug_read(DebugReadRequest::new(DebugTarget::got_id(root.got_id)))
            .map_err(|error| {
                format!(
                    "root `{}` could not be read for the snapshot: {}: {}",
                    root.key, error.code, error.message
                )
            })?;
        observed.push(ObservedRoot { root, read });
    }
    Ok(snapshot_document(
        generation,
        build,
        status.cycles,
        &observed,
    ))
}

/// Assembles the snapshot document from already-completed root reads.
///
/// The roots stay in their input order; the caller supplies manifest order.
fn snapshot_document(
    generation: u64,
    build: GotId,
    cycles: u64,
    roots: &[ObservedRoot<'_>],
) -> Json {
    Json::object([
        ("format".into(), Json::string(SNAPSHOT_FORMAT)),
        (
            "formatVersion".into(),
            Json::string(SNAPSHOT_FORMAT_VERSION),
        ),
        ("generation".into(), Json::unsigned(generation)),
        ("build".into(), Json::string(build.to_hex())),
        ("cycles".into(), Json::unsigned(cycles)),
        (
            "roots".into(),
            Json::Array(roots.iter().map(root_json).collect()),
        ),
    ])
}

/// Encodes one root and its completed read.
fn root_json(observed: &ObservedRoot<'_>) -> Json {
    let root = observed.root;
    let read = &observed.read;
    Json::object([
        ("key".into(), Json::string(read.resolved_key.clone())),
        ("gotId".into(), Json::string(root.got_id.to_hex())),
        (
            "rootKind".into(),
            Json::string(root.root_kind.name().to_owned()),
        ),
        ("type".into(), Json::string(read.type_key.clone())),
        (
            "retention".into(),
            Json::string(root.retention.name().to_owned()),
        ),
        ("protected".into(), Json::Bool(read.protected)),
        ("atomic".into(), Json::Bool(read.atomic)),
        ("consistent".into(), Json::Bool(read.consistent)),
        ("value".into(), debug_value_json(&read.value)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::Json;
    use stc_got::digest::GotId;
    use stc_got::model::{InitializerKind, Retention, RootKind, RootRecord};

    use crate::debug::{DebugRead, DebugTarget, DebugValue};

    fn got_id(byte: u8) -> GotId {
        GotId::from_bytes([byte; 16])
    }

    fn root(key: &str, id: u8, kind: RootKind, retention: Retention) -> RootRecord {
        RootRecord {
            got_id: got_id(id),
            key: key.to_owned(),
            previous_keys: Vec::new(),
            root_kind: kind,
            owner_type: None,
            field_path: String::new(),
            type_key: "DINT".into(),
            layout_hash: got_id(0xf0),
            size: 4,
            align: 4,
            retention,
            initializer: InitializerKind::Zero,
            initializer_digest: None,
            source: None,
            slot: 0,
        }
    }

    fn read(key: &str, id: u8, value: &str, protected: bool) -> DebugRead {
        DebugRead {
            target: DebugTarget::got_id(got_id(id)),
            resolved_key: key.to_owned(),
            generation: 1,
            build_id: got_id(0xaa),
            cycle: 2,
            atomic: true,
            consistent: true,
            protected,
            type_key: "DINT".into(),
            value: DebugValue::Decimal(value.to_owned()),
            view: None,
        }
    }

    fn observed<'a>(root: &'a RootRecord, read: DebugRead) -> ObservedRoot<'a> {
        ObservedRoot { root, read }
    }

    #[test]
    fn empty_roots_render_an_empty_array() {
        let document = snapshot_document(1, got_id(0xaa), 0, &[]);
        let text = document.render();
        assert!(
            text.contains("\"format\": \"stc-runtime-snapshot\""),
            "{text}"
        );
        assert!(text.contains("\"formatVersion\": \"0.1.0\""), "{text}");
        assert!(text.contains("\"generation\": 1"), "{text}");
        assert!(text.contains("\"cycles\": 0"), "{text}");
        assert!(text.contains("\"roots\": []"), "{text}");
    }

    #[test]
    fn roots_stay_in_manifest_order() {
        let first = root("_ROOT::ZULU", 1, RootKind::Global, Retention::None);
        let second = root("_ROOT::ALPHA", 2, RootKind::Global, Retention::None);
        let observed = [
            observed(&first, read("_ROOT::ZULU", 1, "7", false)),
            observed(&second, read("_ROOT::ALPHA", 2, "-2", false)),
        ];
        let document = snapshot_document(1, got_id(0xaa), 1, &observed);
        let roots = document
            .as_object()
            .and_then(|object| object.get("roots"))
            .and_then(Json::as_array)
            .expect("roots array");
        let keys: Vec<&str> = roots
            .iter()
            .map(|root| {
                root.as_object()
                    .and_then(|object| object.get("key"))
                    .and_then(Json::as_str)
                    .expect("root key")
            })
            .collect();
        assert_eq!(keys, ["_ROOT::ZULU", "_ROOT::ALPHA"]);
    }

    #[test]
    fn output_is_deterministic_and_records_each_field() {
        let root = root("_ROOT::G", 3, RootKind::Global, Retention::None);
        let observed = [observed(&root, read("_ROOT::G", 3, "105", false))];
        let first = snapshot_document(1, got_id(0xaa), 2, &observed).render();
        let second = snapshot_document(1, got_id(0xaa), 2, &observed).render();
        assert_eq!(first, second);
        for fragment in [
            "\"key\": \"_ROOT::G\"",
            "\"gotId\": \"0x03030303030303030303030303030303\"",
            "\"rootKind\": \"global\"",
            "\"type\": \"DINT\"",
            "\"retention\": \"none\"",
            "\"protected\": false",
            "\"atomic\": true",
            "\"consistent\": true",
            "\"value\": \"105\"",
            "\"build\": \"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
        ] {
            assert!(first.contains(fragment), "missing {fragment}:\n{first}");
        }
    }

    #[test]
    fn protected_roots_are_marked_and_keep_their_retention() {
        let root = root("_ROOT::SAVED", 4, RootKind::Global, Retention::Retain);
        let observed = [observed(&root, read("_ROOT::SAVED", 4, "5", true))];
        let text = snapshot_document(1, got_id(0xaa), 3, &observed).render();
        assert!(text.contains("\"retention\": \"retain\""), "{text}");
        assert!(text.contains("\"protected\": true"), "{text}");
    }

    #[test]
    fn a_host_without_an_active_generation_is_an_error() {
        let _state = crate::test_support::state_guard();
        let host = Host::new(crate::host_platform(), crate::ManifestStore::memory());
        let error = snapshot_json(&host).expect_err("no active generation");
        assert!(error.contains("no active generation"), "{error}");
    }

    #[test]
    fn an_unwritable_path_is_an_error() {
        let directory = std::env::temp_dir().join(format!(
            "stc-snapshot-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        let path = directory.join("snapshot.json");
        let error = write_document(&path, &Json::Null).expect_err("unwritable snapshot");
        assert!(error.contains("cannot write"), "{error}");
        assert!(!directory.exists(), "the missing directory was not created");
    }
}
