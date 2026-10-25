//! Descriptor format-1 codec, digest, canonical-order, and pairing tests.

use stc_got::catalog::ImportService;
use stc_got::descriptor::{Descriptor, DescriptorFunction, entry_flag, function_flag};
use stc_got::{
    BuildInput, BuildOptions, Catalog, Digest32, Endianness, FunctionDecl, FunctionKind, Identity,
    IdentityKind, InitializerKind, Manifest, Retention, RootDecl, RootKind, Signature, Target,
    TypeKey, build, elementary_layout, elementary_layout_hash, function_key, global_root_key,
    source_fingerprint,
};

fn target() -> Target {
    Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8)
}

fn signature() -> Signature {
    Signature::parse("(IN:DINT)->DINT").unwrap()
}

fn function(path: &str) -> FunctionDecl {
    let signature = signature();
    FunctionDecl {
        identity: Identity::new(IdentityKind::Function, "_ROOT", path, signature.to_string()),
        key: function_key("_ROOT", path, &signature).unwrap(),
        signature_key: signature.to_string(),
        function_kind: FunctionKind::Function,
        owner_type: None,
        flags: function_flag::USER | function_flag::INDIRECT_CALLABLE,
        export_name: None,
        vtable_slot: None,
        source: None,
    }
}

fn root() -> RootDecl {
    let ty = TypeKey::Dint;
    let layout = elementary_layout(&ty).unwrap();
    RootDecl {
        identity: Identity::new(IdentityKind::DataRoot, "_ROOT", "COUNTER", ""),
        key: global_root_key("_ROOT", "COUNTER").unwrap(),
        root_kind: RootKind::Global,
        owner_type: None,
        field_path: String::new(),
        type_key: ty.to_string(),
        layout_hash: elementary_layout_hash(&target(), &ty).unwrap(),
        size: layout.size,
        align: layout.align,
        retention: Retention::None,
        initializer: InitializerKind::Zero,
        initializer_digest: None,
        source: None,
    }
}

fn built() -> (Manifest, Descriptor) {
    let catalog = Catalog {
        roots: vec![root()],
        functions: vec![function("ADD"), function("SUB")],
        imports: vec![ImportService::Trap],
        ..Catalog::default()
    };
    let mut input = BuildInput::new("stc-2013-oop-utf8-v1", "stc 0.1.0", target(), "descriptor");
    input.descriptor_digest = Digest32::from_bytes([0x11; 32]);
    input.source_fingerprint =
        source_fingerprint(&[("_ROOT/main.st".into(), b"PROGRAM Main\n".to_vec())]).unwrap();
    let outcome = build(&catalog, &input, &BuildOptions::default()).expect("manifest build");
    let mut manifest = outcome.manifest;
    let mut descriptor = Descriptor::new(manifest.header.build_id);
    descriptor.profile = manifest.header.profile.clone();
    descriptor.compiler_version = manifest.header.compiler_version.clone();
    descriptor.target_triple = manifest.header.target_triple.clone();
    descriptor.entry_flags = entry_flag::PREPARE_INITIALIZES | entry_flag::CYCLE_CALLABLE;
    descriptor.functions = descriptor_functions(&manifest);
    descriptor.slots = manifest.slots.clone();
    let digest = descriptor.digest().unwrap();
    manifest.header.descriptor_digest = digest;
    descriptor.manifest_digest = manifest.digest().unwrap();
    (manifest, descriptor)
}

fn descriptor_functions(manifest: &Manifest) -> Vec<DescriptorFunction> {
    let mut functions = manifest.functions.iter().collect::<Vec<_>>();
    functions.sort_by_key(|function| function.got_id);
    functions
        .into_iter()
        .map(|function| DescriptorFunction {
            got_id: function.got_id,
            key: function.key.clone(),
            signature_key: function.signature_key.clone(),
            flags: function.flags,
        })
        .collect()
}

fn sealed() -> (Manifest, Descriptor, Vec<u8>) {
    let (manifest, descriptor) = built();
    descriptor
        .validate_against_manifest(&manifest)
        .expect("valid pair");
    let bytes = descriptor.encode().unwrap();
    (manifest, descriptor, bytes)
}

#[test]
fn descriptor_round_trips_and_pairs_with_the_manifest() {
    let (manifest, descriptor, bytes) = sealed();
    assert_eq!(Descriptor::decode(&bytes).unwrap(), descriptor);
    assert_eq!(descriptor.manifest_digest, manifest.digest().unwrap());
    assert_eq!(
        manifest.header.descriptor_digest,
        descriptor.digest().unwrap()
    );
    assert_eq!(descriptor.slots, manifest.slots);
    assert_eq!(descriptor.functions.len(), 2);
    assert_eq!(descriptor.functions.len(), manifest.functions.len());
}

#[test]
fn descriptor_digest_ignores_the_manifest_digest_field() {
    let (_, mut descriptor, _) = sealed();
    let digest = descriptor.digest().unwrap();
    descriptor.manifest_digest = Digest32::from_bytes([0xEE; 32]);
    assert_eq!(descriptor.digest().unwrap(), digest);
}

#[test]
fn tampered_descriptor_bytes_are_rejected() {
    let (_, _, mut bytes) = sealed();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(Descriptor::decode(&bytes).is_err());

    let (_, _, mut bytes) = sealed();
    bytes[12] ^= 0x01;
    assert!(Descriptor::decode(&bytes).is_err(), "descriptor_size");
}

#[test]
fn unsorted_function_records_are_rejected() {
    let (manifest, _, _) = sealed();
    let mut functions = descriptor_functions(&manifest);
    functions.reverse();
    let mut descriptor = Descriptor::new(manifest.header.build_id);
    descriptor.profile = manifest.header.profile.clone();
    descriptor.compiler_version = manifest.header.compiler_version.clone();
    descriptor.target_triple = manifest.header.target_triple.clone();
    descriptor.manifest_digest = manifest.digest().unwrap();
    descriptor.functions = functions;
    descriptor.slots = manifest.slots.clone();
    assert!(descriptor.validate().is_err());
}

#[test]
fn unsorted_or_non_contiguous_slots_are_rejected() {
    let (manifest, _, _) = sealed();
    let mut descriptor = Descriptor::new(manifest.header.build_id);
    descriptor.profile = manifest.header.profile.clone();
    descriptor.compiler_version = manifest.header.compiler_version.clone();
    descriptor.target_triple = manifest.header.target_triple.clone();
    descriptor.manifest_digest = manifest.digest().unwrap();
    descriptor.functions = descriptor_functions(&manifest);
    descriptor.slots = manifest.slots.clone();
    descriptor.slots.swap(0, 1);
    assert!(descriptor.validate().is_err());

    let mut descriptor = Descriptor::new(manifest.header.build_id);
    descriptor.profile = manifest.header.profile.clone();
    descriptor.compiler_version = manifest.header.compiler_version.clone();
    descriptor.target_triple = manifest.header.target_triple.clone();
    descriptor.manifest_digest = manifest.digest().unwrap();
    descriptor.functions = descriptor_functions(&manifest);
    descriptor.slots = manifest.slots.clone();
    descriptor.slots[0].slot = 7;
    assert!(descriptor.validate().is_err());
}

#[test]
fn pairing_rejects_a_slot_table_mismatch() {
    let (manifest, mut descriptor, _) = sealed();
    descriptor.slots.pop();
    assert!(descriptor.validate_against_manifest(&manifest).is_err());
}

#[test]
fn pairing_rejects_a_function_flag_mismatch() {
    let (manifest, mut descriptor, _) = sealed();
    descriptor.functions[0].flags = 0;
    assert!(descriptor.validate_against_manifest(&manifest).is_err());
}

#[test]
fn pairing_rejects_a_manifest_digest_mismatch() {
    let (manifest, mut descriptor, _) = sealed();
    descriptor.manifest_digest = Digest32::from_bytes([0xAA; 32]);
    assert!(descriptor.validate_against_manifest(&manifest).is_err());
}
