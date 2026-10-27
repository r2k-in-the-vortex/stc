//! M6.5 phase-4 acceptance tests for manifest format 2 type metadata and
//! aggregate shapes.
//!
//! The tests use the public catalog/build API so the metadata path is exercised
//! exactly as `stc-codegen` uses it. Binary and canonical JSON must round-trip,
//! ordering is canonical, the catalog/build digests cover the new sections,
//! and format-1 manifests are rejected rather than reinterpreted.

use stc_got::{
    AggregateDecl, AggregateKind, BuildInput, BuildOptions, Catalog, Digest32, Endianness,
    EnumMemberDecl, FieldDecl, FunctionDecl, GotId, Identity, IdentityKind, MANIFEST_FORMAT,
    Manifest, Retention, RootDecl, RootKind, SubrangeDecl, Target, TypeDecl, TypeKey, TypeKind,
    build, field_identity_key, global_root_key, type_identity_key,
};

fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

fn input() -> BuildInput {
    let mut input = BuildInput::new("stc-2013-oop-utf8-v1", "stc 0.1.0", target(), "metadata");
    input.descriptor_digest = Digest32::from_bytes([0x31; 32]);
    input.source_fingerprint = stc_got::source_fingerprint(&[(
        "_ROOT/metadata.st".into(),
        b"TYPE Color : (Red, Blue, Green := 5); END_TYPE\n".to_vec(),
    )])
    .unwrap();
    input
}

fn type_decl(path: &str, type_key: &str, kind: TypeKind, layout: u8) -> TypeDecl {
    TypeDecl {
        identity: Identity::new(IdentityKind::Type, "_ROOT", path, ""),
        key: type_identity_key("_ROOT", path).unwrap(),
        type_key: type_key.into(),
        layout_hash: GotId::from_bytes([layout; 16]),
        size: if kind == TypeKind::Enum { 2 } else { 8 },
        align: if kind == TypeKind::Enum { 2 } else { 4 },
        type_kind: kind,
        base: None,
        interfaces: Vec::new(),
        vtable_shape: None,
        fields: Vec::new(),
        enum_members: Vec::new(),
        subrange: None,
        source: None,
    }
}

fn named(id: GotId) -> String {
    format!("T:{}", id.to_hex_plain())
}

fn named_type_id(path: &str) -> GotId {
    Identity::new(IdentityKind::Type, "_ROOT", path, "")
        .derive()
        .0
}

fn root_for(
    path: &str,
    type_key: &str,
    size: u64,
    align: u32,
    layout: GotId,
    retention: Retention,
) -> RootDecl {
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", path, ""),
        key: global_root_key("_ROOT", path).unwrap(),
        root_kind: RootKind::Global,
        owner_type: None,
        field_path: String::new(),
        type_key: type_key.into(),
        layout_hash: layout,
        size,
        align,
        retention,
        initializer: if retention == Retention::Retain {
            stc_got::InitializerKind::Constant
        } else {
            stc_got::InitializerKind::Zero
        },
        initializer_digest: None,
        source: None,
    }
}

fn metadata_catalog() -> Catalog {
    let color_id = named_type_id("COLOR");
    let small_id = named_type_id("SMALL");
    let point_id = named_type_id("POINT");

    let mut color = type_decl("COLOR", &named(color_id), TypeKind::Enum, 0x21);
    // Catalog validation requires the canonical `(value, name)` order.
    color.enum_members = vec![
        EnumMemberDecl {
            name: "RED".into(),
            value: 0,
        },
        EnumMemberDecl {
            name: "BLUE".into(),
            value: 1,
        },
        EnumMemberDecl {
            name: "GREEN".into(),
            value: 5,
        },
    ];

    let mut small = type_decl("SMALL", &named(small_id), TypeKind::Subrange, 0x22);
    small.size = 2;
    small.align = 2;
    small.subrange = Some(SubrangeDecl {
        lower: 0,
        upper: 10,
    });

    let mut point = type_decl("POINT", &named(point_id), TypeKind::Struct, 0x23);
    point.fields = vec![FieldDecl {
        identity: Identity::new(IdentityKind::Field, "_ROOT", "POINT.X", ""),
        path: "X".into(),
        type_key: "DINT".into(),
        layout_hash: stc_got::elementary_layout_hash(&target(), &TypeKey::Dint).unwrap(),
        offset: 0,
        size: 4,
        align: 4,
        retention: Retention::None,
        source: None,
    }];
    // The field identity key must exist for catalog validation; the build
    // derives field GotIds from the identity, not the key.
    let _ = field_identity_key("_ROOT", "POINT", "X").unwrap();

    Catalog {
        roots: vec![
            root_for(
                "COLOR",
                &named(color_id),
                2,
                2,
                GotId::from_bytes([0x21; 16]),
                Retention::None,
            ),
            root_for(
                "SMALL",
                &named(small_id),
                2,
                2,
                GotId::from_bytes([0x22; 16]),
                Retention::None,
            ),
            root_for(
                "POINT",
                &named(point_id),
                8,
                4,
                GotId::from_bytes([0x23; 16]),
                Retention::Retain,
            ),
        ],
        functions: Vec::<FunctionDecl>::new(),
        types: vec![color, small, point],
        descriptors: Vec::new(),
        instances: Vec::new(),
        tasks: Vec::new(),
        exports: Vec::new(),
        aggregates: vec![AggregateDecl {
            package: "_ROOT".into(),
            path: "POINT".into(),
            qualifier: None,
            kind: AggregateKind::Type,
            shape_key: named(point_id),
            layout_hash: GotId::from_bytes([0x23; 16]),
            fields: Vec::new(),
        }],
        imports: Vec::new(),
    }
}

fn cold() -> stc_got::BuildOutcome {
    build(&metadata_catalog(), &input(), &BuildOptions::default()).expect("cold build")
}

#[test]
fn format_two_metadata_round_trips_and_is_deterministic() {
    assert_eq!(MANIFEST_FORMAT, 2);
    assert_eq!(stc_got::manifest::MANIFEST_DOMAIN, "stc:manifest:v2");

    let first = cold();
    let manifest = &first.manifest;
    assert_eq!(manifest.type_metadata.len(), 2);
    assert_eq!(manifest.aggregates.len(), 1);

    let color_id = named_type_id("COLOR");
    let color = manifest
        .type_metadata
        .iter()
        .find(|record| record.type_id == color_id)
        .expect("color metadata");
    let names: Vec<&str> = color
        .enum_members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, vec!["RED", "BLUE", "GREEN"]);
    assert_eq!(color.enum_members[2].value, 5);
    assert!(color.subrange.is_none());

    let small = manifest
        .type_metadata
        .iter()
        .find(|record| record.type_id == named_type_id("SMALL"))
        .expect("small metadata");
    assert_eq!(small.subrange.expect("bounds").lower, 0);
    assert_eq!(small.subrange.expect("bounds").upper, 10);
    assert!(small.enum_members.is_empty());
    // Records are sorted by type identity.
    let mut sorted = manifest.type_metadata.clone();
    sorted.sort_by_key(|record| record.type_id);
    assert_eq!(sorted, manifest.type_metadata);

    let aggregate = &manifest.aggregates[0];
    assert_eq!(aggregate.package, "_ROOT");
    assert_eq!(aggregate.path, "POINT");
    assert_eq!(aggregate.kind, AggregateKind::Type);
    assert_eq!(aggregate.shape_key, named(named_type_id("POINT")));
    assert!(aggregate.fields.is_empty());

    let decoded_binary = Manifest::decode_binary(&first.binary).expect("binary round trip");
    assert_eq!(decoded_binary, *manifest);
    let decoded_json = Manifest::decode_json(&first.json).expect("json round trip");
    assert_eq!(decoded_json, *manifest);
    assert_eq!(decoded_binary.encode_binary().unwrap(), first.binary);
    assert_eq!(decoded_json.to_json_text().unwrap(), first.json);
    assert!(first.json.contains("\"type_metadata\""));
    assert!(first.json.contains("\"aggregates\""));
    assert!(first.json.contains("\"format\": 2"));

    let second = cold();
    assert_eq!(second.binary, first.binary);
    assert_eq!(second.json, first.json);
    assert_eq!(
        second.manifest.header.build_id, manifest.header.build_id,
        "type metadata and aggregates contribute to the build identity"
    );
}

#[test]
fn format_one_manifests_are_rejected_explicitly() {
    let built = cold();
    let mut old_binary = built.binary.clone();
    old_binary[8..10].copy_from_slice(&1_u16.to_le_bytes());
    let error = Manifest::decode_binary(&old_binary).expect_err("format 1 binary");
    assert!(error.message.contains("format 1"), "{}", error.message);

    let old_json = built.json.replacen("\"format\": 2", "\"format\": 1", 1);
    assert!(old_json.contains("\"format\": 1"));
    let error = Manifest::decode_json(&old_json).expect_err("format 1 JSON");
    assert!(error.message.contains("format 1"), "{}", error.message);
}

#[test]
fn catalog_and_build_digests_cover_type_metadata_and_aggregates() {
    let first = cold();

    let mut enum_changed = metadata_catalog();
    enum_changed.types[0].enum_members[2].value = 6;
    let second = build(&enum_changed, &input(), &BuildOptions::default()).expect("enum build");
    assert_ne!(
        second.manifest.header.catalog_digest, first.manifest.header.catalog_digest,
        "catalog digest covers enum metadata"
    );
    assert_ne!(
        second.manifest.header.build_id, first.manifest.header.build_id,
        "build ID covers enum metadata"
    );

    let mut aggregate_changed = metadata_catalog();
    aggregate_changed.aggregates[0].layout_hash = GotId::from_bytes([0x44; 16]);
    let third =
        build(&aggregate_changed, &input(), &BuildOptions::default()).expect("aggregate build");
    assert_ne!(
        third.manifest.header.catalog_digest, first.manifest.header.catalog_digest,
        "catalog digest covers aggregate shapes"
    );
    assert_ne!(
        third.manifest.header.build_id, first.manifest.header.build_id,
        "build ID covers aggregate shapes"
    );

    // Metadata-only changes with unchanged layouts remain an online candidate
    // against a format-2 baseline and carry the new metadata.
    let options = BuildOptions {
        baseline: Some(&first.manifest),
        ..BuildOptions::default()
    };
    let online = build(&enum_changed, &input(), &options).expect("online metadata change");
    assert!(matches!(
        online.verdict(),
        stc_got::PlanVerdict::OnlineCandidate
    ));
    assert_eq!(
        online.manifest.header.parent_build_id,
        Some(first.manifest.header.build_id)
    );
    assert_eq!(online.manifest.type_metadata[0].enum_members[2].value, 6);
}

#[test]
fn invalid_format_two_metadata_is_rejected() {
    // Build-time validation refuses malformed metadata before an artifact can
    // be emitted.
    let mut bad_enum = metadata_catalog();
    bad_enum.types[0].enum_members[1].value = 0;
    let error = build(&bad_enum, &input(), &BuildOptions::default())
        .expect_err("duplicate enum values are not canonical");
    assert!(error.to_string().contains("canonical order"), "{error}");

    let mut bad_subrange = metadata_catalog();
    bad_subrange.types[1].subrange = Some(SubrangeDecl { lower: 9, upper: 1 });
    let error =
        build(&bad_subrange, &input(), &BuildOptions::default()).expect_err("inverted subrange");
    assert!(error.to_string().contains("lower bound"), "{error}");

    let mut missing_aggregate = metadata_catalog();
    missing_aggregate.aggregates.clear();
    // A type declaration without an aggregate is valid: the aggregate table is
    // only a convenience for split-storage resolution. A shape pointing at a
    // type with no split roots is not.
    build(&missing_aggregate, &input(), &BuildOptions::default())
        .expect("type aggregate is optional");

    let mut bad_aggregate = metadata_catalog();
    bad_aggregate.aggregates[0].layout_hash = GotId::ZERO;
    let error = build(&bad_aggregate, &input(), &BuildOptions::default())
        .expect_err("zero aggregate layout hash");
    assert!(error.to_string().contains("zero layout hash"), "{error}");
}
