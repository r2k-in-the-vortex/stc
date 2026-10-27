//! Runtime descriptor registry integration tests.
//!
//! The fixtures compile the compiler's real descriptor and manifest into a
//! no-op module, so these tests exercise descriptor binding, record contents,
//! stable addresses across a warm swap, entry patching at activation, and the
//! object-record preserve/allocate/rollback paths without generated bodies.

mod common;

use stc_codegen::module::ModuleArtifacts;
use stc_got::{
    DescriptorKind, DescriptorRecord, Digest32, GotId, HOST_ABI_MAJOR, HOST_ABI_MINOR, SlotKind,
    TypeRecord,
};
use stc_runtime::{
    Candidate, CycleOutcome, Host, HostPlatform, LoadMode, LoadOutcome, ManifestStore, PROFILE,
};

use common::{Behavior, Fixture};

const OOP_V1: &str = r#"
INTERFACE IBase
METHOD Area : DINT
END_METHOD
END_INTERFACE
INTERFACE IShape EXTENDS IBase
METHOD Perimeter : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
METHOD PUBLIC Perimeter : DINT
  Perimeter := id + 1;
END_METHOD
END_CLASS

VAR_GLOBAL
  s : Shape;
  i : IShape;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// The same surface with different method bodies.
const OOP_V2: &str = r#"
INTERFACE IBase
METHOD Area : DINT
END_METHOD
END_INTERFACE
INTERFACE IShape EXTENDS IBase
METHOD Perimeter : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id + 1;
END_METHOD
METHOD PUBLIC Perimeter : DINT
  Perimeter := id + 2;
END_METHOD
END_CLASS

VAR_GLOBAL
  s : Shape;
  i : IShape;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// [`OOP_V1`] with a second instance and a new class type.
const OOP_ADDED: &str = r#"
INTERFACE IBase
METHOD Area : DINT
END_METHOD
END_INTERFACE
INTERFACE IShape EXTENDS IBase
METHOD Perimeter : DINT
END_METHOD
END_INTERFACE

CLASS Shape IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
METHOD PUBLIC Perimeter : DINT
  Perimeter := id + 1;
END_METHOD
END_CLASS

CLASS Extra
VAR PUBLIC
  x : DINT;
END_VAR
END_CLASS

VAR_GLOBAL
  s : Shape;
  i : IShape;
  t : Shape;
  e : Extra;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// A class vtable baseline for the shape-change rejection test.
const CLASS_V1: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

/// [`CLASS_V1`] with a second method, which changes the class vtable shape.
const CLASS_V2: &str = r#"
CLASS Thing
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC M : DINT
  M := id;
END_METHOD
METHOD PUBLIC N : DINT
  N := id + 1;
END_METHOD
END_CLASS

VAR_GLOBAL
  th : Thing;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

fn platform() -> HostPlatform {
    HostPlatform::new(PROFILE, common::target(), HOST_ABI_MAJOR, HOST_ABI_MINOR)
}

fn in_memory_host() -> Host {
    Host::new(platform(), ManifestStore::memory())
}

fn open(fixture: &Fixture) -> Candidate {
    Candidate::open(&platform(), &fixture.module_path, &fixture.manifest_path).expect("candidate")
}

fn fixture(name: &str, artifacts: ModuleArtifacts) -> Option<Fixture> {
    Fixture::build(name, artifacts, Behavior::default())
}

fn cold_install(host: &mut Host, fixture: &Fixture) -> u64 {
    let outcome = host
        .load(open(fixture), LoadMode::Auto)
        .expect("cold install");
    let LoadOutcome::ColdInstalled { generation } = outcome else {
        panic!("expected a cold install, got {outcome:?}");
    };
    generation
}

fn type_record<'a>(artifacts: &'a ModuleArtifacts, key: &str) -> &'a TypeRecord {
    artifacts
        .manifest
        .types
        .iter()
        .find(|ty| ty.key == key)
        .unwrap_or_else(|| panic!("type `{key}`"))
}

fn descriptor(
    artifacts: &ModuleArtifacts,
    owner: GotId,
    kind: DescriptorKind,
) -> &DescriptorRecord {
    artifacts
        .manifest
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.owner_type == Some(owner) && descriptor.descriptor_kind == kind
        })
        .unwrap_or_else(|| panic!("missing {kind:?} descriptor for {owner}"))
}

fn root_id(artifacts: &ModuleArtifacts, key: &str) -> GotId {
    artifacts
        .manifest
        .roots
        .iter()
        .find(|root| root.key == key)
        .unwrap_or_else(|| panic!("root `{key}`"))
        .got_id
}

fn slot_of(artifacts: &ModuleArtifacts, got_id: GotId) -> u32 {
    artifacts
        .manifest
        .slots
        .iter()
        .find(|slot| slot.got_id == got_id)
        .unwrap_or_else(|| panic!("slot for {got_id}"))
        .slot
}

fn function_record<'a>(artifacts: &'a ModuleArtifacts, key: &str) -> &'a stc_got::FunctionRecord {
    artifacts
        .manifest
        .functions
        .iter()
        .find(|function| function.key == key)
        .unwrap_or_else(|| panic!("function `{key}`"))
}

fn word(words: &[u64], offset: usize) -> u32 {
    let index = offset / 8;
    let shift = (offset % 8) * 8;
    u32::try_from((words[index] >> shift) & u64::from(u32::MAX)).expect("u32 field")
}

fn id_at(words: &[u64], offset: usize) -> GotId {
    let index = offset / 8;
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&words[index].to_le_bytes());
    bytes[8..].copy_from_slice(&words[index + 1].to_le_bytes());
    GotId::from_bytes(bytes)
}

fn pointer_at(words: &[u64], offset: usize) -> usize {
    usize::try_from(words[offset / 8]).expect("pointer word")
}

fn write_pair(host: &mut Host, root: GotId, record: usize, vtable: usize) {
    let storage = host.root_storage_mut(root).expect("interface root");
    assert_eq!(storage.len(), 16);
    storage[..8].copy_from_slice(&u64::try_from(record).unwrap().to_le_bytes());
    storage[8..].copy_from_slice(&u64::try_from(vtable).unwrap().to_le_bytes());
}

/// Re-seals a mutated manifest and descriptor pair for a test fixture.
///
/// Test mutations deliberately bypass the compiler; resealing keeps the host
/// validator focused on the registry checks under test.
fn reseal(artifacts: &mut ModuleArtifacts) {
    artifacts.manifest.header.catalog_digest =
        artifacts.manifest.catalog_digest().expect("catalog");
    artifacts.manifest.header.plan_digest = artifacts.manifest.plan_digest().expect("plan");
    artifacts.descriptor.manifest_digest = Digest32::ZERO;
    artifacts.manifest.header.descriptor_digest =
        artifacts.descriptor.digest().expect("descriptor");
    artifacts.descriptor.manifest_digest = artifacts.manifest.digest().expect("manifest");
    artifacts.descriptor_digest = artifacts.descriptor.digest().expect("descriptor");
    artifacts.manifest_digest = artifacts.manifest.digest().expect("manifest");
    artifacts.descriptor_bytes = artifacts.descriptor.encode().expect("descriptor bytes");
    artifacts.manifest_binary = artifacts.manifest.encode_binary().expect("manifest bytes");
}

const ARRAY_LEAF_OBJECTS: &str = r#"
INTERFACE IItem
METHOD Value : DINT
END_METHOD
END_INTERFACE

CLASS Item IMPLEMENTS IItem
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Value : DINT
  Value := id;
END_METHOD
END_CLASS

CLASS Bucket
VAR PUBLIC
  one : IItem;
  many : ARRAY[1..2] OF IItem;
  handles : ARRAY[1..2] OF Item;
  count : DINT;
END_VAR
END_CLASS

VAR_GLOBAL
  b : Bucket;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

#[test]
fn cold_install_binds_descriptor_cells_and_builds_records() {
    let artifacts = common::artifacts(OOP_V1, "oop");
    let Some(fixture) = fixture("oop-cold", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);

    for descriptor in &artifacts.manifest.descriptors {
        let slot = slot_of(&artifacts, descriptor.got_id);
        assert_eq!(
            artifacts.manifest.slots[slot as usize].kind,
            SlotKind::Descriptor
        );
        assert_eq!(
            host.active_cell_value(slot),
            host.descriptor_address(descriptor.got_id),
            "descriptor `{}` cell",
            descriptor.key
        );
    }

    let shape = type_record(&artifacts, "_ROOT::SHAPE");
    let typeinfo = descriptor(&artifacts, shape.got_id, DescriptorKind::TypeInfo);
    let vtable = descriptor(&artifacts, shape.got_id, DescriptorKind::Vtable);
    let words = host.descriptor_words(typeinfo.got_id).expect("typeinfo");
    assert_eq!(word(words, 0), 1, "record format");
    assert_eq!(word(words, 4), 1, "class kind");
    assert_eq!(word(words, 8), 0, "flags");
    assert_eq!(word(words, 12), 1, "Shape has one leaf");
    assert_eq!(word(words, 16), 0, "no base");
    assert_eq!(word(words, 20), 2, "IBase and IShape");
    assert_eq!(word(words, 24), 2, "two class method slots");
    assert_eq!(word(words, 28), 0, "reserved");
    assert_eq!(id_at(words, 32), shape.got_id);
    assert_eq!(id_at(words, 48), shape.layout_hash);
    assert_eq!(
        pointer_at(words, 64),
        host.descriptor_address(vtable.got_id).expect("vtable"),
        "TYPEINFO class_vtable"
    );
    let base = type_record(&artifacts, "_ROOT::IBASE");
    let ishape = type_record(&artifacts, "_ROOT::ISHAPE");
    assert_eq!(id_at(words, 72), base.got_id);
    assert_eq!(id_at(words, 96), ishape.got_id);
    assert_eq!(
        pointer_at(words, 88),
        host.interface_vtable_address(shape.got_id, base.got_id)
            .expect("IBase table")
    );
    assert_eq!(
        pointer_at(words, 112),
        host.interface_vtable_address(shape.got_id, ishape.got_id)
            .expect("IShape table")
    );

    let class_words = host.descriptor_words(vtable.got_id).expect("vtable");
    assert_eq!(word(class_words, 0), 1);
    assert_eq!(word(class_words, 4), 1, "class table");
    assert_eq!(word(class_words, 8), 2);
    assert_eq!(id_at(class_words, 16), shape.got_id);
    assert_eq!(id_at(class_words, 32), GotId::ZERO);
    let area = function_record(&artifacts, "_ROOT::SHAPE.AREA@()->DINT");
    let perimeter = function_record(&artifacts, "_ROOT::SHAPE.PERIMETER@()->DINT");
    assert_eq!(
        pointer_at(class_words, 48 + area.vtable_slot.unwrap() as usize * 8),
        host.active_cell_address(slot_of(&artifacts, area.got_id))
            .expect("Area cell")
    );
    assert_eq!(
        pointer_at(
            class_words,
            48 + perimeter.vtable_slot.unwrap() as usize * 8
        ),
        host.active_cell_address(slot_of(&artifacts, perimeter.got_id))
            .expect("Perimeter cell")
    );

    let pair = host
        .interface_vtable_words(shape.got_id, ishape.got_id)
        .expect("IShape pair");
    assert_eq!(word(pair, 0), 1);
    assert_eq!(word(pair, 4), 2, "interface table");
    assert_eq!(word(pair, 8), 2);
    assert_eq!(id_at(pair, 16), shape.got_id);
    assert_eq!(id_at(pair, 32), ishape.got_id);
    assert_eq!(
        pointer_at(pair, 48),
        host.active_cell_address(slot_of(&artifacts, area.got_id))
            .expect("Area cell")
    );
    assert_eq!(
        pointer_at(pair, 56),
        host.active_cell_address(slot_of(&artifacts, perimeter.got_id))
            .expect("Perimeter cell")
    );
}

#[test]
fn object_records_count_single_root_array_fields_once() {
    let artifacts = common::artifacts(ARRAY_LEAF_OBJECTS, "array-leaves");
    let Some(fixture) = fixture("array-leaves", artifacts.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &fixture);

    // Interface values and arrays whose element chain ends in a single-root
    // type are one leaf each, matching the compiler's storage plan (D-131).
    let bucket = type_record(&artifacts, "_ROOT::BUCKET");
    let typeinfo = descriptor(&artifacts, bucket.got_id, DescriptorKind::TypeInfo);
    let words = host.descriptor_words(typeinfo.got_id).expect("typeinfo");
    assert_eq!(
        word(words, 12),
        4,
        "one interface, one interface array, one handle array, one scalar"
    );
    let record_root = root_id(&artifacts, "_ROOT::B#OBJREC");
    let storage = host.root_storage(record_root).expect("object record");
    assert_eq!(storage.len(), (4 + 1) * 8, "TYPEINFO plus four leaves");
}

#[test]
fn warm_swap_keeps_descriptor_addresses_and_patches_interface_entries() {
    let v1 = common::artifacts(OOP_V1, "oop");
    let Some(v1_fixture) = fixture("oop-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);

    let shape = type_record(&v1, "_ROOT::SHAPE");
    let typeinfo = descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo);
    let vtable = descriptor(&v1, shape.got_id, DescriptorKind::Vtable);
    let ishape = type_record(&v1, "_ROOT::ISHAPE");
    let interface_root = root_id(&v1, "_ROOT::I");
    let record_root = root_id(&v1, "_ROOT::S#OBJREC");
    let record_address = host.roots().address(record_root).expect("object record");
    let pair_address = host
        .interface_vtable_address(shape.got_id, ishape.got_id)
        .expect("pair table");
    write_pair(&mut host, interface_root, record_address, pair_address);

    let typeinfo_address = host.descriptor_address(typeinfo.got_id).expect("typeinfo");
    let vtable_address = host.descriptor_address(vtable.got_id).expect("vtable");
    let pair_words = host
        .interface_vtable_words(shape.got_id, ishape.got_id)
        .expect("pair")
        .to_vec();
    let root_before = host
        .root_storage(interface_root)
        .expect("interface")
        .to_vec();
    let area = function_record(&v1, "_ROOT::SHAPE.AREA@()->DINT");
    let perimeter = function_record(&v1, "_ROOT::SHAPE.PERIMETER@()->DINT");
    let old_cells = [
        host.active_cell_address(slot_of(&v1, area.got_id))
            .expect("Area v1 cell"),
        host.active_cell_address(slot_of(&v1, perimeter.got_id))
            .expect("Perimeter v1 cell"),
    ];
    assert_eq!(pointer_at(&pair_words, 48), old_cells[0]);
    assert_eq!(pointer_at(&pair_words, 56), old_cells[1]);
    let area_slot = area.vtable_slot.expect("Area slot") as usize;
    let perimeter_slot = perimeter.vtable_slot.expect("Perimeter slot") as usize;
    let class_before = host
        .descriptor_words(vtable.got_id)
        .expect("vtable")
        .to_vec();
    assert_eq!(pointer_at(&class_before, 48 + area_slot * 8), old_cells[0]);
    assert_eq!(
        pointer_at(&class_before, 48 + perimeter_slot * 8),
        old_cells[1]
    );

    let v2 = common::artifacts_with(OOP_V2, "oop", v1.manifest.clone(), false);
    assert!(v2.is_deployable());
    let Some(v2_fixture) = fixture("oop-v2", v2.clone()) else {
        return;
    };
    let outcome = host
        .load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");
    assert!(matches!(outcome, LoadOutcome::Prepared { .. }));

    // Prepare builds shadow contents but does not patch the live records.
    assert_eq!(
        host.descriptor_address(typeinfo.got_id),
        Some(typeinfo_address)
    );
    assert_eq!(host.descriptor_address(vtable.got_id), Some(vtable_address));
    assert_eq!(
        host.interface_vtable_address(shape.got_id, ishape.got_id),
        Some(pair_address)
    );
    assert_eq!(
        host.interface_vtable_words(shape.got_id, ishape.got_id)
            .expect("pair"),
        pair_words.as_slice(),
        "entries stay on the active generation until activation"
    );
    assert_eq!(
        host.descriptor_words(vtable.got_id).expect("vtable"),
        class_before.as_slice(),
        "class entries stay on the active generation until activation"
    );

    host.activate().expect("activate");
    assert_eq!(
        host.descriptor_address(typeinfo.got_id),
        Some(typeinfo_address)
    );
    assert_eq!(host.descriptor_address(vtable.got_id), Some(vtable_address));
    assert_eq!(
        host.interface_vtable_address(shape.got_id, ishape.got_id),
        Some(pair_address),
        "preserved interface values keep their table address"
    );
    let patched = host
        .interface_vtable_words(shape.got_id, ishape.got_id)
        .expect("pair");
    let new_area = function_record(&v2, "_ROOT::SHAPE.AREA@()->DINT");
    let new_perimeter = function_record(&v2, "_ROOT::SHAPE.PERIMETER@()->DINT");
    let new_cells = [
        host.active_cell_address(slot_of(&v2, new_area.got_id))
            .expect("Area v2 cell"),
        host.active_cell_address(slot_of(&v2, new_perimeter.got_id))
            .expect("Perimeter v2 cell"),
    ];
    assert_eq!(pointer_at(patched, 48), new_cells[0]);
    assert_eq!(pointer_at(patched, 56), new_cells[1]);
    let class_patched = host.descriptor_words(vtable.got_id).expect("vtable");
    assert_eq!(
        pointer_at(class_patched, 48 + area_slot * 8),
        new_cells[0],
        "the class table is patched at activation"
    );
    assert_eq!(
        pointer_at(class_patched, 48 + perimeter_slot * 8),
        new_cells[1]
    );
    let typeinfo_after = host.descriptor_words(typeinfo.got_id).expect("typeinfo");
    assert_eq!(
        pointer_at(typeinfo_after, 64),
        vtable_address,
        "TYPEINFO keeps pointing at the stable class VTABLE"
    );
    assert_ne!(new_cells, old_cells, "a new generation has new host cells");
    assert_eq!(
        host.root_storage(interface_root).expect("interface"),
        root_before.as_slice(),
        "the preserved interface value is untouched"
    );

    let outcome = host.cycle().expect("commit cycle");
    assert!(matches!(outcome, CycleOutcome::Committed { .. }));
    assert_eq!(
        host.descriptor_address(typeinfo.got_id),
        Some(typeinfo_address)
    );
    assert_eq!(
        host.interface_vtable_address(shape.got_id, ishape.got_id),
        Some(pair_address)
    );
}

#[test]
fn object_records_and_new_descriptors_allocate_and_roll_back() {
    let v1 = common::artifacts(OOP_V1, "oop");
    let Some(v1_fixture) = fixture("oop-rec-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);

    let shape = type_record(&v1, "_ROOT::SHAPE");
    let shape_typeinfo = descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo);
    let shape_typeinfo_address = host
        .descriptor_address(shape_typeinfo.got_id)
        .expect("Shape typeinfo");
    let record_root = root_id(&v1, "_ROOT::S#OBJREC");
    let record_address = host.roots().address(record_root).expect("object record");
    assert!(host.roots().contains(record_root));

    let v2 = common::artifacts_with(OOP_ADDED, "oop", v1.manifest.clone(), false);
    assert!(v2.is_deployable());
    assert_eq!(v2.verdict(), stc_got::PlanVerdict::OnlineCandidate);
    let Some(v2_fixture) = fixture("oop-rec-v2", v2.clone()) else {
        return;
    };
    host.load(open(&v2_fixture), LoadMode::Auto)
        .expect("prepare");

    // The preserved instance keeps its record; the new instance gets fresh,
    // zeroed hidden roots.
    let added_record = root_id(&v2, "_ROOT::T#OBJREC");
    let added_handle = root_id(&v2, "_ROOT::T#HANDLE");
    assert_eq!(host.roots().address(record_root), Some(record_address));
    assert!(host.roots().contains(added_record));
    assert!(host.roots().contains(added_handle));
    assert_eq!(
        host.root_storage(added_record).expect("added record"),
        vec![0; 16].as_slice()
    );
    assert_eq!(
        host.root_storage(added_handle).expect("added handle"),
        vec![0; 8].as_slice()
    );

    let extra = type_record(&v2, "_ROOT::EXTRA");
    let extra_typeinfo = descriptor(&v2, extra.got_id, DescriptorKind::TypeInfo);
    assert!(
        host.descriptor_address(extra_typeinfo.got_id).is_some(),
        "a new type is prepared"
    );
    assert_eq!(
        host.descriptor_address(shape_typeinfo.got_id),
        Some(shape_typeinfo_address),
        "a preserved type keeps its descriptor address"
    );

    host.activate().expect("activate");
    assert_eq!(host.roots().address(record_root), Some(record_address));
    host.rollback().expect("rollback");

    // Fresh roots and descriptors are released; the preserved record and
    // descriptor survive at their original addresses.
    assert_eq!(host.roots().address(record_root), Some(record_address));
    assert!(!host.roots().contains(added_record));
    assert!(!host.roots().contains(added_handle));
    assert!(host.descriptor_address(extra_typeinfo.got_id).is_none());
    assert_eq!(
        host.descriptor_address(shape_typeinfo.got_id),
        Some(shape_typeinfo_address)
    );
    assert_eq!(host.quarantined_descriptors(), 0);
}

#[test]
fn malformed_descriptor_metadata_is_rejected_before_prepare() {
    let mut artifacts = common::artifacts(OOP_V1, "oop");
    let shape = artifacts
        .manifest
        .types
        .iter()
        .find(|ty| ty.key == "_ROOT::SHAPE")
        .expect("Shape")
        .got_id;
    let typeinfo = artifacts
        .manifest
        .descriptors
        .iter_mut()
        .find(|descriptor| {
            descriptor.owner_type == Some(shape)
                && descriptor.descriptor_kind == DescriptorKind::TypeInfo
        })
        .expect("Shape TYPEINFO");
    typeinfo.size += 8;
    reseal(&mut artifacts);
    let Some(fixture) = fixture("oop-malformed", artifacts) else {
        return;
    };
    let mut host = in_memory_host();
    let error = host
        .load(open(&fixture), LoadMode::Cold)
        .expect_err("malformed descriptor");
    assert_eq!(error.code, stc_got::codes::MODULE_INVALID, "{error}");
    assert_eq!(host.active_generation(), None);
    assert!(host.roots().is_empty());
}

#[test]
fn vtable_shape_changes_are_rejected_against_a_live_instance() {
    let v1 = common::artifacts(CLASS_V1, "shape");
    let Some(v1_fixture) = fixture("shape-v1", v1.clone()) else {
        return;
    };
    let mut host = in_memory_host();
    cold_install(&mut host, &v1_fixture);
    let shape = type_record(&v1, "_ROOT::THING");

    let broken = common::artifacts_with(CLASS_V2, "shape", v1.manifest.clone(), false);
    assert_eq!(broken.verdict(), stc_got::PlanVerdict::Rejected);
    let Some(broken_fixture) = fixture("shape-broken", broken) else {
        return;
    };
    let error = host
        .load(open(&broken_fixture), LoadMode::Auto)
        .expect_err("a vtable change is rejected");
    assert!(
        matches!(
            error.code,
            stc_got::codes::ONLINE_VTABLE | stc_got::codes::MANIFEST_PLAN
        ),
        "{error}"
    );
    assert_eq!(host.active_build_id(), Some(v1.manifest.header.build_id));
    assert!(
        host.descriptor_address(descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo).got_id)
            .is_some()
    );
}

#[test]
fn feature_bits_reject_oop_modules_on_older_hosts() {
    let artifacts = common::artifacts(OOP_V1, "oop");
    let without_descriptors = platform().with_supported_features(
        stc_runtime::SUPPORTED_FEATURES & !stc_got::descriptor::feature::DESCRIPTORS,
    );
    let error = stc_runtime::ValidatedPair::from_bytes(
        &without_descriptors,
        &artifacts.descriptor_bytes,
        &artifacts.manifest_binary,
    )
    .expect_err("a host without descriptors rejects the module");
    assert_eq!(error.code, stc_got::codes::MODULE_INVALID);

    let abi_20 = HostPlatform::new(PROFILE, common::target(), HOST_ABI_MAJOR, 0);
    let error = stc_runtime::ValidatedPair::from_bytes(
        &abi_20,
        &artifacts.descriptor_bytes,
        &artifacts.manifest_binary,
    )
    .expect_err("a 2.0 host rejects an ABI 2.1 module");
    assert_eq!(error.code, stc_got::codes::ABI_MISMATCH);
}

#[test]
fn a_missing_descriptor_is_rejected_before_prepare() {
    let mut artifacts = common::artifacts(OOP_V1, "oop");
    let shape = artifacts
        .manifest
        .types
        .iter()
        .find(|ty| ty.key == "_ROOT::SHAPE")
        .expect("Shape")
        .got_id;
    let typeinfo = artifacts
        .manifest
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.owner_type == Some(shape)
                && descriptor.descriptor_kind == DescriptorKind::TypeInfo
        })
        .expect("Shape TYPEINFO")
        .got_id;
    let index = artifacts
        .manifest
        .slots
        .iter()
        .position(|slot| slot.kind == SlotKind::Descriptor && slot.got_id == typeinfo)
        .expect("descriptor slot");
    for table in [
        &mut artifacts.manifest.slots,
        &mut artifacts.descriptor.slots,
    ] {
        table.remove(index);
        for (slot, record) in table.iter_mut().enumerate() {
            record.slot = u32::try_from(slot).expect("slot index");
        }
    }
    artifacts
        .manifest
        .descriptors
        .retain(|descriptor| descriptor.got_id != typeinfo);
    artifacts
        .manifest
        .plan
        .actions
        .retain(|action| action.got_id != typeinfo);
    artifacts
        .manifest
        .plan
        .diagnostics
        .retain(|diagnostic| diagnostic.got_id != Some(typeinfo));
    reseal(&mut artifacts);

    let Some(fixture) = fixture("oop-missing-descriptor", artifacts) else {
        return;
    };
    let mut host = in_memory_host();
    let error = host
        .load(open(&fixture), LoadMode::Cold)
        .expect_err("a missing descriptor is rejected");
    assert_eq!(error.code, stc_got::codes::MODULE_INVALID, "{error}");
    assert_eq!(host.active_generation(), None);
}
