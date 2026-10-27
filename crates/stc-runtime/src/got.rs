//! Host-owned GOT cells and per-generation slot tables.
//!
//! Each cell is a stable pointer-sized host slot. Generated code receives the
//! address of the slot table (one address per cell) at attach and reads cell
//! values; only the host writes them (Online Change and GOT §3). The host owns
//! the table for a whole generation, so a cell address stays valid for the
//! generation's lifetime.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::error::{Error, Result, codes};
use stc_got::{GotId, SlotKind, SlotRecord};

use crate::memory::RootRegistry;

/// A host-owned GOT for one module generation.
///
/// Cells are stored as `usize` pointer-sized values. The cell vector and the
/// slot-table address vector are allocated once and never resized, so
/// `cell_address`, `slot_table`, and the array handed to a module remain stable
/// while the host rebinds values.
#[derive(Debug)]
pub struct Got {
    slots: Vec<SlotRecord>,
    cells: Vec<usize>,
    addresses: Vec<usize>,
}

impl Got {
    /// Creates an unbound GOT from a canonical slot table.
    ///
    /// # Errors
    ///
    /// Rejects a non-contiguous, ungrouped, unsorted, or otherwise malformed
    /// slot table.
    pub fn new(slots: &[SlotRecord]) -> Result<Self> {
        validate_slot_table(slots)?;
        let cells = vec![0; slots.len()];
        let mut got = Self {
            slots: slots.to_vec(),
            cells,
            addresses: Vec::new(),
        };
        got.refresh_addresses();
        Ok(got)
    }

    fn refresh_addresses(&mut self) {
        let base = self.cells.as_ptr() as usize;
        let stride = std::mem::size_of::<usize>();
        self.addresses = (0..self.cells.len())
            .map(|index| base + index * stride)
            .collect();
    }

    /// Returns the number of cells.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.cells.len()
    }

    /// Returns the slot table.
    #[must_use]
    pub fn slots(&self) -> &[SlotRecord] {
        &self.slots
    }

    /// Returns one slot record.
    #[must_use]
    pub fn slot(&self, slot: u32) -> Option<&SlotRecord> {
        self.slots.get(usize::try_from(slot).ok()?)
    }

    /// Returns one cell value.
    ///
    /// An unbound cell reads as zero.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range slot.
    pub fn cell_value(&self, slot: u32) -> Result<usize> {
        let index = self.index(slot)?;
        Ok(self.cells[index])
    }

    /// Returns the address of one cell.
    ///
    /// This is the value placed in the slot table passed to a module. The
    /// address is stable for the lifetime of the GOT.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range slot.
    pub fn cell_address(&self, slot: u32) -> Result<*mut u8> {
        let index = self.index(slot)?;
        Ok(self.addresses[index] as *mut u8)
    }

    /// Writes one cell.
    ///
    /// The host calls this during prepare, activation, and rollback; generated
    /// code only reads cells.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range slot.
    pub fn set_cell(&mut self, slot: u32, value: usize) -> Result<()> {
        let index = self.index(slot)?;
        self.cells[index] = value;
        Ok(())
    }

    /// Returns `true` when a cell has a nonzero value.
    ///
    /// # Errors
    ///
    /// Rejects an out-of-range slot.
    pub fn is_bound(&self, slot: u32) -> Result<bool> {
        Ok(self.cell_value(slot)? != 0)
    }

    /// Returns the slots whose cells are still unbound, in slot order.
    #[must_use]
    pub fn unbound_slots(&self) -> Vec<u32> {
        self.slots
            .iter()
            .filter(|slot| {
                slot.slot
                    .try_into()
                    .ok()
                    .and_then(|index: usize| self.cells.get(index))
                    .is_none_or(|value| *value == 0)
            })
            .map(|slot| slot.slot)
            .collect()
    }

    /// Returns the slot-table addresses passed to `stc_module_attach`.
    ///
    /// Each element is the address of the corresponding cell. The slice lives
    /// as long as this GOT, so a module may cache the array pointer for the
    /// generation.
    #[must_use]
    pub fn slot_table(&self) -> &[usize] {
        &self.addresses
    }

    /// Returns the slot-table array as the C `void* const*` an attached module
    /// receives.
    ///
    /// `usize` and pointers have the same size and alignment on every supported
    /// target, so the array can be read as pointer-sized cell addresses.
    #[must_use]
    pub fn slot_table_pointer(&self) -> *const *mut u8 {
        self.addresses.as_ptr().cast()
    }

    /// Builds the `fresh_root_mask` passed to `stc_module_prepare`.
    ///
    /// The mask is indexed by slot number (GOT Artifacts §7.3): bit
    /// `slot % 8` of byte `slot / 8` is set for every data slot whose root is
    /// in `fresh`. Bits for non-data slots stay clear. A cold install ignores
    /// the mask and initializes every data root.
    #[must_use]
    pub fn fresh_root_mask(&self, fresh: &BTreeSet<GotId>) -> Vec<u8> {
        let mut mask = vec![0_u8; self.cells.len().div_ceil(8)];
        for slot in &self.slots {
            if slot.kind != SlotKind::Data || !fresh.contains(&slot.got_id) {
                continue;
            }
            let Ok(index) = usize::try_from(slot.slot) else {
                continue;
            };
            if let Some(byte) = mask.get_mut(index / 8) {
                *byte |= 1 << (index % 8);
            }
        }
        mask
    }

    /// Binds every data cell to its active root's storage.
    ///
    /// Direct cells are not persistent roots and are left unbound; the host
    /// resolves them against the process image.
    ///
    /// # Errors
    ///
    /// Rejects a data slot whose root is not active.
    pub fn bind_data_roots(&mut self, roots: &RootRegistry) -> Result<()> {
        let bindings: Vec<(u32, usize)> = self
            .slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Data)
            .map(|slot| {
                roots
                    .address(slot.got_id)
                    .map(|address| (slot.slot, address))
                    .ok_or_else(|| {
                        Error::new(
                            codes::ONLINE_BINDING,
                            format!(
                                "data slot {} has no active root for {}",
                                slot.slot, slot.got_id
                            ),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        for (slot, address) in bindings {
            self.set_cell(slot, address)?;
        }
        Ok(())
    }

    /// Binds every function cell from a `GotId`-keyed address map.
    ///
    /// # Errors
    ///
    /// Rejects a function slot whose identity is missing from `addresses`.
    pub fn bind_functions(&mut self, addresses: &BTreeMap<GotId, usize>) -> Result<()> {
        self.bind_identities(SlotKind::Function, addresses, "function")
    }

    /// Binds every descriptor cell from a `GotId`-keyed address map.
    ///
    /// # Errors
    ///
    /// Rejects a descriptor slot whose identity is missing from `addresses`.
    pub fn bind_descriptors(&mut self, addresses: &BTreeMap<GotId, usize>) -> Result<()> {
        self.bind_identities(SlotKind::Descriptor, addresses, "descriptor")
    }

    /// Binds every direct cell from a `GotId`-keyed process-image address map.
    ///
    /// # Errors
    ///
    /// Rejects a direct slot whose identity is missing from `addresses`.
    pub fn bind_direct(&mut self, addresses: &BTreeMap<GotId, usize>) -> Result<()> {
        self.bind_identities(SlotKind::Direct, addresses, "direct variable")
    }

    /// Binds every import cell from the host service table.
    ///
    /// `resolve` returns the address of the ABI 2 service entry for a service
    /// ID, or `None` when the host does not provide it.
    ///
    /// # Errors
    ///
    /// Rejects an import slot whose service is not provided by the host.
    pub fn bind_imports(&mut self, resolve: impl Fn(u8) -> Option<usize>) -> Result<()> {
        let bindings: Vec<(u32, usize)> = self
            .slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Import)
            .map(|slot| {
                resolve(slot.service)
                    .map(|address| (slot.slot, address))
                    .ok_or_else(|| {
                        Error::new(
                            codes::ONLINE_BINDING,
                            format!(
                                "import slot {} has no host service {}",
                                slot.slot, slot.service
                            ),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        for (slot, address) in bindings {
            self.set_cell(slot, address)?;
        }
        Ok(())
    }

    fn bind_identities(
        &mut self,
        kind: SlotKind,
        addresses: &BTreeMap<GotId, usize>,
        what: &str,
    ) -> Result<()> {
        let bindings: Vec<(u32, usize)> = self
            .slots
            .iter()
            .filter(|slot| slot.kind == kind)
            .map(|slot| {
                addresses
                    .get(&slot.got_id)
                    .copied()
                    .map(|address| (slot.slot, address))
                    .ok_or_else(|| {
                        Error::new(
                            codes::ONLINE_BINDING,
                            format!(
                                "{what} slot {} has no address for {}",
                                slot.slot, slot.got_id
                            ),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        for (slot, address) in bindings {
            self.set_cell(slot, address)?;
        }
        Ok(())
    }

    fn index(&self, slot: u32) -> Result<usize> {
        let index = usize::try_from(slot).map_err(|_| {
            Error::new(
                codes::ONLINE_BINDING,
                format!("slot {slot} exceeds the platform limit"),
            )
        })?;
        if index >= self.cells.len() {
            return Err(Error::new(
                codes::ONLINE_BINDING,
                format!("slot {slot} is outside the {}-cell GOT", self.cells.len()),
            ));
        }
        Ok(index)
    }
}

fn validate_slot_table(slots: &[SlotRecord]) -> Result<()> {
    let mut group: Option<SlotKind> = None;
    let mut first_in_group = true;
    let mut last_got_id = GotId::ZERO;
    let mut last_service = 0_u8;
    for (index, slot) in slots.iter().enumerate() {
        if usize::try_from(slot.slot).ok() != Some(index) {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "slot table must be contiguous from zero (expected {index}, found {})",
                    slot.slot
                ),
            ));
        }
        match group {
            Some(kind) if slot.kind != kind => {
                if slot.kind.tag() < kind.tag() {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        "slot groups must appear in data, function, descriptor, direct, import order",
                    ));
                }
                group = Some(slot.kind);
                first_in_group = true;
                last_got_id = GotId::ZERO;
                last_service = 0;
            }
            None => {
                group = Some(slot.kind);
                first_in_group = true;
            }
            Some(_) => {}
        }
        match slot.kind {
            SlotKind::Import => {
                if slot.service == 0 || slot.service > 13 {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        format!(
                            "import slot {} service ID {} is outside the ABI 2 service table",
                            slot.slot, slot.service
                        ),
                    ));
                }
                if !slot.got_id.is_zero() {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        format!("import slot {} has a nonzero GotId", slot.slot),
                    ));
                }
                if !first_in_group && slot.service <= last_service {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        "import slots must be sorted by service ID",
                    ));
                }
                last_service = slot.service;
            }
            _ => {
                if slot.service != 0 {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        format!(
                            "non-import slot {} has service ID {}",
                            slot.slot, slot.service
                        ),
                    ));
                }
                if slot.got_id.is_zero() {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        format!("slot {} has a zero GotId", slot.slot),
                    ));
                }
                if !first_in_group && slot.got_id <= last_got_id {
                    return Err(Error::new(
                        codes::MODULE_INVALID,
                        "slots within a group must be strictly sorted by GotId",
                    ));
                }
                last_got_id = slot.got_id;
            }
        }
        first_in_group = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::{RetiredCause, RootKind, RootRecord};

    use crate::test_support;

    fn slot(slot: u32, kind: SlotKind, service: u8, id: u8) -> SlotRecord {
        SlotRecord {
            slot,
            kind,
            service,
            got_id: if kind == SlotKind::Import {
                GotId::ZERO
            } else {
                GotId::from_bytes([id; 16])
            },
        }
    }

    fn record(id: u8, size: u64, align: u32) -> RootRecord {
        RootRecord {
            got_id: GotId::from_bytes([id; 16]),
            key: format!("_ROOT::ROOT{id}"),
            previous_keys: Vec::new(),
            root_kind: RootKind::Global,
            owner_type: None,
            field_path: String::new(),
            type_key: "DINT".into(),
            layout_hash: GotId::from_bytes([0x55; 16]),
            size,
            align,
            retention: stc_got::Retention::None,
            initializer: stc_got::InitializerKind::Zero,
            initializer_digest: None,
            source: None,
            slot: 0,
        }
    }

    #[test]
    fn cells_are_stable_across_binding() {
        let slots = [
            slot(0, SlotKind::Data, 0, 1),
            slot(1, SlotKind::Function, 0, 2),
            slot(2, SlotKind::Import, 7, 0),
        ];
        let mut got = Got::new(&slots).expect("got");
        let addresses: Vec<usize> = (0..3)
            .map(|index| got.cell_address(u32::try_from(index).unwrap()).unwrap() as usize)
            .collect();
        assert_eq!(got.unbound_slots(), vec![0, 1, 2]);

        got.set_cell(0, 0x1000).unwrap();
        got.set_cell(1, 0x2000).unwrap();
        got.set_cell(2, 0x3000).unwrap();
        let after: Vec<usize> = (0..3)
            .map(|index| got.cell_address(u32::try_from(index).unwrap()).unwrap() as usize)
            .collect();
        assert_eq!(addresses, after);
        assert_eq!(got.cell_value(0).unwrap(), 0x1000);
        assert_eq!(got.cell_value(1).unwrap(), 0x2000);
        assert_eq!(got.cell_value(2).unwrap(), 0x3000);
        assert!(got.is_bound(0).unwrap());
        assert!(got.unbound_slots().is_empty());
        assert_eq!(got.slot_table().len(), 3);
        assert_eq!(
            got.slot_table_pointer(),
            got.slot_table().as_ptr().cast::<*mut u8>()
        );
        assert_eq!(got.slot_table()[0], got.cell_address(0).unwrap() as usize);
    }

    #[test]
    fn rejects_noncanonical_slot_tables() {
        let mut import_with_id = slot(0, SlotKind::Import, 7, 0);
        import_with_id.got_id = GotId::from_bytes([1; 16]);
        let cases = [
            vec![slot(1, SlotKind::Data, 0, 1)],
            vec![
                slot(0, SlotKind::Function, 0, 2),
                slot(1, SlotKind::Data, 0, 1),
            ],
            vec![slot(0, SlotKind::Data, 0, 2), slot(1, SlotKind::Data, 0, 1)],
            vec![slot(0, SlotKind::Data, 1, 1)],
            vec![slot(0, SlotKind::Data, 0, 0)],
            vec![slot(0, SlotKind::Import, 0, 0)],
            vec![slot(0, SlotKind::Import, 14, 0)],
            vec![import_with_id],
            vec![
                slot(0, SlotKind::Import, 8, 0),
                slot(1, SlotKind::Import, 7, 0),
            ],
        ];
        for slots in cases {
            assert!(Got::new(&slots).is_err(), "accepted {slots:?}");
        }
    }

    #[test]
    fn accepts_the_debug_check_import_service() {
        let slots = [slot(0, SlotKind::Import, 13, 0)];
        let got = Got::new(&slots).expect("the ABI 2.3 debug_check import is valid");
        assert_eq!(got.slot_count(), 1);
    }

    #[test]
    fn binds_every_cell_kind_by_identity() {
        let artifacts = test_support::artifacts(test_support::SIMPLE, "simple");
        let manifest = &artifacts.manifest;
        let mut roots = RootRegistry::new();
        for root in manifest
            .roots
            .iter()
            .filter(|root| root.root_kind != RootKind::Direct)
        {
            roots.allocate(root, 1).expect("allocate root");
        }
        let mut got = Got::new(&manifest.slots).expect("got");
        got.bind_data_roots(&roots).expect("data roots");

        let functions: BTreeMap<GotId, usize> = manifest
            .functions
            .iter()
            .map(|function| (function.got_id, 0x1000))
            .collect();
        got.bind_functions(&functions).expect("functions");
        got.bind_imports(|service| Some(0x2000 + usize::from(service)))
            .expect("imports");

        for slot in manifest
            .slots
            .iter()
            .filter(|slot| slot.kind != SlotKind::Direct)
        {
            let expected = match slot.kind {
                SlotKind::Data => roots.address(slot.got_id).unwrap(),
                SlotKind::Function => 0x1000,
                SlotKind::Import => 0x2000 + usize::from(slot.service),
                _ => unreachable!("no descriptor slots in the M4.5 scalar surface"),
            };
            assert_eq!(got.cell_value(slot.slot).unwrap(), expected, "{slot:?}");
        }
        assert!(got.unbound_slots().is_empty());
    }

    #[test]
    fn fresh_root_mask_indexes_data_slots_by_slot_number() {
        let artifacts = test_support::artifacts(test_support::SIMPLE, "simple");
        let manifest = &artifacts.manifest;
        let got = Got::new(&manifest.slots).unwrap();
        let first = manifest
            .slots
            .iter()
            .find(|slot| slot.kind == SlotKind::Data)
            .expect("data slot");
        let fresh: BTreeSet<GotId> = [first.got_id].into_iter().collect();
        let mask = got.fresh_root_mask(&fresh);
        assert_eq!(mask.len(), manifest.slots.len().div_ceil(8));
        for slot in &manifest.slots {
            let index = usize::try_from(slot.slot).unwrap();
            let set = mask[index / 8] & (1 << (index % 8)) != 0;
            assert_eq!(
                set,
                slot.kind == SlotKind::Data && slot.got_id == first.got_id,
                "{slot:?}"
            );
        }

        let all: BTreeSet<GotId> = manifest.roots.iter().map(|root| root.got_id).collect();
        let mask = got.fresh_root_mask(&all);
        for slot in &manifest.slots {
            let index = usize::try_from(slot.slot).unwrap();
            let set = mask[index / 8] & (1 << (index % 8)) != 0;
            assert_eq!(set, slot.kind == SlotKind::Data, "{slot:?}");
        }
    }

    #[test]
    fn binding_reports_missing_roots_functions_and_services() {
        let slots = [
            slot(0, SlotKind::Data, 0, 1),
            slot(1, SlotKind::Function, 0, 2),
            slot(2, SlotKind::Import, 7, 0),
        ];
        let mut got = Got::new(&slots).unwrap();
        let error = got.bind_data_roots(&RootRegistry::new()).unwrap_err();
        assert_eq!(error.code, codes::ONLINE_BINDING);
        let error = got.bind_functions(&BTreeMap::new()).unwrap_err();
        assert_eq!(error.code, codes::ONLINE_BINDING);
        let error = got.bind_imports(|_| None).unwrap_err();
        assert_eq!(error.code, codes::ONLINE_BINDING);
        assert_eq!(
            got.cell_address(99).unwrap_err().code,
            codes::ONLINE_BINDING
        );
        assert_eq!(got.cell_value(99).unwrap_err().code, codes::ONLINE_BINDING);
        assert_eq!(got.set_cell(99, 1).unwrap_err().code, codes::ONLINE_BINDING);
    }

    #[test]
    fn binds_descriptor_cells_and_hidden_roots() {
        let artifacts = test_support::artifacts(test_support::OOP, "oop");
        let manifest = &artifacts.manifest;
        let mut roots = RootRegistry::new();
        for root in manifest
            .roots
            .iter()
            .filter(|root| root.root_kind != RootKind::Direct)
        {
            roots.allocate(root, 1).expect("allocate root");
        }
        let mut got = Got::new(&manifest.slots).expect("got");
        got.bind_data_roots(&roots).expect("data roots");
        let functions: BTreeMap<GotId, usize> = manifest
            .functions
            .iter()
            .map(|function| (function.got_id, 0x1000))
            .collect();
        got.bind_functions(&functions).expect("functions");
        let descriptors: BTreeMap<GotId, usize> = manifest
            .descriptors
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.got_id, 0x3000 + index))
            .collect();
        assert!(!descriptors.is_empty());
        got.bind_descriptors(&descriptors).expect("descriptors");
        got.bind_direct(&BTreeMap::new()).expect("direct");
        got.bind_imports(|service| Some(0x2000 + usize::from(service)))
            .expect("imports");
        assert!(got.unbound_slots().is_empty(), "{:?}", got.unbound_slots());

        for slot in manifest
            .slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Descriptor)
        {
            assert_eq!(
                got.cell_value(slot.slot).unwrap(),
                descriptors[&slot.got_id]
            );
        }
        for root in manifest
            .roots
            .iter()
            .filter(|root| stc_got::hidden_root_marker(&root.key).is_some())
        {
            let slot = manifest
                .slots
                .iter()
                .find(|slot| slot.got_id == root.got_id)
                .expect("hidden root slot");
            assert_eq!(
                got.cell_value(slot.slot).unwrap(),
                roots.address(root.got_id).unwrap()
            );
        }
    }

    #[test]
    fn direct_cells_bind_to_process_image_addresses() {
        let slots = [slot(0, SlotKind::Direct, 0, 3)];
        let mut got = Got::new(&slots).unwrap();
        let mut addresses = BTreeMap::new();
        addresses.insert(GotId::from_bytes([3; 16]), 0x5000);
        got.bind_direct(&addresses).expect("direct");
        assert_eq!(got.cell_value(0).unwrap(), 0x5000);
    }

    #[test]
    fn quarantine_keeps_references_away_from_new_cells() {
        let mut roots = RootRegistry::new();
        let record = record(1, 4, 4);
        roots.allocate(&record, 1).unwrap();
        let storage = roots.address(record.got_id).unwrap();
        let slots = [slot(0, SlotKind::Data, 0, 1)];
        let mut got = Got::new(&slots).unwrap();
        got.bind_data_roots(&roots).unwrap();
        assert_eq!(got.cell_value(0).unwrap(), storage);

        roots
            .quarantine(record.got_id, RetiredCause::Dropped)
            .unwrap();
        assert_eq!(got.cell_value(0).unwrap(), storage);
        assert_eq!(roots.quarantined()[0].address(), storage);
    }

    #[test]
    fn root_data_cells_follow_the_registry() {
        let mut roots = RootRegistry::new();
        let record = record(1, 4, 4);
        roots.allocate(&record, 1).unwrap();
        let slots = [slot(0, SlotKind::Data, 0, 1)];
        let mut got = Got::new(&slots).unwrap();
        got.bind_data_roots(&roots).unwrap();
        let first = got.cell_value(0).unwrap();
        roots.get_mut(record.got_id).unwrap().storage_mut()[0] = 42;
        assert_eq!(first, roots.address(record.got_id).unwrap());
        assert_eq!(roots.storage(record.got_id).unwrap()[0], 42);
    }
}
