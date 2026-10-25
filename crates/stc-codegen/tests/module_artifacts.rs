//! Phase-4 module artifact tests: root catalog, canonical slots, descriptor,
//! manifest projection, online-change plan, and determinism.

mod common;

use stc_codegen::module::{FunctionTarget, ModuleArtifacts, ModuleOptions, RootTarget, project};
use stc_got::catalog::ImportService;
use stc_got::{Digest32, Endianness, PlanAction, PlanVerdict, Target};
use stc_hir::{DefId, DefKind};

const SOURCE: &str = r#"
VAR_GLOBAL RETAIN
  Counter : DINT := 3;
END_VAR
VAR_GLOBAL
  Limit : INT;
END_VAR

{stc:export}
FUNCTION Add : DINT
VAR_INPUT
  a : DINT;
  b : DINT;
END_VAR
  Add := a + b;
END_FUNCTION

PROGRAM Main
VAR
  input AT %IX1.2 : BOOL {stc:io {"device":"rack","channel":3,"direction":"input"}};
  output AT %QW1 : WORD;
  plain : DINT;
END_VAR
VAR RETAIN
  saved : DINT := 5;
END_VAR
  plain := Counter;
  output := 16#1234;
END_PROGRAM

CONFIGURATION Default
RESOURCE Cpu ON Cpu
  TASK Fast (SINGLE := TRUE);
  PROGRAM inst WITH Fast : Main;
END_RESOURCE
END_CONFIGURATION
"#;

const SIMPLE: &str = r#"
PROGRAM Main
VAR
  counter : DINT;
  flag : BOOL;
END_VAR
  counter := counter + 1;
  flag := counter > 3;
END_PROGRAM
"#;

fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

fn fingerprint(source: &str, name: &str) -> Digest32 {
    stc_codegen::module::source_fingerprint(&[(format!("{name}.st"), source.as_bytes().to_vec())])
        .expect("source fingerprint")
}

fn module_options<'a>(source: &str, name: &'a str) -> ModuleOptions<'a> {
    let mut options = ModuleOptions::new(target(), name, "stc 0.1.0", fingerprint(source, name));
    options.relaxed = false;
    options
}

fn project_source(source: &str, name: &str) -> ModuleArtifacts {
    let hir = common::clean_hir(source);
    let sources = vec![format!("{name}.st")];
    project(&hir, &sources, &module_options(source, name))
        .unwrap_or_else(|errors| panic!("module projection failed: {errors:#?}"))
}

fn find_global(hir: &stc_hir::HirCrate, name: &str) -> DefId {
    hir.defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            matches!(def.kind, DefKind::Global { .. })
                .then_some(DefId::new(u32::try_from(index).ok()?))
                .filter(|id| hir.def_name(*id).eq_ignore_ascii_case(name))
        })
        .expect("global")
}

#[test]
fn projects_the_m4_root_catalog_and_canonical_slots() {
    let artifacts = project_source(SOURCE, "catalog");
    let hir = common::clean_hir(SOURCE);
    let manifest = &artifacts.manifest;

    assert_eq!(artifacts.verdict(), PlanVerdict::Cold);
    assert!(artifacts.is_deployable());

    // Global data roots, per-instance program fields, direct fields, and one
    // task guard per task.
    let keys: Vec<&str> = manifest
        .roots
        .iter()
        .map(|root| root.key.as_str())
        .collect();
    assert!(keys.contains(&"_ROOT::COUNTER"), "{keys:?}");
    assert!(keys.contains(&"_ROOT::LIMIT"), "{keys:?}");
    assert!(
        keys.contains(&"_ROOT::MAIN.PLAIN#DEFAULT/CPU/INST"),
        "{keys:?}"
    );
    assert!(
        keys.contains(&"_ROOT::MAIN.SAVED#DEFAULT/CPU/INST"),
        "{keys:?}"
    );
    assert!(
        keys.contains(&"_ROOT::MAIN.INPUT#DEFAULT/CPU/INST"),
        "{keys:?}"
    );
    assert!(
        keys.contains(&"_ROOT::MAIN.OUTPUT#DEFAULT/CPU/INST"),
        "{keys:?}"
    );
    assert!(keys.contains(&"_ROOT::DEFAULT/CPU#FAST.GUARD"), "{keys:?}");

    let counter = manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .expect("counter root");
    assert_eq!(counter.type_key, "DINT");
    assert_eq!(counter.retention, stc_got::Retention::Retain);
    let saved = manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::MAIN.SAVED#DEFAULT/CPU/INST")
        .expect("saved root");
    assert_eq!(saved.retention, stc_got::Retention::Retain);
    let input = manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::MAIN.INPUT#DEFAULT/CPU/INST")
        .expect("direct input root");
    assert_eq!(input.root_kind, stc_got::RootKind::Direct);
    assert_eq!(input.initializer, stc_got::InitializerKind::None);

    // User function and program body.
    let function_keys: Vec<&str> = manifest
        .functions
        .iter()
        .map(|function| function.key.as_str())
        .collect();
    assert!(
        function_keys.contains(&"_ROOT::ADD@(IN:DINT,IN:DINT)->DINT"),
        "{function_keys:?}"
    );
    assert!(
        function_keys.contains(&"_ROOT::MAIN#BODY@()->VOID"),
        "{function_keys:?}"
    );

    // Instance, task, and export records.
    assert_eq!(manifest.instances.len(), 1);
    assert_eq!(manifest.instances[0].key, "_ROOT::DEFAULT/CPU/INST");
    assert_eq!(manifest.tasks.len(), 1);
    assert_eq!(manifest.tasks[0].key, "_ROOT::DEFAULT/CPU#FAST");
    assert_eq!(
        manifest.tasks[0].guard_got_id,
        Some(
            manifest
                .roots
                .iter()
                .find(|root| root.key == "_ROOT::DEFAULT/CPU#FAST.GUARD")
                .expect("guard root")
                .got_id
        )
    );
    assert_eq!(manifest.exports.len(), 1);
    assert_eq!(manifest.exports[0].c_name, "stc_export_ADD");

    // Slots are contiguous and grouped Data, Function, Descriptor, Direct,
    // Import (GOT Artifacts §6.2).
    assert_eq!(
        artifacts.slots.len(),
        manifest.slots.len(),
        "a binding per slot"
    );
    for (index, binding) in artifacts.slots.iter().enumerate() {
        assert_eq!(binding.slot as usize, index);
    }
    let kinds = artifacts.slot_kinds();
    let import_start = kinds
        .iter()
        .position(|kind| *kind == stc_codegen::ir::CellKind::Import)
        .expect("import slots");
    assert!(
        kinds[import_start..]
            .iter()
            .all(|kind| *kind == stc_codegen::ir::CellKind::Import)
    );
    assert_eq!(
        artifacts.import_slot(ImportService::Trap),
        Some(import_start as u32)
    );

    // Bindings resolve back to HIR declarations.
    let counter_id = find_global(&hir, "Counter");
    let counter_slot = artifacts
        .slots
        .iter()
        .find(|binding| binding.got_id == counter.got_id)
        .expect("counter slot")
        .slot;
    assert_eq!(
        artifacts.root_slot(&RootTarget::Global(counter_id)),
        Some(counter_slot)
    );
    let add = hir
        .defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            (def.export_name.is_some() && matches!(def.kind, DefKind::Function { .. }))
                .then_some(DefId::new(u32::try_from(index).ok()?))
        })
        .expect("Add");
    assert!(
        artifacts
            .function_slot(FunctionTarget::Function(add))
            .is_some()
    );
    let program = hir
        .defs
        .iter()
        .enumerate()
        .find_map(|(index, def)| {
            matches!(def.kind, DefKind::Program { .. })
                .then_some(DefId::new(u32::try_from(index).ok()?))
        })
        .expect("Main");
    assert!(
        artifacts
            .function_slot(FunctionTarget::ProgramBody(program))
            .is_some()
    );
}

#[test]
fn descriptor_and_manifest_are_a_valid_pair() {
    let artifacts = project_source(SOURCE, "pair");
    artifacts.descriptor.validate().expect("descriptor format");
    artifacts
        .descriptor
        .validate_against_manifest(&artifacts.manifest)
        .expect("descriptor/manifest pair");
    assert_eq!(
        artifacts.descriptor.digest().unwrap(),
        artifacts.descriptor_digest
    );
    assert_eq!(
        artifacts.descriptor.manifest_digest,
        artifacts.manifest_digest
    );
    assert_eq!(
        artifacts.manifest.header.descriptor_digest,
        artifacts.descriptor_digest
    );
    assert_eq!(
        artifacts.manifest.digest().unwrap(),
        artifacts.manifest_digest
    );

    // Binary and JSON round-trip to the same model.
    let binary = stc_got::Manifest::decode_binary(&artifacts.manifest_binary)
        .expect("binary manifest round-trip");
    assert_eq!(binary, artifacts.manifest);
    let json =
        stc_got::Manifest::decode_json(&artifacts.manifest_json).expect("JSON manifest round-trip");
    assert_eq!(json, artifacts.manifest);
    assert_eq!(
        json.encode_binary().expect("binary projection"),
        artifacts.manifest_binary,
        "JSON and binary projections must produce the same body"
    );
    let descriptor =
        stc_got::Descriptor::decode(&artifacts.descriptor_bytes).expect("descriptor round-trip");
    assert_eq!(descriptor, artifacts.descriptor);
}

#[test]
fn double_build_is_byte_identical() {
    let first = project_source(SOURCE, "determinism");
    let second = project_source(SOURCE, "determinism");
    assert_eq!(first.descriptor_bytes, second.descriptor_bytes);
    assert_eq!(first.manifest_binary, second.manifest_binary);
    assert_eq!(first.manifest_json, second.manifest_json);
    assert_eq!(
        first.manifest.header.build_id,
        second.manifest.header.build_id
    );
    assert_eq!(
        first.manifest.header.catalog_digest,
        second.manifest.header.catalog_digest
    );
    assert_eq!(
        first.manifest.header.plan_digest,
        second.manifest.header.plan_digest
    );
    assert_eq!(first.slots, second.slots);
}

#[test]
fn online_builds_with_the_same_baseline_are_byte_identical() {
    let first = project_source(SOURCE, "determinism_online");
    let hir = common::clean_hir(SOURCE);
    let sources = vec!["determinism_online.st".to_owned()];
    let mut options = module_options(SOURCE, "determinism_online");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("first online projection");
    let third = project(&hir, &sources, &options).expect("second online projection");
    assert_eq!(second.descriptor_bytes, third.descriptor_bytes);
    assert_eq!(second.manifest_binary, third.manifest_binary);
    assert_eq!(second.manifest_json, third.manifest_json);
    assert_eq!(second.manifest.plan, third.manifest.plan);
    assert_eq!(
        second.manifest.header.build_id,
        third.manifest.header.build_id
    );
}

#[test]
fn second_build_consumes_the_baseline_and_preserves_got_ids() {
    let first = project_source(SOURCE, "baseline");
    let hir = common::clean_hir(SOURCE);
    let sources = vec!["baseline.st".to_owned()];
    let mut options = module_options(SOURCE, "baseline");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("second projection");

    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    assert_eq!(
        second.manifest.header.parent_build_id,
        Some(first.manifest.header.build_id)
    );
    assert_eq!(
        second.manifest.header.parent_manifest_digest,
        Some(first.manifest_digest)
    );
    // Persistent identities are preserved; direct-variable identities are
    // always per-generation allocations (GOT Artifacts §10).
    assert!(
        second.manifest.plan.actions.iter().all(|action| {
            if action.key.contains("MAIN.INPUT") || action.key.contains("MAIN.OUTPUT") {
                action.action == PlanAction::Allocate
            } else {
                action.action == PlanAction::Preserve
            }
        }),
        "{:#?}",
        second.manifest.plan.actions
    );
    // Every active identity keeps its GotId.
    let baseline: std::collections::BTreeMap<_, _> = first
        .manifest
        .roots
        .iter()
        .map(|root| (root.key.clone(), root.got_id))
        .chain(
            first
                .manifest
                .functions
                .iter()
                .map(|function| (function.key.clone(), function.got_id)),
        )
        .collect();
    for root in &second.manifest.roots {
        assert_eq!(baseline.get(&root.key), Some(&root.got_id), "{}", root.key);
    }
    for function in &second.manifest.functions {
        assert_eq!(
            baseline.get(&function.key),
            Some(&function.got_id),
            "{}",
            function.key
        );
    }
}

#[test]
fn a_code_only_change_preserves_every_persistent_root() {
    let changed = SOURCE.replace("  Add := a + b;", "  Add := a + b + 1;");
    let first = project_source(SOURCE, "code_only");
    let hir = common::clean_hir(&changed);
    let sources = vec!["code_only.st".to_owned()];
    let mut options = module_options(&changed, "code_only");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("code-only projection");

    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    assert_ne!(
        second.manifest.header.build_id, first.manifest.header.build_id,
        "the code change produces a new build identity"
    );
    for action in &second.manifest.plan.actions {
        if action.key.contains("MAIN.INPUT") || action.key.contains("MAIN.OUTPUT") {
            assert_eq!(action.action, PlanAction::Allocate);
        } else {
            assert_eq!(
                action.action,
                PlanAction::Preserve,
                "code-only change must preserve `{}`",
                action.key
            );
        }
    }
}

#[test]
fn a_changed_initializer_is_ignored_on_a_preserved_root() {
    let changed = SOURCE.replace("Counter : DINT := 3;", "Counter : DINT := 4;");
    let first = project_source(SOURCE, "initializer");
    let hir = common::clean_hir(&changed);
    let sources = vec!["initializer.st".to_owned()];
    let mut options = module_options(&changed, "initializer");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("initializer projection");

    let counter = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.key == "_ROOT::COUNTER")
        .expect("counter action");
    assert_eq!(counter.action, PlanAction::Preserve);
    assert!(
        second
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == stc_got::codes::W_ONLINE_INITIALIZER),
        "{:#?}",
        second.diagnostics
    );
}

#[test]
fn a_new_global_is_allocated_and_existing_roots_are_preserved() {
    let changed = format!("VAR_GLOBAL\n  Extra : DINT;\nEND_VAR\n{SOURCE}");
    let first = project_source(SOURCE, "new_global");
    let hir = common::clean_hir(&changed);
    let sources = vec!["new_global.st".to_owned()];
    let mut options = module_options(&changed, "new_global");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("new-global projection");

    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    let extra = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.key == "_ROOT::EXTRA")
        .expect("new root action");
    assert_eq!(extra.action, PlanAction::Allocate);
    let counter = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.key == "_ROOT::COUNTER")
        .expect("preserved root action");
    assert_eq!(counter.action, PlanAction::Preserve);
}

#[test]
fn cold_and_stale_baselines_select_the_cold_path() {
    let first = project_source(SOURCE, "cold");
    let hir = common::clean_hir(SOURCE);
    let sources = vec!["cold.st".to_owned()];

    let mut options = module_options(SOURCE, "cold");
    options.baseline = Some(&first.manifest);
    options.cold = true;
    let cold = project(&hir, &sources, &options).expect("forced cold");
    assert_eq!(cold.verdict(), PlanVerdict::Cold);
    assert_eq!(cold.manifest.header.parent_build_id, None);
    assert!(
        cold.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == stc_got::codes::W_MANIFEST_COLD)
    );

    let mut options = module_options(SOURCE, "cold");
    options.target = Target::new("x86_64-pc-windows-msvc", Endianness::Little, 8);
    options.baseline = Some(&first.manifest);
    let stale = project(&hir, &sources, &options).expect("stale baseline");
    assert_eq!(stale.verdict(), PlanVerdict::Cold);
    assert_eq!(stale.manifest.header.parent_build_id, None);
    assert!(
        stale
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == stc_got::codes::W_ONLINE_STALE_BASELINE)
    );
}

#[test]
fn implicit_program_synthesizes_a_configuration_resource_and_task() {
    let artifacts = project_source(SIMPLE, "implicit");
    assert_eq!(artifacts.manifest.instances.len(), 1);
    assert_eq!(
        artifacts.manifest.instances[0].key,
        "_ROOT::_DEFAULT/_DEFAULT/_DEFAULT"
    );
    assert_eq!(artifacts.manifest.tasks.len(), 1);
    assert_eq!(
        artifacts.manifest.tasks[0].key,
        "_ROOT::_DEFAULT/_DEFAULT#_DEFAULT"
    );
    assert!(
        artifacts
            .manifest
            .roots
            .iter()
            .any(|root| root.key == "_ROOT::_DEFAULT/_DEFAULT#_DEFAULT.GUARD")
    );
    assert_eq!(
        artifacts.manifest.instances[0].task_key.as_deref(),
        Some("_ROOT::_DEFAULT/_DEFAULT#_DEFAULT")
    );
    // Program field roots use the synthesized instance path.
    assert!(
        artifacts
            .manifest
            .roots
            .iter()
            .any(|root| root.key == "_ROOT::MAIN.COUNTER#_DEFAULT/_DEFAULT/_DEFAULT")
    );
}

#[test]
fn source_locations_are_package_relative() {
    let artifacts = project_source(SIMPLE, "locations");
    let source = artifacts
        .manifest
        .roots
        .iter()
        .find_map(|root| root.source.as_ref())
        .expect("a root source location");
    assert_eq!(source.package, "_ROOT");
    assert_eq!(source.file, "locations.st");
    assert!(!source.file.contains('\\'));
    assert!(!source.file.contains(':'));
    assert!(!source.file.starts_with('/'));
}

#[test]
fn source_fingerprint_is_order_independent_and_package_scoped() {
    let first = stc_codegen::module::source_fingerprint(&[
        ("b.st".into(), b"PROGRAM B\n".to_vec()),
        ("a.st".into(), b"PROGRAM A\n".to_vec()),
    ])
    .expect("fingerprint");
    let second = stc_codegen::module::source_fingerprint(&[
        ("a.st".into(), b"PROGRAM A\n".to_vec()),
        ("b.st".into(), b"PROGRAM B\n".to_vec()),
    ])
    .expect("fingerprint");
    assert_eq!(first, second);
    let expected = stc_got::source_fingerprint(&[
        ("_ROOT/a.st".into(), b"PROGRAM A\n".to_vec()),
        ("_ROOT/b.st".into(), b"PROGRAM B\n".to_vec()),
    ])
    .expect("reference fingerprint");
    assert_eq!(first, expected);
}

#[test]
fn package_relative_paths_strip_drive_prefixes() {
    assert_eq!(
        stc_codegen::module::package_relative_paths(&[
            r"C:\projects\a\x.st".into(),
            r"D:\other\b\y.st".into(),
        ]),
        vec!["projects/a/x.st".to_owned(), "other/b/y.st".to_owned()]
    );
    assert_eq!(
        stc_codegen::module::package_relative_paths(&[r"C:relative.st".into()]),
        vec!["relative.st".to_owned()]
    );
    assert_eq!(
        stc_codegen::module::package_relative_paths(&[
            r"C:\projects\same.st".into(),
            r"C:\projects\same.st".into(),
        ]),
        vec!["same.st".to_owned(), "1_same.st".to_owned()]
    );
}

#[test]
fn tracked_rename_preserves_the_got_id() {
    let renamed = SOURCE.replace("Counter", "Ticks");
    let first = project_source(SOURCE, "rename");
    let hir = common::clean_hir(&renamed);
    let sources = vec!["rename.st".to_owned()];
    let mut options = module_options(&renamed, "rename");
    options.baseline = Some(&first.manifest);
    options.renames = vec![stc_got::Rename::new("_ROOT::COUNTER", "_ROOT::TICKS")];
    let second = project(&hir, &sources, &options).expect("renamed projection");
    let old = first
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .expect("counter");
    let new = second
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::TICKS")
        .expect("ticks");
    assert_eq!(old.got_id, new.got_id);
    assert_eq!(new.previous_keys, vec!["_ROOT::COUNTER".to_owned()]);
    assert!(second.manifest.plan.actions.iter().any(|action| {
        action.got_id == new.got_id
            && action.action == PlanAction::Preserve
            && action.old_key.as_deref() == Some("_ROOT::COUNTER")
    }));
}

#[test]
fn untracked_removal_requires_the_explicit_policy() {
    let removed = SOURCE.replace("  Limit : INT;\n", "");
    let first = project_source(SOURCE, "removal");
    let hir = common::clean_hir(&removed);
    let sources = vec!["removal.st".to_owned()];
    let mut options = module_options(&removed, "removal");
    options.baseline = Some(&first.manifest);
    let errors = project(&hir, &sources, &options).expect_err("removal is diagnosed");
    assert!(
        errors
            .iter()
            .any(|error| error.code == "STC-E-MANIFEST-003"),
        "{errors:#?}"
    );

    let mut options = module_options(&removed, "removal");
    options.baseline = Some(&first.manifest);
    options.allow_untracked_removal = true;
    let second = project(&hir, &sources, &options).expect("allowed removal");
    assert!(
        second
            .manifest
            .plan
            .actions
            .iter()
            .any(|action| { action.key == "_ROOT::LIMIT" && action.action == PlanAction::Drop })
    );
    assert!(second.manifest.history.retired.iter().any(|retired| {
        retired.key == "_ROOT::LIMIT" && retired.retired_build_id == first.manifest.header.build_id
    }));
}

#[test]
fn layout_change_is_rejected_and_keeps_the_old_version_usable() {
    let changed = SOURCE.replace("  Limit : INT;", "  Limit : LINT;");
    let first = project_source(SOURCE, "reject");
    let hir = common::clean_hir(&changed);
    let sources = vec!["reject.st".to_owned()];
    let mut options = module_options(&changed, "reject");
    options.baseline = Some(&first.manifest);
    let second = project(&hir, &sources, &options).expect("rejected manifest projection");
    assert_eq!(second.verdict(), PlanVerdict::Rejected);
    assert!(!second.is_deployable());
    assert!(
        second
            .manifest
            .plan
            .actions
            .iter()
            .any(|action| { action.key == "_ROOT::LIMIT" && action.action == PlanAction::Reject })
    );
    assert!(
        second
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "STC-E-ONLINE-001")
    );
}

#[test]
fn a_tampered_descriptor_or_manifest_is_rejected() {
    let artifacts = project_source(SIMPLE, "tamper");
    let mut bytes = artifacts.descriptor_bytes.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(stc_got::Descriptor::decode(&bytes).is_err());

    let mut bytes = artifacts.manifest_binary.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(stc_got::Manifest::decode_binary(&bytes).is_err());

    let mut manifest = artifacts.manifest.clone();
    manifest.header.build_id = stc_got::GotId::ZERO;
    assert!(manifest.validate().is_err());
}

#[test]
fn golden_descriptor_and_manifest_snapshots() {
    let artifacts = project_source(SIMPLE, "golden");
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let json_path = directory.join("module.stc-got.json");
    let descriptor_path = directory.join("module.stc-desc.hex");
    let binary_path = directory.join("module.stc-got.hex");
    let update = std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some();
    if update {
        std::fs::create_dir_all(&directory).expect("snapshot directory");
        std::fs::write(&json_path, &artifacts.manifest_json).expect("write JSON snapshot");
        std::fs::write(&descriptor_path, hex(&artifacts.descriptor_bytes)).expect("descriptor");
        std::fs::write(&binary_path, hex(&artifacts.manifest_binary)).expect("manifest");
    }
    let expected_json =
        std::fs::read_to_string(&json_path).expect("missing snapshot; run STC_UPDATE_SNAPSHOTS=1");
    assert_eq!(expected_json, artifacts.manifest_json);
    let expected_descriptor = std::fs::read_to_string(&descriptor_path)
        .expect("missing snapshot; run STC_UPDATE_SNAPSHOTS=1");
    assert_eq!(expected_descriptor, hex(&artifacts.descriptor_bytes));
    let expected_binary = std::fs::read_to_string(&binary_path)
        .expect("missing snapshot; run STC_UPDATE_SNAPSHOTS=1");
    assert_eq!(expected_binary, hex(&artifacts.manifest_binary));
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 1);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('\n');
    out
}

#[test]
fn symbol_bindings_match_the_descriptor_function_order() {
    let artifacts = project_source(SOURCE, "symbols");
    let function_bindings = artifacts
        .slots
        .iter()
        .filter(|binding| binding.kind == stc_codegen::ir::CellKind::Function)
        .collect::<Vec<_>>();
    assert_eq!(
        function_bindings.len(),
        artifacts.descriptor.functions.len(),
        "one function slot per descriptor function"
    );
    let mut got_ids = function_bindings
        .iter()
        .map(|binding| binding.got_id)
        .collect::<Vec<_>>();
    got_ids.sort();
    let descriptor_ids = artifacts
        .descriptor
        .functions
        .iter()
        .map(|function| function.got_id)
        .collect::<Vec<_>>();
    assert_eq!(got_ids, descriptor_ids);
    assert!(
        function_bindings
            .iter()
            .all(|binding| binding.symbol.is_some())
    );
}
