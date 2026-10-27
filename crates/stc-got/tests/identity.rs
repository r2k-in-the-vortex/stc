//! M4.5 phase 2 acceptance tests: first/second-build identity, tracked
//! rename/move, preserve/allocate/drop, deterministic manifest snapshots, and
//! malformed-manifest rejection.

use std::path::Path;

use stc_got::{
    BuildInput, BuildOptions, Catalog, Digest32, Endianness, ExportDecl, FunctionDecl,
    FunctionKind, Identity, IdentityKind, ImportService, InitializerKind, InstanceDecl, Manifest,
    PlanAction, PlanVerdict, Rename, Retention, RootDecl, RootKind, Signature, Target, TaskDecl,
    TypeKey, build, codes, elementary_layout, elementary_layout_hash, export_identity_key,
    field_root_key, function_key, global_root_key, instance_identity_key, program_body_key,
    source_fingerprint, task_guard_root_key, task_identity_key,
};

fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

fn input() -> BuildInput {
    let mut input = BuildInput::new("stc-2013-oop-utf8-v1", "stc 0.1.0", target(), "phase2");
    input.descriptor_digest = Digest32::from_bytes([0x11; 32]);
    input.source_fingerprint =
        source_fingerprint(&[("_ROOT/main.st".into(), b"PROGRAM Main\n".to_vec())]).unwrap();
    input
}

fn layout(ty: &TypeKey) -> (stc_got::GotId, u64, u32) {
    let layout = elementary_layout(ty).expect("elementary layout");
    (
        elementary_layout_hash(&target(), ty).expect("elementary hash"),
        layout.size,
        layout.align,
    )
}

fn root_global(path: &str, ty: TypeKey, retention: Retention) -> RootDecl {
    let (layout_hash, size, align) = layout(&ty);
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", path, ""),
        key: global_root_key("_ROOT", path).unwrap(),
        root_kind: RootKind::Global,
        owner_type: None,
        field_path: String::new(),
        type_key: ty.to_string(),
        layout_hash,
        size,
        align,
        retention,
        initializer: if retention == Retention::Retain {
            InitializerKind::Constant
        } else {
            InitializerKind::Zero
        },
        initializer_digest: None,
        source: None,
    }
}

fn root_field(program: &str, field: &str, instance: &str, ty: TypeKey) -> RootDecl {
    let (layout_hash, size, align) = layout(&ty);
    let path = format!("{program}.{field}");
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", &path, instance),
        key: field_root_key("_ROOT", &path, instance).unwrap(),
        root_kind: RootKind::ProgramField,
        owner_type: None,
        field_path: field.into(),
        type_key: ty.to_string(),
        layout_hash,
        size,
        align,
        retention: Retention::None,
        initializer: InitializerKind::Zero,
        initializer_digest: None,
        source: None,
    }
}

fn root_direct(path: &str, instance: Option<&str>, ty: TypeKey) -> RootDecl {
    let (layout_hash, size, align) = layout(&ty);
    RootDecl {
        identity: Identity::new(IdentityKind::Direct, "_ROOT", path, instance.unwrap_or("")),
        key: stc_got::direct_root_key("_ROOT", path, instance).unwrap(),
        root_kind: RootKind::Direct,
        owner_type: None,
        field_path: path.rsplit('.').next().unwrap_or(path).into(),
        type_key: ty.to_string(),
        layout_hash,
        size,
        align,
        retention: Retention::None,
        initializer: InitializerKind::None,
        initializer_digest: None,
        source: None,
    }
}

fn task_guard() -> RootDecl {
    let (layout_hash, size, align) = layout(&TypeKey::Bool);
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", "MAIN/R1#FAST.GUARD", ""),
        key: task_guard_root_key("_ROOT", "MAIN", "R1", "FAST").unwrap(),
        root_kind: RootKind::TaskGuard,
        owner_type: None,
        field_path: String::new(),
        type_key: "BOOL".into(),
        layout_hash,
        size,
        align,
        retention: Retention::None,
        initializer: InitializerKind::Zero,
        initializer_digest: None,
        source: None,
    }
}

fn sign() -> Signature {
    Signature::parse("(IN:DINT,IN:DINT)->DINT").unwrap()
}

fn user_function() -> FunctionDecl {
    let signature = sign();
    FunctionDecl {
        identity: Identity::new(
            IdentityKind::Function,
            "_ROOT",
            "MATH.ADD",
            signature.to_string(),
        ),
        key: function_key("_ROOT", "MATH.ADD", &signature).unwrap(),
        signature_key: signature.to_string(),
        function_kind: FunctionKind::Function,
        owner_type: None,
        flags: 1,
        export_name: None,
        vtable_slot: None,
        source: None,
    }
}

fn program_body() -> FunctionDecl {
    FunctionDecl {
        identity: Identity::new(IdentityKind::Function, "_ROOT", "MAIN#BODY", "()->VOID"),
        key: program_body_key("_ROOT", "MAIN").unwrap(),
        signature_key: "()->VOID".into(),
        function_kind: FunctionKind::ProgramBody,
        owner_type: None,
        flags: 0b10,
        export_name: None,
        vtable_slot: None,
        source: None,
    }
}

fn export_decl() -> ExportDecl {
    let signature = sign();
    let function = function_key("_ROOT", "MATH.ADD", &signature).unwrap();
    let qualifier = format!("{}#C:stc_add", signature);
    ExportDecl {
        identity: Identity::new(IdentityKind::Export, "_ROOT", "MATH.ADD", qualifier),
        key: export_identity_key("_ROOT", "MATH.ADD", &signature, "stc_add").unwrap(),
        c_name: "stc_add".into(),
        function_key: function,
        signature_key: signature.to_string(),
        no_mangle: false,
        source: None,
    }
}

fn instance_decl() -> InstanceDecl {
    let task_key = task_identity_key("_ROOT", "MAIN", "R1", "FAST").unwrap();
    InstanceDecl {
        identity: Identity::new(IdentityKind::Instance, "_ROOT", "MAIN/R1/INST", "MAIN"),
        key: instance_identity_key("_ROOT", "MAIN", "R1", "INST").unwrap(),
        program_type: None,
        program_type_key: "MAIN".into(),
        configuration: "MAIN".into(),
        resource: "R1".into(),
        instance_path: "MAIN/R1/INST".into(),
        task_key: Some(task_key),
        source: None,
    }
}

fn task_decl() -> TaskDecl {
    TaskDecl {
        identity: Identity::new(IdentityKind::Task, "_ROOT", "MAIN/R1", "FAST"),
        key: task_identity_key("_ROOT", "MAIN", "R1", "FAST").unwrap(),
        configuration: "MAIN".into(),
        resource: "R1".into(),
        name: "FAST".into(),
        single: true,
        interval_ns: None,
        priority: None,
        guard_key: Some(task_guard_root_key("_ROOT", "MAIN", "R1", "FAST").unwrap()),
        source: None,
    }
}

fn catalog() -> Catalog {
    Catalog {
        roots: vec![
            root_global("COUNTER", TypeKey::Dint, Retention::Retain),
            root_global("LIMIT", TypeKey::Int, Retention::None),
            root_field("MAIN", "SPEED", "MAIN/R1/INST", TypeKey::Dint),
            root_direct("MAIN.INPUT", Some("MAIN/R1/INST"), TypeKey::Bool),
            task_guard(),
        ],
        functions: vec![program_body(), user_function()],
        types: Vec::new(),
        descriptors: Vec::new(),
        instances: vec![instance_decl()],
        tasks: vec![task_decl()],
        exports: vec![export_decl()],
        aggregates: Vec::new(),
        imports: vec![
            ImportService::Trap,
            ImportService::CycleNowNs,
            ImportService::IoImage,
            ImportService::IoSize,
            ImportService::StateLoad,
            ImportService::StateSave,
        ],
    }
}

fn renamed_catalog() -> Catalog {
    let mut catalog = catalog();
    let index = catalog
        .roots
        .iter()
        .position(|root| root.identity.path == "COUNTER")
        .unwrap();
    catalog.roots[index] = root_global("TICKS", TypeKey::Dint, Retention::Retain);
    catalog
}

fn without_limit() -> Catalog {
    let mut catalog = catalog();
    catalog.roots.retain(|root| root.identity.path != "LIMIT");
    catalog
}

fn changed_layout() -> Catalog {
    let mut catalog = catalog();
    let index = catalog
        .roots
        .iter()
        .position(|root| root.identity.path == "COUNTER")
        .unwrap();
    catalog.roots[index] = root_global("COUNTER", TypeKey::Lint, Retention::Retain);
    catalog
}

fn changed_direct_layout() -> Catalog {
    let mut catalog = catalog();
    let index = catalog
        .roots
        .iter()
        .position(|root| root.identity.path == "MAIN.INPUT")
        .unwrap();
    catalog.roots[index] = root_direct("MAIN.INPUT", Some("MAIN/R1/INST"), TypeKey::Dint);
    catalog
}

fn without_export() -> Catalog {
    let mut catalog = catalog();
    catalog.exports.clear();
    catalog
}

fn cold() -> stc_got::BuildOutcome {
    build(&catalog(), &input(), &BuildOptions::default()).expect("cold build")
}

#[test]
fn cold_build_round_trips_and_is_deterministic() {
    let first = cold();
    assert_eq!(first.verdict(), PlanVerdict::Cold);
    assert!(first.is_deployable());
    assert!(first.manifest.header.parent_build_id.is_none());
    assert!(
        first
            .manifest
            .plan
            .actions
            .iter()
            .all(|action| action.action == PlanAction::Allocate)
    );
    assert!(first.manifest.header.generator_flags & (1 << 6) != 0);

    let decoded = Manifest::decode_binary(&first.binary).expect("binary round trip");
    assert_eq!(decoded, first.manifest);
    let parsed = Manifest::decode_json(&first.json).expect("json round trip");
    assert_eq!(parsed, first.manifest);

    let second = cold();
    assert_eq!(second.binary, first.binary);
    assert_eq!(second.json, first.json);
    assert_eq!(
        second.manifest.header.build_id,
        first.manifest.header.build_id
    );
}

#[test]
fn second_build_preserves_matching_identities() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&catalog(), &input(), &options).expect("online build");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    assert!(second.is_deployable());
    assert_eq!(
        second.manifest.header.parent_build_id,
        Some(first.manifest.header.build_id)
    );
    assert_eq!(
        second.manifest.header.parent_manifest_digest,
        Some(first.manifest.digest().unwrap())
    );
    assert_ne!(
        second.manifest.header.build_id,
        first.manifest.header.build_id
    );
    for root in &first.manifest.roots {
        let same = second
            .manifest
            .roots
            .iter()
            .find(|candidate| candidate.key == root.key)
            .expect("root survives");
        assert_eq!(same.got_id, root.got_id, "{}", root.key);
        assert_eq!(same.previous_keys, root.previous_keys);
        let action = second
            .manifest
            .plan
            .actions
            .iter()
            .find(|action| action.got_id == root.got_id)
            .unwrap();
        if root.root_kind == RootKind::Direct {
            assert_eq!(action.action, PlanAction::Allocate);
        } else {
            assert_eq!(action.action, PlanAction::Preserve, "{}", root.key);
        }
    }
}

#[test]
fn tracked_rename_preserves_the_got_id() {
    let first = cold();
    let old_key = "_ROOT::COUNTER";
    let new_key = "_ROOT::TICKS";
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        renames: vec![Rename::new(old_key, new_key)],
        ..BuildOptions::default()
    };
    let second = build(&renamed_catalog(), &input(), &options).expect("renamed build");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    let old = first
        .manifest
        .roots
        .iter()
        .find(|root| root.key == old_key)
        .unwrap();
    let renamed = second
        .manifest
        .roots
        .iter()
        .find(|root| root.key == new_key)
        .unwrap();
    assert_eq!(renamed.got_id, old.got_id);
    assert_eq!(renamed.previous_keys, vec![old_key.to_owned()]);
    let action = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.got_id == old.got_id)
        .unwrap();
    assert_eq!(action.action, PlanAction::Preserve);
    assert_eq!(action.old_key.as_deref(), Some(old_key));
}

#[test]
fn untracked_removal_requires_the_explicit_policy() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let error = build(&without_limit(), &input(), &options).expect_err("removal must fail");
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::MANIFEST_UNTRACKED)
    );

    let allowed = BuildOptions {
        baseline: Some(&first.manifest),
        allow_untracked_removal: true,
        ..BuildOptions::default()
    };
    let outcome = build(&without_limit(), &input(), &allowed).expect("allowed removal");
    assert_eq!(outcome.verdict(), PlanVerdict::OnlineCandidate);
    let removed = first
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::LIMIT")
        .unwrap();
    let action = outcome
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.got_id == removed.got_id)
        .unwrap();
    assert_eq!(action.action, PlanAction::Drop);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::W_ONLINE_REMOVED)
    );
    assert!(
        outcome
            .manifest
            .history
            .retired
            .iter()
            .any(|record| record.got_id == removed.got_id)
    );
}

#[test]
fn layout_change_is_rejected_by_default() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&changed_layout(), &input(), &options).expect("plan build");
    assert_eq!(second.verdict(), PlanVerdict::Rejected);
    assert!(!second.is_deployable());
    let counter = second
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .unwrap();
    let action = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.got_id == counter.got_id)
        .unwrap();
    assert_eq!(action.action, PlanAction::Reject);
    assert!(
        second
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::ONLINE_LAYOUT)
    );
}

#[test]
fn a_direct_root_with_a_changed_type_is_remapped_with_a_warning() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&changed_direct_layout(), &input(), &options).expect("plan build");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    assert!(second.is_deployable());
    assert!(
        second
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::W_ONLINE_DIRECT),
        "{:#?}",
        second.diagnostics
    );
}

#[test]
fn export_removal_is_rejected() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&without_export(), &input(), &options).expect("plan build");
    assert_eq!(second.verdict(), PlanVerdict::Rejected);
    assert!(
        second
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::ONLINE_EXPORT)
    );
}

#[test]
fn slot_assignment_is_canonical() {
    let outcome = cold();
    let manifest = &outcome.manifest;
    assert_eq!(
        manifest
            .slots
            .iter()
            .map(|slot| slot.slot)
            .collect::<Vec<_>>(),
        (0..manifest.slots.len() as u32).collect::<Vec<_>>()
    );
    let data: Vec<_> = manifest
        .slots
        .iter()
        .filter(|slot| slot.kind == stc_got::SlotKind::Data)
        .map(|slot| slot.got_id)
        .collect();
    assert!(data.windows(2).all(|pair| pair[0] < pair[1]));
    for root in &manifest.roots {
        let slot = manifest
            .slots
            .iter()
            .find(|slot| slot.got_id == root.got_id)
            .unwrap();
        assert_eq!(slot.slot, root.slot);
        let expected = if root.root_kind == RootKind::Direct {
            stc_got::SlotKind::Direct
        } else {
            stc_got::SlotKind::Data
        };
        assert_eq!(slot.kind, expected);
    }
    let services: Vec<_> = manifest
        .slots
        .iter()
        .filter(|slot| slot.kind == stc_got::SlotKind::Import)
        .map(|slot| slot.service)
        .collect();
    assert_eq!(services, vec![1, 2, 7, 8, 9, 10]);
}

#[test]
fn binary_tampering_and_truncation_are_rejected() {
    let outcome = cold();
    let mut tampered = outcome.binary.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    assert!(Manifest::decode_binary(&tampered).is_err());

    let mut bad_magic = outcome.binary.clone();
    bad_magic[0] ^= 0xff;
    assert!(Manifest::decode_binary(&bad_magic).is_err());

    assert!(Manifest::decode_binary(&outcome.binary[..20]).is_err());
    assert!(Manifest::decode_binary(&[outcome.binary.as_slice(), &[0]].concat()).is_err());
}

#[test]
fn json_tampering_is_rejected() {
    let outcome = cold();
    let text = outcome.json.replacen("0x", "0x00", 1);
    assert!(Manifest::decode_json(&text).is_err());
}

#[test]
fn duplicate_root_keys_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    let duplicate = manifest.roots[0].clone();
    manifest.roots.push(duplicate);
    assert!(manifest.validate().is_err());
}

#[test]
fn golden_manifest_snapshot() {
    let outcome = cold();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/cold.stc-got.json");
    if std::env::var_os("STC_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &outcome.json).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot; run with STC_UPDATE_SNAPSHOTS=1"));
    assert_eq!(expected, outcome.json);
}

#[test]
fn target_mismatched_baseline_selects_cold_layout() {
    let first = cold();
    let mut mismatched = input();
    mismatched.target = Target::new("aarch64-unknown-linux-gnu", Endianness::Little, 8);
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let outcome = build(&catalog(), &mismatched, &options).expect("cold-layout fallback");
    assert_eq!(outcome.verdict(), PlanVerdict::Cold);
    assert!(outcome.manifest.header.parent_build_id.is_none());
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::W_ONLINE_STALE_BASELINE)
    );
    assert!(outcome.manifest.header.generator_flags & (1 << 6) != 0);
}

#[test]
fn forced_cold_ignores_a_usable_baseline() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        cold: true,
        ..BuildOptions::default()
    };
    let outcome = build(&catalog(), &input(), &options).expect("forced cold build");
    assert_eq!(outcome.verdict(), PlanVerdict::Cold);
    assert!(outcome.manifest.header.parent_build_id.is_none());
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::W_MANIFEST_COLD)
    );
}

#[test]
fn reintroduced_retired_identity_is_rejected() {
    let first = cold();
    let removed = build(
        &without_limit(),
        &input(),
        &BuildOptions {
            baseline: Some(&first.manifest),
            allow_untracked_removal: true,
            ..BuildOptions::default()
        },
    )
    .expect("allowed removal");
    assert!(
        removed
            .manifest
            .history
            .retired
            .iter()
            .any(|record| record.key == "_ROOT::LIMIT")
    );
    let error = build(
        &catalog(),
        &input(),
        &BuildOptions {
            baseline: Some(&removed.manifest),
            ..BuildOptions::default()
        },
    )
    .expect_err("reintroduction must not resurrect a retired identity");
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::MANIFEST_INVALID)
    );
}

#[test]
fn json_to_binary_reproduces_the_exact_artifact() {
    let outcome = cold();
    let parsed = Manifest::decode_json(&outcome.json).expect("json decode");
    assert_eq!(parsed.encode_binary().unwrap(), outcome.binary);
    assert_eq!(parsed.digest().unwrap(), outcome.manifest.digest().unwrap());
}

#[test]
fn non_canonical_table_order_is_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.roots.swap(0, 1);
    assert!(manifest.validate().is_err());
}

#[test]
fn duplicate_got_ids_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.roots[1].got_id = manifest.roots[0].got_id;
    assert!(manifest.validate().is_err());
}

#[test]
fn duplicate_slots_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.slots[1].slot = manifest.slots[0].slot;
    assert!(manifest.validate().is_err());
}

#[test]
fn duplicate_slot_identities_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    let first = manifest.slots[0].got_id;
    let duplicate = manifest
        .slots
        .iter_mut()
        .find(|slot| slot.kind == stc_got::SlotKind::Data && slot.got_id != first)
        .expect("a second data slot");
    duplicate.got_id = first;
    assert!(manifest.validate().is_err());
}

#[test]
fn unknown_required_features_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.header.required_features |= 1 << 10;
    assert!(manifest.validate().is_err());
}

#[test]
fn an_unpaired_parent_identity_is_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.header.parent_build_id = Some(stc_got::GotId::from_bytes([0x22; 16]));
    assert!(manifest.validate().is_err());
}

#[test]
fn persistent_pointer_roots_are_rejected() {
    let outcome = cold();
    let mut manifest = outcome.manifest.clone();
    manifest.roots[0].type_key = "PTR_TO(DINT)".into();
    assert!(manifest.validate().is_err());
}

fn namespace_moved_catalog() -> Catalog {
    let mut catalog = catalog();
    let signature = sign();
    let new_function = function_key("_ROOT", "NUM.ADD", &signature).unwrap();
    let index = catalog
        .functions
        .iter()
        .position(|function| function.identity.path == "MATH.ADD")
        .unwrap();
    catalog.functions[index] = FunctionDecl {
        identity: Identity::new(
            IdentityKind::Function,
            "_ROOT",
            "NUM.ADD",
            signature.to_string(),
        ),
        key: new_function.clone(),
        signature_key: signature.to_string(),
        function_kind: FunctionKind::Function,
        owner_type: None,
        flags: 1,
        export_name: None,
        vtable_slot: None,
        source: None,
    };
    let qualifier = format!("{}#C:stc_add", signature);
    catalog.exports[0] = ExportDecl {
        identity: Identity::new(IdentityKind::Export, "_ROOT", "NUM.ADD", qualifier),
        key: export_identity_key("_ROOT", "NUM.ADD", &signature, "stc_add").unwrap(),
        c_name: "stc_add".into(),
        function_key: new_function,
        signature_key: signature.to_string(),
        no_mangle: false,
        source: None,
    };
    catalog
}

fn field_renamed_catalog() -> Catalog {
    let mut catalog = catalog();
    let index = catalog
        .roots
        .iter()
        .position(|root| root.identity.path == "MAIN.SPEED")
        .unwrap();
    catalog.roots[index] = root_field("MAIN", "RATE", "MAIN/R1/INST", TypeKey::Dint);
    catalog
}

#[test]
fn namespace_move_preserves_nested_identities() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        renames: vec![Rename::new("_ROOT::MATH", "_ROOT::NUM")],
        ..BuildOptions::default()
    };
    let second = build(&namespace_moved_catalog(), &input(), &options).expect("moved build");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);

    let old_function = first
        .manifest
        .functions
        .iter()
        .find(|function| function.key.contains("MATH.ADD@"))
        .unwrap();
    let new_function = second
        .manifest
        .functions
        .iter()
        .find(|function| function.key.contains("NUM.ADD@"))
        .unwrap();
    assert_eq!(new_function.got_id, old_function.got_id);
    assert_eq!(new_function.previous_keys, vec![old_function.key.clone()]);

    let old_export = &first.manifest.exports[0];
    let new_export = &second.manifest.exports[0];
    assert_eq!(new_export.got_id, old_export.got_id);
    assert_eq!(new_export.previous_keys, vec![old_export.key.clone()]);
    assert_eq!(new_export.function_got_id, new_function.got_id);
}

#[test]
fn field_rename_preserves_the_field_identity() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        renames: vec![Rename::new("MAIN.SPEED", "MAIN.RATE")],
        ..BuildOptions::default()
    };
    let second = build(&field_renamed_catalog(), &input(), &options).expect("field rename");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    let old = first
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::MAIN.SPEED#MAIN/R1/INST")
        .unwrap();
    let new = second
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::MAIN.RATE#MAIN/R1/INST")
        .unwrap();
    assert_eq!(new.got_id, old.got_id);
    assert_eq!(
        new.previous_keys,
        vec!["_ROOT::MAIN.SPEED#MAIN/R1/INST".to_owned()]
    );
}

#[test]
fn an_untracked_rename_is_a_removal_plus_allocation() {
    let first = cold();
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        allow_untracked_removal: true,
        ..BuildOptions::default()
    };
    let second = build(&renamed_catalog(), &input(), &options).expect("untracked rename");
    assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
    let old = first
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::COUNTER")
        .unwrap();
    let new = second
        .manifest
        .roots
        .iter()
        .find(|root| root.key == "_ROOT::TICKS")
        .unwrap();
    assert_ne!(new.got_id, old.got_id);
    let dropped = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.got_id == old.got_id)
        .unwrap();
    assert_eq!(dropped.action, PlanAction::Drop);
    let allocated = second
        .manifest
        .plan
        .actions
        .iter()
        .find(|action| action.got_id == new.got_id)
        .unwrap();
    assert_eq!(allocated.action, PlanAction::Allocate);
}
