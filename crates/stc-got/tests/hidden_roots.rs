//! Phase 4b.1 acceptance tests: hidden `HANDLE`/`OBJREC` roots pass catalog,
//! manifest, slot, and online-change classification with no format change.

use stc_got::{
    BuildInput, BuildOptions, Catalog, DescriptorDecl, Digest32, Endianness, GotId, Identity,
    IdentityKind, InitializerKind, Manifest, PlanAction, PlanVerdict, Retention, RootDecl,
    RootKind, SlotKind, Target, TypeDecl, TypeKey, build, descriptor_identity_key, global_root_key,
    hidden_root_key, hidden_root_marker, source_fingerprint, type_identity_key,
};

fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

fn input() -> BuildInput {
    let mut input = BuildInput::new("stc-2013-oop-utf8-v1", "stc 0.1.0", target(), "hidden");
    input.descriptor_digest = Digest32::from_bytes([0x11; 32]);
    input.source_fingerprint =
        source_fingerprint(&[("_ROOT/main.st".into(), b"PROGRAM Main\n".to_vec())]).unwrap();
    input
}

fn hidden_root(
    path: &str,
    instance: Option<&str>,
    marker: &str,
    type_key: &str,
    size: u64,
    align: u32,
) -> RootDecl {
    let qualifier = match instance {
        Some(instance) => format!("{instance}/{marker}"),
        None => marker.to_owned(),
    };
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", path, qualifier),
        key: hidden_root_key("_ROOT", path, instance, marker).unwrap(),
        root_kind: RootKind::NestedField,
        owner_type: None,
        field_path: path.rsplit('.').next().unwrap_or(path).to_owned(),
        type_key: type_key.into(),
        layout_hash: GotId::from_bytes([0x42; 16]),
        size,
        align,
        retention: Retention::None,
        initializer: InitializerKind::None,
        initializer_digest: None,
        source: None,
    }
}

fn ordinary_root(path: &str) -> RootDecl {
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", path, ""),
        key: global_root_key("_ROOT", path).unwrap(),
        root_kind: RootKind::Global,
        owner_type: None,
        field_path: String::new(),
        type_key: "DINT".into(),
        layout_hash: GotId::from_bytes([0x43; 16]),
        size: 4,
        align: 4,
        retention: Retention::None,
        initializer: InitializerKind::Zero,
        initializer_digest: None,
        source: None,
    }
}

fn catalog() -> Catalog {
    Catalog {
        roots: vec![
            ordinary_root("G"),
            hidden_root(
                "G",
                None,
                "HANDLE",
                "PTR_TO(T:0123456789abcdef0123456789abcdef)",
                8,
                8,
            ),
            hidden_root(
                "G",
                None,
                "OBJREC",
                "ARRAY[0..1]OF(PTR_TO(T:0123456789abcdef0123456789abcdef))",
                16,
                8,
            ),
            hidden_root(
                "MAIN.M",
                Some("C/R/INST"),
                "HANDLE",
                "PTR_TO(T:0123456789abcdef0123456789abcdef)",
                8,
                8,
            ),
            hidden_root(
                "MAIN.M",
                Some("C/R/INST"),
                "OBJREC",
                "ARRAY[0..1]OF(PTR_TO(T:0123456789abcdef0123456789abcdef))",
                16,
                8,
            ),
        ],
        ..Catalog::default()
    }
}

#[test]
fn hidden_roots_get_data_slots_and_survive_an_online_build() {
    let first_catalog = catalog();
    first_catalog.validate().expect("hidden catalog");
    let first = build(&first_catalog, &input(), &BuildOptions::default()).expect("cold build");
    assert_eq!(first.manifest.plan.verdict, PlanVerdict::Cold);
    for root in &first_catalog.roots {
        let record = first
            .manifest
            .roots
            .iter()
            .find(|record| record.key == root.key)
            .unwrap_or_else(|| panic!("missing {}", root.key));
        if hidden_root_marker(&root.key).is_some() {
            assert_eq!(record.root_kind, RootKind::NestedField);
            assert_eq!(record.retention, Retention::None);
        }
        let slot = first
            .manifest
            .slots
            .iter()
            .find(|slot| slot.got_id == record.got_id)
            .expect("data slot");
        assert_eq!(slot.kind, stc_got::SlotKind::Data);
        assert_eq!(
            hidden_root_marker(&root.key).is_some(),
            root.key.contains('#')
        );
    }

    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&first_catalog, &input(), &options).expect("online build");
    assert_eq!(second.manifest.plan.verdict, PlanVerdict::OnlineCandidate);
    for root in &first_catalog.roots {
        let action = second
            .manifest
            .plan
            .actions
            .iter()
            .find(|action| action.key == root.key)
            .expect("action");
        assert_eq!(action.action, PlanAction::Preserve, "{}", root.key);
    }
    let repeat = build(&first_catalog, &input(), &options).expect("repeat");
    assert_eq!(
        second.manifest.encode_binary().unwrap(),
        repeat.manifest.encode_binary().unwrap(),
        "double builds are byte-identical"
    );
    Manifest::decode_binary(&second.manifest.encode_binary().unwrap())
        .expect("manifest round-trip")
        .validate()
        .expect("valid manifest");
}

#[test]
fn catalog_rejects_malformed_hidden_roots() {
    let mut wrong_kind = catalog();
    wrong_kind.roots[1].root_kind = RootKind::Global;
    assert!(
        wrong_kind.validate().is_err(),
        "hidden root must be nested_field"
    );

    let mut no_pointer = catalog();
    no_pointer.roots[1].type_key = "DINT".into();
    assert!(
        no_pointer.validate().is_err(),
        "hidden root needs a handle type"
    );

    let mut ordinary_pointer = catalog();
    ordinary_pointer.roots[0].type_key = "PTR_TO(DINT)".into();
    assert!(
        ordinary_pointer.validate().is_err(),
        "only hidden roots may use a pointer type key"
    );

    let mut bad_marker = catalog();
    bad_marker.roots[1].key = "_ROOT::G#HANDLES".into();
    assert!(bad_marker.validate().is_err(), "unknown marker");

    // Direct roots may not be hidden.
    let mut direct = catalog();
    direct.roots[1].root_kind = RootKind::Direct;
    direct.roots[1].identity.kind = IdentityKind::Direct;
    assert!(direct.validate().is_err(), "marker on a direct root");
}

#[test]
fn descriptors_get_slots_and_are_preserved_by_key() {
    let type_key = "T:0123456789abcdef0123456789abcdef";
    let layout_hash = GotId::from_bytes([0x52; 16]);
    let catalog = Catalog {
        types: vec![TypeDecl {
            identity: Identity::new(IdentityKind::Type, "_ROOT", "MY.CLASS", ""),
            key: type_identity_key("_ROOT", "MY.CLASS").unwrap(),
            type_key: type_key.into(),
            layout_hash,
            size: 8,
            align: 8,
            type_kind: stc_got::TypeKind::Class,
            base: None,
            interfaces: Vec::new(),
            vtable_shape: Some(GotId::from_bytes([0x61; 16])),
            fields: Vec::new(),
            enum_members: Vec::new(),
            subrange: None,
            source: None,
        }],
        descriptors: ["TYPEINFO", "VTABLE"]
            .into_iter()
            .map(|kind| DescriptorDecl {
                identity: Identity::new(IdentityKind::Descriptor, "_ROOT", "MY.CLASS", kind),
                key: descriptor_identity_key("_ROOT", "MY.CLASS", kind).unwrap(),
                descriptor_kind: if kind == "TYPEINFO" {
                    stc_got::DescriptorKind::TypeInfo
                } else {
                    stc_got::DescriptorKind::Vtable
                },
                owner_type: Some(type_identity_key("_ROOT", "MY.CLASS").unwrap()),
                type_key: type_key.into(),
                layout_hash,
                size: 8,
                align: 8,
                source: None,
            })
            .collect(),
        ..Catalog::default()
    };
    catalog.validate().expect("descriptor catalog");
    let first = build(&catalog, &input(), &BuildOptions::default()).expect("cold build");
    assert_eq!(first.manifest.descriptors.len(), 2);
    for descriptor in &first.manifest.descriptors {
        let slot = first
            .manifest
            .slots
            .iter()
            .find(|slot| slot.got_id == descriptor.got_id)
            .expect("descriptor slot");
        assert_eq!(slot.kind, SlotKind::Descriptor);
        assert_eq!(descriptor.type_key, type_key);
        assert_eq!(descriptor.layout_hash, layout_hash);
        let owner = descriptor.owner_type.expect("owner type");
        assert_eq!(
            first
                .manifest
                .types
                .iter()
                .find(|ty| ty.got_id == owner)
                .expect("owner record")
                .type_key,
            type_key
        );
    }

    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let second = build(&catalog, &input(), &options).expect("online build");
    for descriptor in &second.manifest.descriptors {
        assert_eq!(descriptor.got_id, {
            first
                .manifest
                .descriptors
                .iter()
                .find(|first| first.key == descriptor.key)
                .expect("baseline descriptor")
                .got_id
        });
        let action = second
            .manifest
            .plan
            .actions
            .iter()
            .find(|action| action.got_id == descriptor.got_id)
            .expect("descriptor action");
        assert_eq!(action.action, PlanAction::Preserve);
    }
}

#[test]
fn hidden_root_keys_do_not_collide_with_source_names() {
    let handle = hidden_root_key("_ROOT", "MAIN.MOTOR", None, "HANDLE").unwrap();
    let record = hidden_root_key("_ROOT", "MAIN.MOTOR", None, "OBJREC").unwrap();
    let field = stc_got::field_root_key("_ROOT", "MAIN.HANDLE", "C/R/INST").unwrap();
    let source_named = global_root_key("_ROOT", "MAIN.MOTOR").unwrap();
    assert_ne!(handle, record);
    assert_ne!(handle, field);
    assert_ne!(handle, source_named);
    assert_eq!(hidden_root_marker(&handle), Some("HANDLE"));
    assert_eq!(hidden_root_marker(&source_named), None);
    assert!(TypeKey::parse("PTR_TO(T:0123456789abcdef0123456789abcdef)").is_ok());
}
