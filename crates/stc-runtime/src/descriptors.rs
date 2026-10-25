//! Host-owned TYPEINFO, VTABLE, and per-(type, interface) VTABLE records.
//!
//! GOT Artifacts §8.5 fixes the byte layouts generated OOP code reads inline;
//! Runtime and Execution §2.6 fixes their lifecycle. This module builds those
//! records from the manifest's type and function records during prepare, keeps
//! a stable address for every compatible shape across an online change, and
//! patches entries and interface maps at activation.
//!
//! The registry is keyed by catalog descriptor `GotId` for TYPEINFO and class
//! VTABLE records. Per-(object type, interface) tables have no catalog
//! identity (D-148), so they are keyed by the two participating type
//! identities instead. Every record is a `Box<[u64]>`: its address is stable
//! for the life of the registry entry, and the words are host-owned and never
//! read or written by generated code.
//!
//! A candidate prepares shadow contents without touching a preserved record.
//! Activation copies the shadow words into the stable record, and rollback
//! restores the previous words and releases fresh records. Records that a
//! candidate removes are quarantined, like roots, so a preserved interface
//! value that still points at one remains addressable.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::descriptor::function_flag;
use stc_got::error::{Error, Result, codes};
use stc_got::{
    DescriptorKind, DescriptorRecord, FunctionKind, FunctionRecord, GotId, Manifest, SlotKind,
    TypeKey, TypeKind, TypeRecord, hidden_root_marker,
};

/// Runtime record format word (GOT Artifacts §8.5).
pub const RECORD_FORMAT: u32 = 1;

/// TYPEINFO `kind` values (GOT Artifacts §8.5.2).
pub mod typeinfo_kind {
    /// A class object.
    pub const CLASS: u32 = 1;
    /// A function-block object.
    pub const FUNCTION_BLOCK: u32 = 2;
}

/// VTABLE `kind` values (GOT Artifacts §8.5.3).
pub mod vtable_kind {
    /// A class method table.
    pub const CLASS: u32 = 1;
    /// A per-(object type, interface) method table.
    pub const INTERFACE: u32 = 2;
}

/// Fixed TYPEINFO header size in bytes.
const TYPEINFO_HEADER: usize = 72;
/// Fixed VTABLE header size in bytes.
const VTABLE_HEADER: usize = 48;
/// Pointer-sized record word.
const WORD_BYTES: usize = 8;

/// The identity of one host-owned runtime record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RecordKey {
    /// A TYPEINFO or class VTABLE descriptor with a catalog identity.
    Descriptor(GotId),
    /// A per-(object type, interface) VTABLE with no catalog identity.
    InterfaceVtable {
        /// Implementing class or function block.
        owner: GotId,
        /// Interface type.
        interface: GotId,
    },
}

/// The compatibility shape of one runtime record.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordShape {
    /// A TYPEINFO record.
    TypeInfo(TypeInfoShape),
    /// A class method VTABLE.
    ClassVtable(VtableShape),
    /// A per-(object type, interface) VTABLE.
    InterfaceVtable(PairShape),
}

impl RecordShape {
    fn owner(&self) -> GotId {
        match self {
            Self::TypeInfo(shape) => shape.owner,
            Self::ClassVtable(shape) => shape.owner,
            Self::InterfaceVtable(shape) => shape.owner,
        }
    }
}

/// The shape of one TYPEINFO record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TypeInfoShape {
    owner: GotId,
    type_key: String,
    layout_hash: GotId,
    size: u64,
    align: u32,
    kind: u32,
    leaf_count: u32,
    base_chain: Vec<GotId>,
    interfaces: Vec<GotId>,
    vtable_slot_count: u32,
    vtable_shape: Option<GotId>,
}

/// The shape of one class VTABLE record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VtableShape {
    owner: GotId,
    type_key: String,
    layout_hash: GotId,
    size: u64,
    align: u32,
    entry_count: u32,
    vtable_shape: Option<GotId>,
}

/// The shape of one per-(object type, interface) VTABLE record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PairShape {
    owner: GotId,
    interface: GotId,
    entry_count: u32,
    interface_vtable_shape: Option<GotId>,
    interface_layout_hash: GotId,
}

/// One active registry record.
#[derive(Debug)]
struct RecordEntry {
    shape: RecordShape,
    /// Stable record words; the allocation never moves.
    storage: Box<[u64]>,
}

impl RecordEntry {
    fn address(&self) -> usize {
        self.storage.as_ptr() as usize
    }
}

/// One quarantined registry record.
#[derive(Debug)]
struct QuarantinedRecord {
    key: RecordKey,
    entry: RecordEntry,
}

/// One descriptor record a candidate wants the host to own.
struct CandidateRecord {
    key: RecordKey,
    shape: RecordShape,
    content: CandidateContent,
}

/// The semantic content of one candidate record.
enum CandidateContent {
    TypeInfo {
        owner: GotId,
        kind: u32,
        leaf_count: u32,
        base_chain: Vec<GotId>,
        interfaces: Vec<(GotId, RecordKey)>,
        class_vtable: Option<RecordKey>,
    },
    ClassVtable {
        owner: GotId,
        entries: Vec<Option<GotId>>,
    },
    InterfaceVtable {
        owner: GotId,
        interface: GotId,
        entries: Vec<GotId>,
    },
}

impl CandidateRecord {
    fn words_len(&self) -> usize {
        match &self.content {
            CandidateContent::TypeInfo {
                base_chain,
                interfaces,
                ..
            } => (TYPEINFO_HEADER / WORD_BYTES) + base_chain.len() * 2 + interfaces.len() * 3,
            CandidateContent::ClassVtable { entries, .. } => {
                (VTABLE_HEADER / WORD_BYTES) + entries.len()
            }
            CandidateContent::InterfaceVtable { entries, .. } => {
                (VTABLE_HEADER / WORD_BYTES) + entries.len()
            }
        }
    }

    fn render(
        &self,
        addresses: &BTreeMap<RecordKey, usize>,
        function_cells: &BTreeMap<GotId, usize>,
    ) -> Result<Vec<u64>> {
        let mut words = vec![0_u64; self.words_len()];
        match &self.content {
            CandidateContent::TypeInfo {
                owner,
                kind,
                leaf_count,
                base_chain,
                interfaces,
                class_vtable,
            } => {
                let base_count = u32::try_from(base_chain.len())
                    .map_err(|_| invalid("TYPEINFO base chain exceeds u32"))?;
                let interface_count = u32::try_from(interfaces.len())
                    .map_err(|_| invalid("TYPEINFO interface map exceeds u32"))?;
                let slot_count = match &self.shape {
                    RecordShape::TypeInfo(shape) => shape.vtable_slot_count,
                    _ => unreachable!("candidate kind matches its shape"),
                };
                let shape = match &self.shape {
                    RecordShape::TypeInfo(shape) => shape,
                    _ => unreachable!("candidate kind matches its shape"),
                };
                set_u32(&mut words, 0, RECORD_FORMAT);
                set_u32(&mut words, 4, *kind);
                set_u32(&mut words, 8, 0);
                set_u32(&mut words, 12, *leaf_count);
                set_u32(&mut words, 16, base_count);
                set_u32(&mut words, 20, interface_count);
                set_u32(&mut words, 24, slot_count);
                set_u32(&mut words, 28, 0);
                set_id(&mut words, 32, *owner);
                set_id(&mut words, 48, shape.layout_hash);
                if let Some(key) = class_vtable {
                    let address = *addresses
                        .get(key)
                        .ok_or_else(|| invalid("TYPEINFO class VTABLE has no record"))?;
                    set_u64(
                        &mut words,
                        64,
                        u64::try_from(address)
                            .map_err(|_| invalid("record address exceeds u64"))?,
                    );
                }
                let mut offset = TYPEINFO_HEADER;
                for base in base_chain {
                    set_id(&mut words, offset, *base);
                    offset += 16;
                }
                for (interface, key) in interfaces {
                    let address = *addresses
                        .get(key)
                        .ok_or_else(|| invalid("TYPEINFO interface table has no record"))?;
                    set_id(&mut words, offset, *interface);
                    set_u64(
                        &mut words,
                        offset + 16,
                        u64::try_from(address)
                            .map_err(|_| invalid("record address exceeds u64"))?,
                    );
                    offset += 24;
                }
            }
            CandidateContent::ClassVtable { owner, entries } => {
                let count = u32::try_from(entries.len())
                    .map_err(|_| invalid("class VTABLE exceeds u32 entries"))?;
                set_u32(&mut words, 0, RECORD_FORMAT);
                set_u32(&mut words, 4, vtable_kind::CLASS);
                set_u32(&mut words, 8, count);
                set_u32(&mut words, 12, 0);
                set_id(&mut words, 16, *owner);
                for (index, function) in entries.iter().enumerate() {
                    let address = match function {
                        Some(got_id) => *function_cells.get(got_id).ok_or_else(|| {
                            binding(format!("VTABLE entry for {got_id} has no function cell"))
                        })?,
                        None => 0,
                    };
                    set_u64(
                        &mut words,
                        VTABLE_HEADER + index * WORD_BYTES,
                        u64::try_from(address)
                            .map_err(|_| invalid("function cell address exceeds u64"))?,
                    );
                }
            }
            CandidateContent::InterfaceVtable {
                owner,
                interface,
                entries,
            } => {
                let count = u32::try_from(entries.len())
                    .map_err(|_| invalid("interface VTABLE exceeds u32 entries"))?;
                set_u32(&mut words, 0, RECORD_FORMAT);
                set_u32(&mut words, 4, vtable_kind::INTERFACE);
                set_u32(&mut words, 8, count);
                set_u32(&mut words, 12, 0);
                set_id(&mut words, 16, *owner);
                set_id(&mut words, 32, *interface);
                for (index, function) in entries.iter().enumerate() {
                    let address = *function_cells.get(function).ok_or_else(|| {
                        binding(format!("interface VTABLE entry for {function} has no cell"))
                    })?;
                    set_u64(
                        &mut words,
                        VTABLE_HEADER + index * WORD_BYTES,
                        u64::try_from(address)
                            .map_err(|_| invalid("function cell address exceeds u64"))?,
                    );
                }
            }
        }
        Ok(words)
    }
}

/// One record whose shadow contents a candidate built.
#[derive(Debug)]
struct PreparedRecord {
    key: RecordKey,
    words: Vec<u64>,
}

/// The descriptor work a prepared candidate needs at activation.
#[derive(Debug)]
pub(crate) struct PreparedDescriptors {
    bindings: BTreeMap<GotId, usize>,
    records: Vec<PreparedRecord>,
    fresh: Vec<RecordKey>,
    quarantined: Vec<RecordKey>,
}

impl PreparedDescriptors {
    /// Returns the descriptor-cell bindings for the candidate GOT.
    pub(crate) fn bindings(&self) -> &BTreeMap<GotId, usize> {
        &self.bindings
    }

    /// Returns the number of records the candidate created.
    #[cfg(test)]
    pub(crate) fn fresh_count(&self) -> usize {
        self.fresh.len()
    }
}

/// The descriptor state captured by activation, for rollback.
pub(crate) struct DescriptorActivation {
    previous: Vec<(RecordKey, Vec<u64>)>,
    prepared: PreparedDescriptors,
}

/// Host-owned runtime descriptor records and their quarantine.
#[derive(Debug, Default)]
pub(crate) struct DescriptorRegistry {
    records: BTreeMap<RecordKey, RecordEntry>,
    quarantined: Vec<QuarantinedRecord>,
}

impl DescriptorRegistry {
    /// Creates an empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Builds and validates the candidate's record set.
    ///
    /// Compatible shapes keep their active record address. A shape change or
    /// removal is rejected while a preserved root can depend on the record;
    /// otherwise the old record is quarantined and a fresh allocation is used.
    /// Fresh records are inserted immediately so descriptor cells can be
    /// bound, but their contents are only written at activation.
    pub(crate) fn prepare(
        &mut self,
        manifest: &Manifest,
        live: &BTreeSet<GotId>,
        function_cells: &BTreeMap<GotId, usize>,
    ) -> Result<PreparedDescriptors> {
        let candidate = build_candidate_records(manifest)?;
        let live = self.expand_live(live);
        for got_id in &live {
            if !candidate.type_ids.contains(got_id) {
                return Err(shape_error(format!(
                    "type {got_id} was removed while a live object or interface value depends on it"
                )));
            }
        }

        let candidate_keys: BTreeSet<RecordKey> =
            candidate.records.iter().map(|record| record.key).collect();
        let mut addresses = BTreeMap::new();
        let mut fresh = Vec::new();
        let mut quarantined = Vec::new();
        for record in &candidate.records {
            match self.records.get(&record.key) {
                Some(entry) if entry.shape == record.shape => {
                    addresses.insert(record.key, entry.address());
                }
                Some(_) => {
                    if self.key_is_live(record.key, &live) {
                        return Err(self.shape_change_error(record.key));
                    }
                    fresh.push(record.key);
                    quarantined.push(record.key);
                }
                None => fresh.push(record.key),
            }
        }
        for key in self.records.keys() {
            if candidate_keys.contains(key) {
                continue;
            }
            if self.key_is_live(*key, &live) {
                return Err(self.shape_change_error(*key));
            }
            quarantined.push(*key);
        }

        for key in &quarantined {
            let Some(entry) = self.records.remove(key) else {
                continue;
            };
            self.quarantined
                .push(QuarantinedRecord { key: *key, entry });
        }
        for record in &candidate.records {
            if !fresh.contains(&record.key) {
                continue;
            }
            let storage = vec![0_u64; record.words_len()].into_boxed_slice();
            let entry = RecordEntry {
                shape: record.shape.clone(),
                storage,
            };
            addresses.insert(record.key, entry.address());
            self.records.insert(record.key, entry);
        }

        let mut records = Vec::with_capacity(candidate.records.len());
        let mut bindings = BTreeMap::new();
        for record in &candidate.records {
            let words = match record.render(&addresses, function_cells) {
                Ok(words) => words,
                Err(error) => {
                    self.revert_lists(&fresh, &quarantined);
                    return Err(error);
                }
            };
            if let RecordKey::Descriptor(got_id) = record.key {
                let address = addresses[&record.key];
                bindings.insert(got_id, address);
            }
            records.push(PreparedRecord {
                key: record.key,
                words,
            });
        }
        Ok(PreparedDescriptors {
            bindings,
            records,
            fresh,
            quarantined,
        })
    }

    /// Writes the candidate's shadow contents into the stable records.
    pub(crate) fn activate(&mut self, prepared: PreparedDescriptors) -> DescriptorActivation {
        let mut previous = Vec::with_capacity(prepared.records.len());
        for record in &prepared.records {
            let Some(entry) = self.records.get_mut(&record.key) else {
                continue;
            };
            previous.push((record.key, entry.storage.to_vec()));
            let count = entry.storage.len().min(record.words.len());
            entry.storage[..count].copy_from_slice(&record.words[..count]);
        }
        DescriptorActivation { previous, prepared }
    }

    /// Restores the pre-activation contents and undoes the prepare changes.
    pub(crate) fn rollback(&mut self, activation: DescriptorActivation) {
        for (key, words) in &activation.previous {
            if let Some(entry) = self.records.get_mut(key) {
                let count = entry.storage.len().min(words.len());
                entry.storage[..count].copy_from_slice(&words[..count]);
            }
        }
        self.revert_prepare(activation.prepared);
    }

    /// Commits a prepared candidate; quarantined records stay quarantined.
    pub(crate) fn commit(&mut self, activation: DescriptorActivation) {
        drop(activation);
    }

    /// Releases a prepared candidate that never activated.
    pub(crate) fn revert_prepare(&mut self, prepared: PreparedDescriptors) {
        self.revert_lists(&prepared.fresh, &prepared.quarantined);
    }

    fn revert_lists(&mut self, fresh: &[RecordKey], quarantined: &[RecordKey]) {
        for key in fresh {
            self.records.remove(key);
        }
        for key in quarantined {
            // Quarantine entries are appended in order, so the most recent
            // entry for this key is the one this prepare moved.
            let Some(index) = self
                .quarantined
                .iter()
                .rposition(|record| record.key == *key)
            else {
                continue;
            };
            let record = self.quarantined.remove(index);
            self.records.insert(record.key, record.entry);
        }
    }

    fn expand_live(&self, live: &BTreeSet<GotId>) -> BTreeSet<GotId> {
        let mut live = live.clone();
        for key in self.records.keys() {
            if let RecordKey::InterfaceVtable { owner, interface } = key
                && live.contains(interface)
            {
                live.insert(*owner);
            }
        }
        loop {
            let mut changed = false;
            for entry in self.records.values() {
                if let RecordShape::TypeInfo(shape) = &entry.shape
                    && shape.base_chain.iter().any(|base| live.contains(base))
                    && live.insert(shape.owner)
                {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        live
    }

    fn key_is_live(&self, key: RecordKey, live: &BTreeSet<GotId>) -> bool {
        match key {
            RecordKey::Descriptor(got_id) => self
                .records
                .get(&RecordKey::Descriptor(got_id))
                .is_some_and(|entry| live.contains(&entry.shape.owner())),
            RecordKey::InterfaceVtable { owner, interface } => {
                live.contains(&owner) || live.contains(&interface)
            }
        }
    }

    fn shape_change_error(&self, key: RecordKey) -> Error {
        match key {
            RecordKey::Descriptor(got_id) => shape_error(format!(
                "descriptor {got_id} changed shape while a live object or interface value depends on it"
            )),
            RecordKey::InterfaceVtable { owner, interface } => shape_error(format!(
                "interface table ({owner}, {interface}) changed shape while a live value depends on it"
            )),
        }
    }

    /// Returns one descriptor record's words.
    pub(crate) fn words(&self, got_id: GotId) -> Option<&[u64]> {
        self.records
            .get(&RecordKey::Descriptor(got_id))
            .map(|entry| entry.storage.as_ref())
    }

    /// Returns one descriptor record's stable address.
    pub(crate) fn address(&self, got_id: GotId) -> Option<usize> {
        self.records
            .get(&RecordKey::Descriptor(got_id))
            .map(RecordEntry::address)
    }

    /// Returns one per-(object type, interface) VTABLE's words.
    pub(crate) fn interface_words(&self, owner: GotId, interface: GotId) -> Option<&[u64]> {
        self.records
            .get(&RecordKey::InterfaceVtable { owner, interface })
            .map(|entry| entry.storage.as_ref())
    }

    /// Returns one per-(object type, interface) VTABLE's stable address.
    pub(crate) fn interface_address(&self, owner: GotId, interface: GotId) -> Option<usize> {
        self.records
            .get(&RecordKey::InterfaceVtable { owner, interface })
            .map(RecordEntry::address)
    }

    /// Returns the number of quarantined descriptor records.
    pub(crate) fn quarantined_len(&self) -> usize {
        self.quarantined.len()
    }

    /// Releases every quarantined descriptor record.
    pub(crate) fn clear_quarantine(&mut self) -> usize {
        let count = self.quarantined.len();
        self.quarantined.clear();
        count
    }
}

/// The validated candidate record set.
struct CandidateRecords {
    type_ids: BTreeSet<GotId>,
    records: Vec<CandidateRecord>,
}

fn build_candidate_records(manifest: &Manifest) -> Result<CandidateRecords> {
    let types = TypeIndex::new(manifest)?;
    let functions = manifest.functions.iter().collect::<Vec<_>>();
    let function_slots = function_slots(manifest)?;
    let descriptors = descriptor_declarations(manifest, &types)?;
    let mut records = Vec::new();
    for ty in manifest
        .types
        .iter()
        .filter(|ty| is_object_kind(ty.type_kind))
    {
        let descriptor = descriptors
            .typeinfo
            .get(&ty.got_id)
            .ok_or_else(|| invalid(format!("type `{}` has no TYPEINFO descriptor", ty.key)))?;
        validate_descriptor_metadata(descriptor, ty)?;
        let base_chain = types.base_chain(ty.got_id)?;
        let interfaces = types.interface_map(ty.got_id)?;
        let vtable_slot_count = class_slot_count(&functions, &types, ty.got_id)?;
        let vtable_descriptor = descriptors.vtable.get(&ty.got_id);
        match (vtable_descriptor, ty.vtable_shape) {
            (Some(descriptor), Some(_)) => validate_descriptor_metadata(descriptor, ty)?,
            (None, None) => {}
            (Some(_), None) => {
                return Err(invalid(format!(
                    "type `{}` has a VTABLE descriptor but no vtable shape",
                    ty.key
                )));
            }
            (None, Some(_)) => {
                return Err(invalid(format!(
                    "type `{}` has method slots but no VTABLE descriptor",
                    ty.key
                )));
            }
        }
        if vtable_slot_count > 0 && vtable_descriptor.is_none() {
            return Err(invalid(format!(
                "type `{}` has {vtable_slot_count} method slots but no VTABLE descriptor",
                ty.key
            )));
        }
        let leaf_count = types.leaf_count(ty.got_id)?;
        let class_vtable =
            vtable_descriptor.map(|descriptor| RecordKey::Descriptor(descriptor.got_id));
        let interface_views = interfaces
            .iter()
            .map(|interface| {
                (
                    *interface,
                    RecordKey::InterfaceVtable {
                        owner: ty.got_id,
                        interface: *interface,
                    },
                )
            })
            .collect::<Vec<_>>();
        let shape = TypeInfoShape {
            owner: ty.got_id,
            type_key: ty.type_key.clone(),
            layout_hash: ty.layout_hash,
            size: ty.size,
            align: ty.align,
            kind: typeinfo_kind(ty.type_kind),
            leaf_count: u32::try_from(leaf_count)
                .map_err(|_| invalid("object leaf count exceeds u32"))?,
            base_chain: base_chain.clone(),
            interfaces: interfaces.clone(),
            vtable_slot_count,
            vtable_shape: ty.vtable_shape,
        };
        records.push(CandidateRecord {
            key: RecordKey::Descriptor(descriptor.got_id),
            shape: RecordShape::TypeInfo(shape),
            content: CandidateContent::TypeInfo {
                owner: ty.got_id,
                kind: typeinfo_kind(ty.type_kind),
                leaf_count: u32::try_from(leaf_count)
                    .map_err(|_| invalid("object leaf count exceeds u32"))?,
                base_chain,
                interfaces: interface_views,
                class_vtable,
            },
        });
        if let Some(descriptor) = vtable_descriptor {
            let entries = class_vtable_entries(
                &functions,
                &types,
                &function_slots,
                ty.got_id,
                vtable_slot_count,
            )?;
            records.push(CandidateRecord {
                key: RecordKey::Descriptor(descriptor.got_id),
                shape: RecordShape::ClassVtable(VtableShape {
                    owner: ty.got_id,
                    type_key: ty.type_key.clone(),
                    layout_hash: ty.layout_hash,
                    size: ty.size,
                    align: ty.align,
                    entry_count: vtable_slot_count,
                    vtable_shape: ty.vtable_shape,
                }),
                content: CandidateContent::ClassVtable {
                    owner: ty.got_id,
                    entries,
                },
            });
        }
        for interface in &interfaces {
            let interface_type = types.get(*interface)?;
            if interface_type.type_kind != TypeKind::Interface {
                return Err(invalid(format!(
                    "type `{}` implements non-interface `{}`",
                    ty.key, interface_type.key
                )));
            }
            let entries = interface_vtable_entries(
                &functions,
                &types,
                &function_slots,
                ty.got_id,
                *interface,
            )?;
            let entry_count = u32::try_from(entries.len())
                .map_err(|_| invalid("interface vtable exceeds u32 entries"))?;
            records.push(CandidateRecord {
                key: RecordKey::InterfaceVtable {
                    owner: ty.got_id,
                    interface: *interface,
                },
                shape: RecordShape::InterfaceVtable(PairShape {
                    owner: ty.got_id,
                    interface: *interface,
                    entry_count,
                    interface_vtable_shape: interface_type.vtable_shape,
                    interface_layout_hash: interface_type.layout_hash,
                }),
                content: CandidateContent::InterfaceVtable {
                    owner: ty.got_id,
                    interface: *interface,
                    entries,
                },
            });
        }
    }
    validate_hidden_roots(manifest, &types)?;
    Ok(CandidateRecords {
        type_ids: manifest
            .types
            .iter()
            .map(|ty| ty.got_id)
            .collect::<BTreeSet<_>>(),
        records,
    })
}

/// Descriptor declarations grouped by the owning type.
struct DescriptorDecls<'a> {
    typeinfo: BTreeMap<GotId, &'a DescriptorRecord>,
    vtable: BTreeMap<GotId, &'a DescriptorRecord>,
}

fn descriptor_declarations<'a>(
    manifest: &'a Manifest,
    types: &TypeIndex<'a>,
) -> Result<DescriptorDecls<'a>> {
    let mut decls = DescriptorDecls {
        typeinfo: BTreeMap::new(),
        vtable: BTreeMap::new(),
    };
    for descriptor in &manifest.descriptors {
        if descriptor.descriptor_kind == DescriptorKind::Object {
            return Err(invalid(format!(
                "descriptor `{}` projects an unowned object record",
                descriptor.key
            )));
        }
        let owner = descriptor.owner_type.ok_or_else(|| {
            invalid(format!(
                "descriptor `{}` has no owning type",
                descriptor.key
            ))
        })?;
        let owner_type = types.get(owner)?;
        if !is_object_kind(owner_type.type_kind) {
            return Err(invalid(format!(
                "descriptor `{}` is owned by non-object type `{}`",
                descriptor.key, owner_type.key
            )));
        }
        let entry = match descriptor.descriptor_kind {
            DescriptorKind::TypeInfo => &mut decls.typeinfo,
            DescriptorKind::Vtable => &mut decls.vtable,
            DescriptorKind::Object => unreachable!("object descriptors are rejected above"),
        };
        if entry.insert(owner, descriptor).is_some() {
            return Err(invalid(format!(
                "type `{}` has duplicate {:?} descriptors",
                owner_type.key, descriptor.descriptor_kind
            )));
        }
    }
    Ok(decls)
}

fn validate_descriptor_metadata(descriptor: &DescriptorRecord, ty: &TypeRecord) -> Result<()> {
    if descriptor.type_key != ty.type_key
        || descriptor.layout_hash != ty.layout_hash
        || descriptor.size != ty.size
        || descriptor.align != ty.align
    {
        return Err(invalid(format!(
            "descriptor `{}` type/layout identity does not match `{}`",
            descriptor.key, ty.key
        )));
    }
    Ok(())
}

fn is_object_kind(kind: TypeKind) -> bool {
    matches!(kind, TypeKind::Class | TypeKind::FunctionBlock)
}

fn typeinfo_kind(kind: TypeKind) -> u32 {
    match kind {
        TypeKind::Class => typeinfo_kind::CLASS,
        TypeKind::FunctionBlock => typeinfo_kind::FUNCTION_BLOCK,
        _ => 0,
    }
}

/// A checked index of the manifest's named types.
struct TypeIndex<'a> {
    types: BTreeMap<GotId, &'a TypeRecord>,
}

impl<'a> TypeIndex<'a> {
    fn new(manifest: &'a Manifest) -> Result<Self> {
        let mut types = BTreeMap::new();
        for ty in &manifest.types {
            if types.insert(ty.got_id, ty).is_some() {
                return Err(invalid(format!(
                    "type identity {} is duplicated",
                    ty.got_id
                )));
            }
        }
        Ok(Self { types })
    }

    fn get(&self, got_id: GotId) -> Result<&'a TypeRecord> {
        self.types
            .get(&got_id)
            .copied()
            .ok_or_else(|| invalid(format!("type {got_id} is not in the manifest")))
    }

    fn base_chain(&self, got_id: GotId) -> Result<Vec<GotId>> {
        let owner = self.get(got_id)?;
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(got_id);
        let mut current = owner.base;
        while let Some(base) = current {
            if !seen.insert(base) {
                return Err(invalid(format!(
                    "type `{}` has a cyclic base chain",
                    owner.key
                )));
            }
            let base_type = self.get(base)?;
            if base_type.type_kind != owner.type_kind {
                return Err(invalid(format!(
                    "type `{}` extends `{}` of a different kind",
                    owner.key, base_type.key
                )));
            }
            chain.push(base);
            current = base_type.base;
        }
        Ok(chain)
    }

    fn interface_map(&self, got_id: GotId) -> Result<Vec<GotId>> {
        let ty = self.get(got_id)?;
        let mut entries = match ty.base {
            Some(base) => self.interface_map(base)?,
            None => Vec::new(),
        };
        let mut seen = entries.iter().copied().collect::<BTreeSet<_>>();
        let mut added = Vec::new();
        for interface in &ty.interfaces {
            self.interface_closure(*interface, &mut seen, &mut added)?;
        }
        added.sort();
        entries.extend(added);
        Ok(entries)
    }

    fn interface_closure(
        &self,
        interface: GotId,
        seen: &mut BTreeSet<GotId>,
        out: &mut Vec<GotId>,
    ) -> Result<()> {
        if !seen.insert(interface) {
            return Ok(());
        }
        let ty = self.get(interface)?;
        if ty.type_kind != TypeKind::Interface {
            return Err(invalid(format!("type `{}` is not an interface", ty.key)));
        }
        out.push(interface);
        for parent in &ty.interfaces {
            self.interface_closure(*parent, seen, out)?;
        }
        Ok(())
    }

    /// Counts the object record's leaf entries (`N` in `ptr[N + 1]`).
    fn leaf_count(&self, got_id: GotId) -> Result<usize> {
        let mut active = BTreeSet::new();
        self.instance_leaf_count(got_id, &mut active)
    }

    /// Counts the storage leaves of one class/FB instance.
    fn instance_leaf_count(&self, got_id: GotId, active: &mut BTreeSet<GotId>) -> Result<usize> {
        if !active.insert(got_id) {
            return Err(invalid(format!("type {got_id} contains itself by value")));
        }
        let ty = self.get(got_id)?;
        let count = match ty.type_kind {
            TypeKind::Class | TypeKind::FunctionBlock => {
                let mut count = match ty.base {
                    Some(base) => self.instance_leaf_count(base, active)?,
                    None => 0,
                };
                for field in &ty.fields {
                    count += self.field_leaf_count(field, active)?;
                }
                count
            }
            TypeKind::Struct => {
                if overlap_struct(ty) {
                    1
                } else {
                    let mut count = 0;
                    for field in &ty.fields {
                        count += self.field_leaf_count(field, active)?;
                    }
                    count
                }
            }
            _ => 1,
        };
        active.remove(&got_id);
        Ok(count)
    }

    fn field_leaf_count(
        &self,
        field: &stc_got::FieldRecord,
        active: &mut BTreeSet<GotId>,
    ) -> Result<usize> {
        if field.size == 0 {
            return Ok(0);
        }
        self.key_leaf_count(&TypeKey::parse(&field.type_key)?, active)
    }

    fn key_leaf_count(&self, key: &TypeKey, active: &mut BTreeSet<GotId>) -> Result<usize> {
        match key {
            TypeKey::Named(got_id) => {
                let ty = self.types.get(got_id).copied().ok_or_else(|| {
                    invalid(format!(
                        "type {got_id} referenced by a field is not in the manifest"
                    ))
                })?;
                match ty.type_kind {
                    // A class value is a pointer-sized handle.
                    TypeKind::Class => Ok(1),
                    TypeKind::FunctionBlock | TypeKind::Struct => {
                        self.instance_leaf_count(*got_id, active)
                    }
                    _ => Ok(1),
                }
            }
            TypeKey::Array {
                lower,
                upper,
                element,
            } => {
                let count = array_length(*lower, *upper)?;
                Ok(count.saturating_mul(self.key_leaf_count(element, active)?))
            }
            _ => Ok(1),
        }
    }
}

fn overlap_struct(ty: &TypeRecord) -> bool {
    let mut ranges = Vec::new();
    for field in &ty.fields {
        let end = field.offset.saturating_add(field.size);
        if ranges
            .iter()
            .any(|(start, finish)| field.offset < *finish && *start < end)
        {
            return true;
        }
        ranges.push((field.offset, end));
    }
    false
}

fn array_length(lower: stc_got::ArrayBound, upper: stc_got::ArrayBound) -> Result<usize> {
    match (lower, upper) {
        (stc_got::ArrayBound::Value(lower), stc_got::ArrayBound::Value(upper)) => {
            let count = upper
                .checked_sub(lower)
                .and_then(|span| span.checked_add(1))
                .filter(|count| *count > 0)
                .ok_or_else(|| invalid("array bound is empty"))?;
            usize::try_from(count).map_err(|_| invalid("array length exceeds the platform limit"))
        }
        _ => Err(invalid("a conformant array cannot be an object leaf")),
    }
}

/// Maps every function identity to its function GOT slot.
fn function_slots(manifest: &Manifest) -> Result<BTreeMap<GotId, u32>> {
    let mut slots = BTreeMap::new();
    for slot in &manifest.slots {
        if slot.kind != SlotKind::Function {
            continue;
        }
        if slots.insert(slot.got_id, slot.slot).is_some() {
            return Err(binding(format!(
                "function {} has duplicate function slots",
                slot.got_id
            )));
        }
    }
    Ok(slots)
}

/// Returns the class VTABLE slot count for one object type.
fn class_slot_count(
    functions: &[&FunctionRecord],
    types: &TypeIndex<'_>,
    owner: GotId,
) -> Result<u32> {
    let chain = types.base_chain(owner)?;
    let mut count = 0_u32;
    for function in functions {
        if function.function_kind != FunctionKind::Method
            || !function
                .owner_type
                .is_some_and(|id| chain_contains(owner, &chain, id))
        {
            continue;
        }
        let Some(slot) = function.vtable_slot else {
            continue;
        };
        count = count.max(slot.saturating_add(1));
    }
    Ok(count)
}

fn chain_contains(owner: GotId, chain: &[GotId], candidate: GotId) -> bool {
    candidate == owner || chain.contains(&candidate)
}

fn class_vtable_entries(
    functions: &[&FunctionRecord],
    types: &TypeIndex<'_>,
    function_slots: &BTreeMap<GotId, u32>,
    owner: GotId,
    entry_count: u32,
) -> Result<Vec<Option<GotId>>> {
    let mut chain = vec![owner];
    chain.extend(types.base_chain(owner)?);
    let mut entries = Vec::with_capacity(entry_count as usize);
    for slot in 0..entry_count {
        let mut found = None;
        for type_id in &chain {
            let matching = functions
                .iter()
                .filter(|function| {
                    function.function_kind == FunctionKind::Method
                        && function.owner_type == Some(*type_id)
                        && function.vtable_slot == Some(slot)
                })
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                return Err(invalid(format!(
                    "type `{}` has duplicate functions for vtable slot {slot}",
                    types.get(*type_id)?.key
                )));
            }
            if let Some(function) = matching.first() {
                if function.flags & function_flag::USER == 0 {
                    return Err(invalid(format!(
                        "class VTABLE slot {slot} for {owner} has no implementation"
                    )));
                }
                found = Some(function.got_id);
                break;
            }
        }
        if let Some(got_id) = found
            && !function_slots.contains_key(&got_id)
        {
            return Err(binding(format!("function {got_id} has no function slot")));
        }
        entries.push(found);
    }
    Ok(entries)
}

fn interface_vtable_entries(
    functions: &[&FunctionRecord],
    types: &TypeIndex<'_>,
    function_slots: &BTreeMap<GotId, u32>,
    owner: GotId,
    interface: GotId,
) -> Result<Vec<GotId>> {
    let mut closure_seen = BTreeSet::new();
    let mut closure = Vec::new();
    types.interface_closure(interface, &mut closure_seen, &mut closure)?;
    let mut prototypes = BTreeMap::new();
    for function in functions {
        if function.function_kind != FunctionKind::Method {
            continue;
        }
        let Some(function_owner) = function.owner_type else {
            continue;
        };
        if !closure.contains(&function_owner) {
            continue;
        }
        let Some(slot) = function.vtable_slot else {
            continue;
        };
        if prototypes.insert(slot, *function).is_some() {
            return Err(invalid(format!(
                "interface {} has duplicate prototype slot {slot}",
                types.get(interface)?.key
            )));
        }
    }
    let entry_count = prototypes
        .keys()
        .next_back()
        .map(|slot| slot.saturating_add(1))
        .unwrap_or(0);
    if let Some(last) = prototypes.keys().next_back()
        && prototypes.len() as u32 != *last + 1
    {
        return Err(invalid(format!(
            "interface {} prototype slots are not contiguous",
            types.get(interface)?.key
        )));
    }
    let interface_key = &types.get(interface)?.key;
    let mut entries = Vec::with_capacity(entry_count as usize);
    let mut chain = vec![owner];
    chain.extend(types.base_chain(owner)?);
    for slot in 0..entry_count {
        let prototype = prototypes.get(&slot).copied().ok_or_else(|| {
            invalid(format!(
                "interface {interface_key} has no prototype at slot {slot}"
            ))
        })?;
        if prototype.flags & function_flag::USER != 0 {
            return Err(invalid(format!(
                "interface prototype `{}` has a body",
                prototype.key
            )));
        }
        if !function_slots.contains_key(&prototype.got_id) {
            return Err(binding(format!(
                "interface prototype `{}` has no trap-stub function slot",
                prototype.key
            )));
        }
        let name = method_name(&prototype.key)
            .ok_or_else(|| invalid(format!("prototype `{}` has no method name", prototype.key)))?;
        let mut found = None;
        for type_id in &chain {
            let matching = functions
                .iter()
                .filter(|function| {
                    function.function_kind == FunctionKind::Method
                        && function.owner_type == Some(*type_id)
                        && function.flags & function_flag::USER != 0
                        && function.signature_key == prototype.signature_key
                        && method_name(&function.key) == Some(name)
                })
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                return Err(invalid(format!(
                    "type `{}` has duplicate implementations of `{name}`",
                    types.get(*type_id)?.key
                )));
            }
            if let Some(function) = matching.first() {
                found = Some(function.got_id);
                break;
            }
        }
        let entry = found.unwrap_or(prototype.got_id);
        if !function_slots.contains_key(&entry) {
            return Err(binding(format!("function {entry} has no function slot")));
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn method_name(key: &str) -> Option<&str> {
    let (path, _) = key.rsplit_once('@')?;
    path.rsplit('.').next()
}

fn validate_hidden_roots(manifest: &Manifest, types: &TypeIndex<'_>) -> Result<()> {
    for root in &manifest.roots {
        let Some(marker) = hidden_root_marker(&root.key) else {
            continue;
        };
        let key = TypeKey::parse(&root.type_key)?;
        match marker {
            "OBJREC" => {
                let TypeKey::Array {
                    lower,
                    upper,
                    element,
                } = key
                else {
                    return Err(invalid(format!(
                        "object record `{}` does not have an array type key",
                        root.key
                    )));
                };
                let (stc_got::ArrayBound::Value(0), stc_got::ArrayBound::Value(last)) =
                    (lower, upper)
                else {
                    return Err(invalid(format!(
                        "object record `{}` is not ARRAY[0..N]",
                        root.key
                    )));
                };
                let TypeKey::PtrTo(inner) = *element else {
                    return Err(invalid(format!(
                        "object record `{}` does not hold pointers",
                        root.key
                    )));
                };
                let TypeKey::Named(owner) = *inner else {
                    return Err(invalid(format!(
                        "object record `{}` does not name its owner type",
                        root.key
                    )));
                };
                let ty = types.get(owner)?;
                if !is_object_kind(ty.type_kind) {
                    return Err(invalid(format!(
                        "object record `{}` names non-object type `{}`",
                        root.key, ty.key
                    )));
                }
                let records =
                    u64::try_from(last).map_err(|_| invalid("object record length exceeds u64"))?;
                if root.size != (records + 1) * 8 || root.align != 8 {
                    return Err(invalid(format!(
                        "object record `{}` size/alignment does not match its leaf count",
                        root.key
                    )));
                }
                if types.leaf_count(owner)? as u64 != records {
                    return Err(invalid(format!(
                        "object record `{}` leaf count does not match `{}`",
                        root.key, ty.key
                    )));
                }
            }
            "HANDLE" => {
                let (count, owner) = match key {
                    TypeKey::PtrTo(inner) => match *inner {
                        TypeKey::Named(owner) => (1_u64, owner),
                        _ => {
                            return Err(invalid(format!(
                                "handle root `{}` does not name a class",
                                root.key
                            )));
                        }
                    },
                    TypeKey::Array {
                        lower,
                        upper,
                        element,
                    } => {
                        let TypeKey::PtrTo(inner) = *element else {
                            return Err(invalid(format!(
                                "handle array `{}` does not hold pointers",
                                root.key
                            )));
                        };
                        let TypeKey::Named(owner) = *inner else {
                            return Err(invalid(format!(
                                "handle array `{}` does not name a class",
                                root.key
                            )));
                        };
                        (array_length(lower, upper)? as u64, owner)
                    }
                    _ => {
                        return Err(invalid(format!(
                            "handle root `{}` does not have a pointer type key",
                            root.key
                        )));
                    }
                };
                let ty = types.get(owner)?;
                if ty.type_kind != TypeKind::Class {
                    return Err(invalid(format!(
                        "handle root `{}` names non-class type `{}`",
                        root.key, ty.key
                    )));
                }
                if root.size != count * 8 || root.align != 8 {
                    return Err(invalid(format!(
                        "handle root `{}` size/alignment does not match its handle count",
                        root.key
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn set_u32(words: &mut [u64], offset: usize, value: u32) {
    let index = offset / WORD_BYTES;
    let shift = (offset % WORD_BYTES) * 8;
    words[index] = (words[index] & !(u64::from(u32::MAX) << shift)) | (u64::from(value) << shift);
}

fn set_u64(words: &mut [u64], offset: usize, value: u64) {
    words[offset / WORD_BYTES] = value;
}

fn set_id(words: &mut [u64], offset: usize, id: GotId) {
    let bytes = id.as_bytes();
    let index = offset / WORD_BYTES;
    words[index] = u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes"));
    words[index + 1] = u64::from_le_bytes(bytes[8..].try_into().expect("eight bytes"));
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(codes::MODULE_INVALID, message)
}

fn binding(message: impl Into<String>) -> Error {
    Error::new(codes::ONLINE_BINDING, message)
}

fn shape_error(message: impl Into<String>) -> Error {
    Error::new(codes::ONLINE_VTABLE, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::DescriptorKind;
    use stc_got::descriptor::function_flag;

    const OOP: &str = r#"
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

CLASS Holder
VAR PUBLIC
  shape : Shape;
END_VAR
END_CLASS

FUNCTION_BLOCK FB
VAR_INPUT
  inc : DINT;
END_VAR
VAR
  cv : DINT;
END_VAR
  cv := cv + inc;
END_FUNCTION_BLOCK

VAR_GLOBAL
  s : Shape;
  f : FB;
  sa : ARRAY[1..2] OF Shape;
END_VAR

PROGRAM Main
VAR
  m : Shape;
  fb : FB;
END_VAR
  m := s;
  fb(inc := 1);
END_PROGRAM
"#;

    fn artifacts() -> stc_codegen::module::ModuleArtifacts {
        crate::test_support::artifacts(OOP, "oop")
    }

    fn function_cells(manifest: &Manifest) -> BTreeMap<GotId, usize> {
        manifest
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.got_id, 0x1000 + index * 0x40))
            .collect()
    }

    fn word(words: &[u64], offset: usize) -> u64 {
        let index = offset / 8;
        let shift = (offset % 8) * 8;
        (words[index] >> shift) & u64::from(u32::MAX)
    }

    fn id(words: &[u64], offset: usize) -> GotId {
        let index = offset / 8;
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&words[index].to_le_bytes());
        bytes[8..].copy_from_slice(&words[index + 1].to_le_bytes());
        GotId::from_bytes(bytes)
    }

    fn descriptor(manifest: &Manifest, owner: GotId, kind: DescriptorKind) -> &DescriptorRecord {
        manifest
            .descriptors
            .iter()
            .find(|descriptor| {
                descriptor.owner_type == Some(owner) && descriptor.descriptor_kind == kind
            })
            .unwrap_or_else(|| panic!("missing {kind:?} descriptor for {owner}"))
    }

    fn record<'a>(manifest: &'a Manifest, key: &str) -> &'a TypeRecord {
        manifest
            .types
            .iter()
            .find(|ty| ty.key == key)
            .unwrap_or_else(|| panic!("missing type `{key}`"))
    }

    #[test]
    fn builds_typeinfo_and_vtable_records_from_the_catalog() {
        let artifacts = artifacts();
        let manifest = &artifacts.manifest;
        let cells = function_cells(manifest);
        let mut registry = DescriptorRegistry::new();
        let prepared = registry
            .prepare(manifest, &BTreeSet::new(), &cells)
            .expect("descriptor prepare");
        registry.activate(prepared);
        let shape = record(manifest, "_ROOT::SHAPE");
        let typeinfo = descriptor(manifest, shape.got_id, DescriptorKind::TypeInfo);
        let vtable = descriptor(manifest, shape.got_id, DescriptorKind::Vtable);
        let words = registry.words(typeinfo.got_id).expect("typeinfo");
        assert_eq!(word(words, 0), u64::from(RECORD_FORMAT));
        assert_eq!(word(words, 4), u64::from(typeinfo_kind::CLASS));
        assert_eq!(word(words, 8), 0);
        assert_eq!(word(words, 12), 1, "Shape has one leaf");
        assert_eq!(word(words, 16), 0, "no base");
        assert_eq!(word(words, 20), 2, "IBase and IShape");
        assert_eq!(word(words, 24), 2, "two class method slots");
        assert_eq!(id(words, 32), shape.got_id);
        assert_eq!(id(words, 48), shape.layout_hash);
        assert_eq!(
            words[8],
            registry.address(vtable.got_id).expect("vtable") as u64
        );
        let base = &manifest
            .types
            .iter()
            .find(|ty| ty.key == "_ROOT::IBASE")
            .unwrap();
        let ishape = &manifest
            .types
            .iter()
            .find(|ty| ty.key == "_ROOT::ISHAPE")
            .unwrap();
        assert_eq!(id(words, 72), base.got_id);
        assert_eq!(id(words, 96), ishape.got_id);
        assert_eq!(
            words[11],
            registry
                .interface_address(shape.got_id, base.got_id)
                .expect("IBase table") as u64
        );
        assert_eq!(
            words[14],
            registry
                .interface_address(shape.got_id, ishape.got_id)
                .expect("IShape table") as u64
        );

        let class_words = registry.words(vtable.got_id).expect("vtable words");
        assert_eq!(word(class_words, 0), u64::from(RECORD_FORMAT));
        assert_eq!(word(class_words, 4), u64::from(vtable_kind::CLASS));
        assert_eq!(word(class_words, 8), 2);
        assert_eq!(id(class_words, 16), shape.got_id);
        assert_eq!(id(class_words, 32), GotId::ZERO);

        let area = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::SHAPE.AREA@()->DINT")
            .expect("Area method");
        let perimeter = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::SHAPE.PERIMETER@()->DINT")
            .expect("Perimeter method");
        let slots: Vec<u32> = vec![area.vtable_slot.unwrap(), perimeter.vtable_slot.unwrap()];
        for (index, function) in [area, perimeter].iter().enumerate() {
            let slot = slots[index];
            assert_eq!(
                class_words[6 + slot as usize],
                cells[&function.got_id] as u64
            );
        }

        let pair = registry
            .interface_words(shape.got_id, ishape.got_id)
            .expect("IShape pair");
        assert_eq!(word(pair, 4), u64::from(vtable_kind::INTERFACE));
        assert_eq!(word(pair, 8), 2);
        assert_eq!(id(pair, 16), shape.got_id);
        assert_eq!(id(pair, 32), ishape.got_id);
        let area_proto = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::IBASE.AREA@()->DINT")
            .expect("Area prototype");
        let perimeter_proto = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::ISHAPE.PERIMETER@()->DINT")
            .expect("Perimeter prototype");
        assert!(area_proto.flags & function_flag::USER == 0);
        assert!(perimeter_proto.flags & function_flag::USER == 0);
        assert_eq!(pair[6], cells[&area.got_id] as u64);
        assert_eq!(pair[7], cells[&perimeter.got_id] as u64);
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    use stc_got::DescriptorKind;

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

CLASS Holder
VAR PUBLIC
  shape : Shape;
END_VAR
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

CLASS Holder
VAR PUBLIC
  shape : Shape;
END_VAR
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

    fn artifacts(source: &str) -> stc_codegen::module::ModuleArtifacts {
        crate::test_support::artifacts(source, "oop")
    }

    fn artifacts_with(
        source: &str,
        baseline: stc_got::Manifest,
    ) -> stc_codegen::module::ModuleArtifacts {
        crate::test_support::artifacts_with(source, "oop", baseline, false)
    }

    fn function_cells_with(manifest: &Manifest, salt: usize) -> BTreeMap<GotId, usize> {
        manifest
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.got_id, salt + 0x1000 + index * 0x40))
            .collect()
    }

    fn function_cells(manifest: &Manifest) -> BTreeMap<GotId, usize> {
        function_cells_with(manifest, 0)
    }

    fn prepare(
        registry: &mut DescriptorRegistry,
        manifest: &Manifest,
        live: &BTreeSet<GotId>,
    ) -> Result<PreparedDescriptors> {
        let cells = function_cells(manifest);
        registry.prepare(manifest, live, &cells)
    }

    fn type_record<'a>(
        artifacts: &'a stc_codegen::module::ModuleArtifacts,
        key: &str,
    ) -> &'a TypeRecord {
        artifacts
            .manifest
            .types
            .iter()
            .find(|ty| ty.key == key)
            .unwrap_or_else(|| panic!("type `{key}`"))
    }

    fn descriptor(
        artifacts: &stc_codegen::module::ModuleArtifacts,
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

    fn commit(registry: &mut DescriptorRegistry, prepared: PreparedDescriptors) {
        let activation = registry.activate(prepared);
        registry.commit(activation);
    }

    #[test]
    fn compatible_shapes_keep_addresses_and_roll_back_entries() {
        let v1 = artifacts(OOP_V1);
        let mut registry = DescriptorRegistry::new();
        let prepared = prepare(&mut registry, &v1.manifest, &BTreeSet::new()).expect("v1");
        assert_eq!(prepared.fresh_count(), v1.manifest.descriptors.len() + 2);
        let shape = type_record(&v1, "_ROOT::SHAPE");
        let ishape = type_record(&v1, "_ROOT::ISHAPE");
        let typeinfo = descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo);
        commit(&mut registry, prepared);
        let typeinfo_address = registry.address(typeinfo.got_id).expect("typeinfo");
        let pair_address = registry
            .interface_address(shape.got_id, ishape.got_id)
            .expect("pair");
        let old_pair = registry
            .interface_words(shape.got_id, ishape.got_id)
            .expect("pair")
            .to_vec();

        let v2 = artifacts_with(OOP_V2, v1.manifest.clone());
        let cells = function_cells_with(&v2.manifest, 0x10_0000);
        let prepared = registry
            .prepare(&v2.manifest, &BTreeSet::new(), &cells)
            .expect("v2");
        assert_eq!(prepared.fresh_count(), 0, "compatible shapes are reused");
        assert_eq!(registry.address(typeinfo.got_id), Some(typeinfo_address));
        assert_eq!(
            registry.interface_address(shape.got_id, ishape.got_id),
            Some(pair_address)
        );
        assert_eq!(
            registry
                .interface_words(shape.got_id, ishape.got_id)
                .expect("pair"),
            old_pair.as_slice(),
            "prepare leaves the active record untouched"
        );

        let activation = registry.activate(prepared);
        let patch_pair = registry
            .interface_words(shape.got_id, ishape.got_id)
            .expect("pair");
        let area = v2
            .manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::SHAPE.AREA@()->DINT")
            .expect("Area");
        assert_eq!(patch_pair[6], cells[&area.got_id] as u64);
        assert_ne!(patch_pair, old_pair.as_slice());

        registry.rollback(activation);
        assert_eq!(registry.address(typeinfo.got_id), Some(typeinfo_address));
        assert_eq!(
            registry
                .interface_words(shape.got_id, ishape.got_id)
                .expect("pair"),
            old_pair.as_slice(),
            "rollback restores the previous entries"
        );
    }

    #[test]
    fn shape_changes_reject_live_types_and_reallocate_otherwise() {
        let v1 = artifacts(OOP_V1);
        let mut registry = DescriptorRegistry::new();
        let prepared = prepare(&mut registry, &v1.manifest, &BTreeSet::new()).unwrap();
        commit(&mut registry, prepared);
        let shape = type_record(&v1, "_ROOT::SHAPE");
        let typeinfo = descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo);
        let old_address = registry.address(typeinfo.got_id).expect("typeinfo");

        let mut v2 = artifacts_with(OOP_V2, v1.manifest.clone());
        let new_hash = GotId::from_bytes([0x5A; 16]);
        for ty in &mut v2.manifest.types {
            if ty.got_id == shape.got_id {
                ty.layout_hash = new_hash;
            }
        }
        for descriptor in &mut v2.manifest.descriptors {
            if descriptor.owner_type == Some(shape.got_id) {
                descriptor.layout_hash = new_hash;
            }
        }
        let live = [shape.got_id].into_iter().collect::<BTreeSet<_>>();
        let error = prepare(&mut registry, &v2.manifest, &live).expect_err("live shape change");
        assert_eq!(error.code, codes::ONLINE_VTABLE);
        assert_eq!(registry.address(typeinfo.got_id), Some(old_address));

        let prepared = prepare(&mut registry, &v2.manifest, &BTreeSet::new()).expect("no live");
        assert_eq!(prepared.fresh_count(), 2, "TYPEINFO and class VTABLE");
        let moved = registry.address(typeinfo.got_id).expect("typeinfo");
        assert_ne!(moved, old_address);
        assert_eq!(registry.quarantined_len(), 2);
        registry.revert_prepare(prepared);
        assert_eq!(registry.address(typeinfo.got_id), Some(old_address));
        assert_eq!(registry.quarantined_len(), 0);

        let prepared = prepare(&mut registry, &v2.manifest, &BTreeSet::new()).expect("retry");
        commit(&mut registry, prepared);
        assert_eq!(registry.quarantined_len(), 2);
        assert_eq!(registry.clear_quarantine(), 2);
        assert_eq!(registry.quarantined_len(), 0);
        assert!(registry.address(typeinfo.got_id).is_some());
    }

    #[test]
    fn removing_a_live_type_or_interface_is_rejected() {
        let v1 = artifacts(OOP_V1);
        let mut registry = DescriptorRegistry::new();
        let prepared = prepare(&mut registry, &v1.manifest, &BTreeSet::new()).unwrap();
        commit(&mut registry, prepared);
        let holder = type_record(&v1, "_ROOT::HOLDER");
        let ishape = type_record(&v1, "_ROOT::ISHAPE");

        let mut v2 = artifacts_with(OOP_V2, v1.manifest.clone());
        v2.manifest.types.retain(|ty| ty.got_id != holder.got_id);
        v2.manifest
            .descriptors
            .retain(|descriptor| descriptor.owner_type != Some(holder.got_id));

        let live = [holder.got_id].into_iter().collect::<BTreeSet<_>>();
        let error = prepare(&mut registry, &v2.manifest, &live).expect_err("live removal");
        assert_eq!(error.code, codes::ONLINE_VTABLE);
        prepare(&mut registry, &v2.manifest, &BTreeSet::new()).expect("unreferenced removal");

        let live = [ishape.got_id].into_iter().collect::<BTreeSet<_>>();
        let mut v3 = artifacts_with(OOP_V2, v1.manifest.clone());
        v3.manifest.types.retain(|ty| ty.got_id != ishape.got_id);
        for ty in &mut v3.manifest.types {
            ty.interfaces
                .retain(|interface| *interface != ishape.got_id);
        }
        v3.manifest
            .descriptors
            .retain(|descriptor| descriptor.owner_type != Some(ishape.got_id));
        let error = prepare(&mut registry, &v3.manifest, &live).expect_err("live interface");
        assert_eq!(error.code, codes::ONLINE_VTABLE);
    }

    #[test]
    fn a_missing_implementation_uses_the_prototype_trap_stub() {
        let mut artifacts = artifacts(OOP_V1);
        let area_method = artifacts
            .manifest
            .functions
            .iter_mut()
            .find(|function| function.key == "_ROOT::SHAPE.AREA@()->DINT")
            .expect("Area method");
        area_method.key = "_ROOT::SHAPE.AREAB@()->DINT".into();
        let shape = artifacts
            .manifest
            .types
            .iter()
            .find(|ty| ty.key == "_ROOT::SHAPE")
            .expect("Shape")
            .got_id;
        let prototype = artifacts
            .manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::IBASE.AREA@()->DINT")
            .expect("Area prototype")
            .got_id;
        let ishape = artifacts
            .manifest
            .types
            .iter()
            .find(|ty| ty.key == "_ROOT::ISHAPE")
            .expect("IShape")
            .got_id;

        let cells = function_cells(&artifacts.manifest);
        let mut registry = DescriptorRegistry::new();
        let prepared = registry
            .prepare(&artifacts.manifest, &BTreeSet::new(), &cells)
            .expect("prepare");
        let activation = registry.activate(prepared);
        let pair = registry
            .interface_words(shape, ishape)
            .expect("IShape pair");
        assert_eq!(pair[6], cells[&prototype] as u64);
        registry.rollback(activation);

        artifacts
            .manifest
            .functions
            .iter_mut()
            .find(|function| function.got_id == prototype)
            .expect("prototype")
            .flags |= function_flag::USER;
        let error = registry
            .prepare(&artifacts.manifest, &BTreeSet::new(), &cells)
            .expect_err("a body-bearing prototype is invalid");
        assert_eq!(error.code, codes::MODULE_INVALID);
    }

    #[test]
    fn rejects_missing_and_malformed_descriptors() {
        let v1 = artifacts(OOP_V1);
        let shape = type_record(&v1, "_ROOT::SHAPE");
        let typeinfo = descriptor(&v1, shape.got_id, DescriptorKind::TypeInfo);

        let mut malformed = v1.clone();
        malformed
            .manifest
            .descriptors
            .iter_mut()
            .find(|descriptor| descriptor.got_id == typeinfo.got_id)
            .expect("typeinfo")
            .size += 8;
        let mut registry = DescriptorRegistry::new();
        let error = prepare(&mut registry, &malformed.manifest, &BTreeSet::new())
            .expect_err("size mismatch");
        assert_eq!(error.code, codes::MODULE_INVALID);

        let mut missing = v1.clone();
        missing
            .manifest
            .descriptors
            .retain(|descriptor| descriptor.got_id != typeinfo.got_id);
        let error = prepare(&mut registry, &missing.manifest, &BTreeSet::new())
            .expect_err("missing typeinfo");
        assert_eq!(error.code, codes::MODULE_INVALID);

        let mut duplicate = v1.clone();
        let mut extra = typeinfo.clone();
        extra.got_id = GotId::from_bytes([0x77; 16]);
        extra.key = "_ROOT::SHAPE#DESC:TYPEINFO2".into();
        duplicate.manifest.descriptors.push(extra);
        let error = prepare(&mut registry, &duplicate.manifest, &BTreeSet::new())
            .expect_err("duplicate typeinfo");
        assert_eq!(error.code, codes::MODULE_INVALID);
    }

    #[test]
    fn rejects_noncontiguous_interface_slots_and_missing_stub_cells() {
        let v1 = artifacts(OOP_V1);
        let mut gapped = v1.clone();
        gapped
            .manifest
            .functions
            .iter_mut()
            .find(|function| function.key == "_ROOT::IBASE.AREA@()->DINT")
            .expect("prototype")
            .vtable_slot = Some(5);
        let mut registry = DescriptorRegistry::new();
        let error =
            prepare(&mut registry, &gapped.manifest, &BTreeSet::new()).expect_err("slot gap");
        assert_eq!(error.code, codes::MODULE_INVALID);

        let mut missing = v1.clone();
        let prototype = missing
            .manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::IBASE.AREA@()->DINT")
            .expect("prototype")
            .got_id;
        missing
            .manifest
            .slots
            .retain(|slot| slot.got_id != prototype);
        let error = prepare(&mut registry, &missing.manifest, &BTreeSet::new())
            .expect_err("missing stub cell");
        assert_eq!(error.code, codes::ONLINE_BINDING);
    }

    const DERIVED: &str = r#"
INTERFACE IA
METHOD A : DINT
END_METHOD
END_INTERFACE
INTERFACE IZ
METHOD Z : DINT
END_METHOD
END_INTERFACE
INTERFACE IB EXTENDS IA
METHOD B : DINT
END_METHOD
END_INTERFACE

CLASS Base IMPLEMENTS IZ
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Z : DINT
  Z := id;
END_METHOD
END_CLASS

CLASS Derived EXTENDS Base IMPLEMENTS IB
METHOD PUBLIC A : DINT
  A := id;
END_METHOD
METHOD PUBLIC B : DINT
  B := id + 1;
END_METHOD
END_CLASS

VAR_GLOBAL
  d : Derived;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

    #[test]
    fn derived_types_project_the_base_first_map_and_inherited_slots() {
        let artifacts = artifacts(DERIVED);
        let manifest = &artifacts.manifest;
        let base = type_record(&artifacts, "_ROOT::BASE");
        let derived = type_record(&artifacts, "_ROOT::DERIVED");
        let ia = type_record(&artifacts, "_ROOT::IA");
        let iz = type_record(&artifacts, "_ROOT::IZ");
        let ib = type_record(&artifacts, "_ROOT::IB");
        let typeinfo = descriptor(&artifacts, derived.got_id, DescriptorKind::TypeInfo);
        let vtable = descriptor(&artifacts, derived.got_id, DescriptorKind::Vtable);
        let cells = function_cells(manifest);
        let mut registry = DescriptorRegistry::new();
        let prepared = registry
            .prepare(manifest, &BTreeSet::new(), &cells)
            .expect("prepare");
        let activation = registry.activate(prepared);
        let words = registry.words(typeinfo.got_id).expect("typeinfo");

        assert_eq!(word(words, 12), 1, "one instance leaf");
        assert_eq!(word(words, 16), 1, "one base");
        assert_eq!(word(words, 20), 3, "IZ plus IB's closure");
        assert_eq!(word(words, 24), 3, "Z, A, B slots");
        assert_eq!(id_at_word(words, 72), base.got_id);

        let inherited = iz.got_id;
        let mut added = [ia.got_id, ib.got_id];
        added.sort();
        let map = [inherited, added[0], added[1]];
        for (index, interface) in map.iter().enumerate() {
            assert_eq!(id_at_word(words, 88 + index * 24), *interface);
            assert_eq!(
                pointer_word(words, 88 + index * 24 + 16),
                registry
                    .interface_address(derived.got_id, *interface)
                    .expect("pair table")
            );
        }

        let class_words = registry.words(vtable.got_id).expect("vtable");
        assert_eq!(word(class_words, 8), 3);
        let base_z = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::BASE.Z@()->DINT")
            .expect("Base.Z");
        let derived_a = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::DERIVED.A@()->DINT")
            .expect("Derived.A");
        let derived_b = manifest
            .functions
            .iter()
            .find(|function| function.key == "_ROOT::DERIVED.B@()->DINT")
            .expect("Derived.B");
        assert_eq!(
            class_words[6 + base_z.vtable_slot.unwrap() as usize],
            cells[&base_z.got_id] as u64,
            "an inherited method keeps its base slot"
        );
        assert_eq!(
            class_words[6 + derived_a.vtable_slot.unwrap() as usize],
            cells[&derived_a.got_id] as u64
        );
        assert_eq!(
            class_words[6 + derived_b.vtable_slot.unwrap() as usize],
            cells[&derived_b.got_id] as u64
        );

        let iz_pair = registry
            .interface_words(derived.got_id, iz.got_id)
            .expect("IZ pair");
        assert_eq!(iz_pair[6], cells[&base_z.got_id] as u64);

        registry.rollback(activation);
    }

    fn word(words: &[u64], offset: usize) -> u32 {
        let index = offset / 8;
        let shift = (offset % 8) * 8;
        u32::try_from((words[index] >> shift) & u64::from(u32::MAX)).expect("u32 field")
    }

    fn id_at_word(words: &[u64], offset: usize) -> GotId {
        let index = offset / 8;
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&words[index].to_le_bytes());
        bytes[8..].copy_from_slice(&words[index + 1].to_le_bytes());
        GotId::from_bytes(bytes)
    }

    fn pointer_word(words: &[u64], offset: usize) -> usize {
        usize::try_from(words[offset / 8]).expect("pointer word")
    }
}
