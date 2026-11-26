//! PDO capability model and constrained selection (WL-7 phase 3).
//!
//! The ESI declares which sync managers, PDOs and entries a device supports.
//! It does not decide the machine configuration: the engineer selects declared
//! PDOs and may remap only from the entries the accepted ESI declares for that
//! device and direction (D-269). Fixed and mandatory PDOs stay assigned, fixed
//! entries keep their PDO and order, entry identity and width are never
//! editable, and `PdoAssign`/`PdoConfig` gate whether a remap is possible at
//! all. A PDO without an explicit ESI sync-manager declaration is not assigned
//! by inference.
//!
//! Everything here is selected or validated from persisted facts and the ESI
//! capability model. It never reads hardware; live comparison is a separate,
//! policy-free step that reports `match`, `different`, `unverified` or
//! `not applicable` and never turns unavailable read-back into a match
//! (D-278).

use std::collections::{BTreeMap, BTreeSet};

use stc_got::json::Json;

use super::diagnostics::{EngineeringDiagnostic, codes};
use super::esi::model::{
    DcOpMode, DeviceDescription, MailboxCapabilities, Pdo, PdoDirection, SyncManager,
};
use super::topology::CheckResult;

/// An object-dictionary entry used by a PDO, addressed by index and subindex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryKey {
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
}

impl EntryKey {
    /// Creates an entry key.
    #[must_use]
    pub fn new(index: u16, subindex: u8) -> Self {
        Self { index, subindex }
    }

    /// Stable spelling used in structured output and diagnostics.
    #[must_use]
    pub fn describe(self) -> String {
        format!("0x{:04x}:{}", self.index, self.subindex)
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            ("subindex".into(), Json::unsigned(u64::from(self.subindex))),
        ])
    }
}

/// One entry as the accepted ESI declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredEntry {
    /// Object index and subindex.
    pub key: EntryKey,
    /// Declared bit length; never editable.
    pub bit_len: u8,
    /// Optional declared name.
    pub name: Option<String>,
    /// Optional declared data type.
    pub data_type: Option<String>,
    /// The declared PDO this entry was declared in.
    pub declared_in: u16,
    /// Direction of the declaring PDO.
    pub direction: PdoDirection,
}

impl DeclaredEntry {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("entry".into(), self.key.to_json()),
            ("bit_len".into(), Json::unsigned(u64::from(self.bit_len))),
            (
                "name".into(),
                Json::optional(self.name.as_ref().map(|name| Json::string(name.as_str()))),
            ),
            (
                "data_type".into(),
                Json::optional(
                    self.data_type
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "declared_in".into(),
                Json::unsigned(u64::from(self.declared_in)),
            ),
        ])
    }
}

/// One declared PDO alternative, including its constraints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoAlternative {
    /// Direction from the declaring element.
    pub direction: PdoDirection,
    /// PDO index.
    pub index: u16,
    /// Declared sync manager, when the ESI declares one.
    pub sm: Option<u8>,
    /// `Fixed="1"`: no reassignment, omission, reorder or move.
    pub fixed: bool,
    /// `Mandatory="1"`: the PDO must stay assigned.
    pub mandatory: bool,
    /// Sync managers this PDO excludes when assigned.
    pub excluded_sm: Vec<u8>,
    /// PDO indices this PDO excludes when assigned.
    pub exclude: Vec<u16>,
    /// Optional PDO name.
    pub name: Option<String>,
    /// Declared entries, in declaration order.
    pub entries: Vec<DeclaredEntry>,
}

impl PdoAlternative {
    fn from_pdo(pdo: &Pdo) -> Self {
        Self {
            direction: pdo.direction,
            index: pdo.index,
            sm: pdo.sm,
            fixed: pdo.fixed,
            mandatory: pdo.mandatory,
            excluded_sm: pdo.excluded_sm.clone(),
            exclude: pdo.exclude.clone(),
            name: pdo.name.clone(),
            entries: pdo
                .entries
                .iter()
                .map(|entry| DeclaredEntry {
                    key: EntryKey::new(entry.index, entry.subindex),
                    bit_len: entry.bit_len,
                    name: entry.name.clone(),
                    data_type: entry.data_type.clone(),
                    declared_in: pdo.index,
                    direction: pdo.direction,
                })
                .collect(),
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("direction".into(), Json::string(self.direction.as_str())),
            (
                "channel_direction".into(),
                Json::string(self.direction.channel_direction()),
            ),
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            (
                "sm".into(),
                Json::optional(self.sm.map(|sm| Json::unsigned(u64::from(sm)))),
            ),
            ("fixed".into(), Json::Bool(self.fixed)),
            ("mandatory".into(), Json::Bool(self.mandatory)),
            (
                "exclude".into(),
                Json::Array(
                    self.exclude
                        .iter()
                        .map(|index| Json::unsigned(u64::from(*index)))
                        .collect(),
                ),
            ),
            (
                "excluded_sm".into(),
                Json::Array(
                    self.excluded_sm
                        .iter()
                        .map(|sm| Json::unsigned(u64::from(*sm)))
                        .collect(),
                ),
            ),
            (
                "name".into(),
                Json::optional(self.name.as_ref().map(|name| Json::string(name.as_str()))),
            ),
            (
                "entries".into(),
                Json::Array(self.entries.iter().map(DeclaredEntry::to_json).collect()),
            ),
        ])
    }
}

/// The declared PDO/sync-manager capabilities of one device description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoCapabilities {
    /// Device name from the ESI.
    pub name: String,
    /// Device type name from the ESI.
    pub type_name: String,
    /// Exact identity basis.
    pub vendor_id: u32,
    /// Product code.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
    /// Digest of the accepted description.
    pub digest: String,
    /// Declared sync managers, in declaration order.
    pub sync_managers: Vec<SyncManager>,
    /// Declared `RxPdo` alternatives.
    pub rx_pdos: Vec<PdoAlternative>,
    /// Declared `TxPdo` alternatives.
    pub tx_pdos: Vec<PdoAlternative>,
    /// Mailbox/CoE capability gates.
    pub mailbox: MailboxCapabilities,
    /// Declared DC op modes, when the ESI declares a `<Dc>` section.
    pub dc_op_modes: Option<Vec<DcOpMode>>,
}

impl PdoCapabilities {
    /// Projects the capabilities of one accepted device description.
    #[must_use]
    pub fn from_device(device: &DeviceDescription) -> Self {
        Self {
            name: device.name.clone(),
            type_name: device.type_name.clone(),
            vendor_id: device.vendor_id,
            product_code: device.product_code,
            revision: device.revision,
            digest: device.digest.clone(),
            sync_managers: device.sync_managers.clone(),
            rx_pdos: device
                .rx_pdos
                .iter()
                .map(PdoAlternative::from_pdo)
                .collect(),
            tx_pdos: device
                .tx_pdos
                .iter()
                .map(PdoAlternative::from_pdo)
                .collect(),
            mailbox: device.mailbox,
            dc_op_modes: device.dc.as_ref().map(|dc| dc.op_modes.clone()),
        }
    }

    /// Every declared alternative, `Rx` before `Tx`, in declaration order.
    pub fn alternatives(&self) -> impl Iterator<Item = &PdoAlternative> {
        self.rx_pdos.iter().chain(self.tx_pdos.iter())
    }

    /// Returns the declared alternative with this index and direction.
    #[must_use]
    pub fn alternative(&self, index: u16, direction: PdoDirection) -> Option<&PdoAlternative> {
        let pdos = match direction {
            PdoDirection::Rx => &self.rx_pdos,
            PdoDirection::Tx => &self.tx_pdos,
        };
        pdos.iter().find(|pdo| pdo.index == index)
    }

    /// Every declaration of `key` in `direction`, in declaration order.
    pub fn declarations(
        &self,
        key: EntryKey,
        direction: PdoDirection,
    ) -> impl Iterator<Item = &DeclaredEntry> {
        let pdos = match direction {
            PdoDirection::Rx => &self.rx_pdos,
            PdoDirection::Tx => &self.tx_pdos,
        };
        pdos.iter()
            .flat_map(move |pdo| pdo.entries.iter())
            .filter(move |entry| entry.key == key)
    }

    /// Returns the declared entry, when the ESI declares it for this direction.
    #[must_use]
    pub fn declared_entry(&self, key: EntryKey, direction: PdoDirection) -> Option<&DeclaredEntry> {
        self.declarations(key, direction).next()
    }

    /// The engineer's starting proposal: every declared PDO with its declared
    /// entries, in declaration order. Applying it is an explicit action.
    #[must_use]
    pub fn default_selection(&self) -> PdoSelection {
        PdoSelection {
            pdos: self
                .alternatives()
                .map(|pdo| SelectedPdo {
                    direction: pdo.direction,
                    index: pdo.index,
                    sm: pdo.sm.unwrap_or(0),
                    fixed: pdo.fixed,
                    mandatory: pdo.mandatory,
                    exclude: pdo.exclude.clone(),
                    excluded_sm: pdo.excluded_sm.clone(),
                    entries: pdo
                        .entries
                        .iter()
                        .map(|entry| SelectedEntry {
                            index: entry.key.index,
                            subindex: entry.key.subindex,
                            bit_len: entry.bit_len,
                            declared_in: entry.declared_in,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Structured projection of every alternative with its constraints.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("name".into(), Json::string(self.name.as_str())),
            ("type".into(), Json::string(self.type_name.as_str())),
            (
                "vendor_id".into(),
                Json::unsigned(u64::from(self.vendor_id)),
            ),
            (
                "product_code".into(),
                Json::unsigned(u64::from(self.product_code)),
            ),
            ("revision".into(), Json::unsigned(u64::from(self.revision))),
            ("digest".into(), Json::string(self.digest.as_str())),
            (
                "sync_managers".into(),
                Json::Array(
                    self.sync_managers
                        .iter()
                        .map(|sync| {
                            Json::object([
                                ("index".into(), Json::unsigned(u64::from(sync.index))),
                                (
                                    "start_address".into(),
                                    Json::unsigned(u64::from(sync.start_address)),
                                ),
                                (
                                    "control_byte".into(),
                                    Json::unsigned(u64::from(sync.control_byte)),
                                ),
                                ("enable".into(), Json::Bool(sync.enable)),
                                (
                                    "name".into(),
                                    Json::optional(
                                        sync.name.as_ref().map(|name| Json::string(name.as_str())),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "rx_pdos".into(),
                Json::Array(self.rx_pdos.iter().map(PdoAlternative::to_json).collect()),
            ),
            (
                "tx_pdos".into(),
                Json::Array(self.tx_pdos.iter().map(PdoAlternative::to_json).collect()),
            ),
            (
                "dc".into(),
                match &self.dc_op_modes {
                    None => Json::Null,
                    Some(op_modes) => Json::object([(
                        "op_modes".into(),
                        Json::Array(op_modes.iter().map(dc_op_mode_json).collect()),
                    )]),
                },
            ),
            ("mailbox".into(), self.mailbox.to_json()),
        ])
    }
}

fn dc_op_mode_json(mode: &DcOpMode) -> Json {
    Json::object([
        ("index".into(), Json::unsigned(mode.index as u64)),
        ("name".into(), Json::string(mode.name.as_str())),
        (
            "description".into(),
            Json::optional(
                mode.description
                    .as_ref()
                    .map(|text| Json::string(text.as_str())),
            ),
        ),
        (
            "assign_activate".into(),
            Json::unsigned(u64::from(mode.assign_activate)),
        ),
        (
            "assign_activate_hex".into(),
            Json::string(format!("#x{:04x}", mode.assign_activate)),
        ),
        (
            "sync0_cycle".into(),
            Json::unsigned(u64::from(mode.sync0_cycle)),
        ),
        (
            "sync0_factor".into(),
            Json::optional(
                mode.sync0_factor
                    .map(|value| Json::unsigned(u64::from(value))),
            ),
        ),
        (
            "sync0_shift".into(),
            Json::signed(i64::from(mode.sync0_shift)),
        ),
        (
            "sync1_cycle".into(),
            Json::optional(
                mode.sync1_cycle
                    .map(|value| Json::unsigned(u64::from(value))),
            ),
        ),
        (
            "sync1_shift".into(),
            Json::optional(mode.sync1_shift.map(|value| Json::signed(i64::from(value)))),
        ),
    ])
}

/// One entry of a selected PDO, with the declared facts needed for offline
/// revalidation without the ESI (D-263, D-269).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedEntry {
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
    /// Declared bit length.
    pub bit_len: u8,
    /// The declared PDO this entry came from.
    pub declared_in: u16,
}

impl SelectedEntry {
    /// Object index and subindex.
    #[must_use]
    pub fn key(self) -> EntryKey {
        EntryKey::new(self.index, self.subindex)
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            ("subindex".into(), Json::unsigned(u64::from(self.subindex))),
            ("bit_len".into(), Json::unsigned(u64::from(self.bit_len))),
            (
                "declared_in".into(),
                Json::unsigned(u64::from(self.declared_in)),
            ),
        ])
    }
}

/// One selected PDO with its effective entries and constraint facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPdo {
    /// Direction from the declaring element.
    pub direction: PdoDirection,
    /// PDO index; never synthesized.
    pub index: u16,
    /// Selected sync manager, which must be the declared one.
    pub sm: u8,
    /// Persisted `Fixed` fact.
    pub fixed: bool,
    /// Persisted `Mandatory` fact.
    pub mandatory: bool,
    /// Persisted `Exclude` facts.
    pub exclude: Vec<u16>,
    /// Persisted `ExcludedSm` facts.
    pub excluded_sm: Vec<u8>,
    /// Effective entries, in mapping order.
    pub entries: Vec<SelectedEntry>,
}

impl SelectedPdo {
    /// Object keys of the effective entries, in mapping order.
    #[must_use]
    pub fn entry_keys(&self) -> Vec<EntryKey> {
        self.entries.iter().map(|entry| entry.key()).collect()
    }

    /// Total effective entry width in bits.
    #[must_use]
    pub fn bit_len(&self) -> u32 {
        self.entries
            .iter()
            .map(|entry| u32::from(entry.bit_len))
            .sum()
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("direction".into(), Json::string(self.direction.as_str())),
            ("index".into(), Json::unsigned(u64::from(self.index))),
            (
                "index_hex".into(),
                Json::string(format!("0x{:04x}", self.index)),
            ),
            ("sm".into(), Json::unsigned(u64::from(self.sm))),
            ("fixed".into(), Json::Bool(self.fixed)),
            ("mandatory".into(), Json::Bool(self.mandatory)),
            (
                "exclude".into(),
                Json::Array(
                    self.exclude
                        .iter()
                        .map(|index| Json::unsigned(u64::from(*index)))
                        .collect(),
                ),
            ),
            (
                "excluded_sm".into(),
                Json::Array(
                    self.excluded_sm
                        .iter()
                        .map(|sm| Json::unsigned(u64::from(*sm)))
                        .collect(),
                ),
            ),
            (
                "entries".into(),
                Json::Array(self.entries.iter().map(|entry| entry.to_json()).collect()),
            ),
        ])
    }
}

/// A complete selected PDO configuration for one device mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdoSelection {
    /// Selected PDOs. A PDO appears at most once.
    pub pdos: Vec<SelectedPdo>,
}

impl PdoSelection {
    /// Returns the selected PDO with this index and direction.
    #[must_use]
    pub fn pdo(&self, index: u16, direction: PdoDirection) -> Option<&SelectedPdo> {
        self.pdos
            .iter()
            .find(|pdo| pdo.index == index && pdo.direction == direction)
    }

    /// Iterates every effective entry.
    pub fn entries(&self) -> impl Iterator<Item = (&SelectedPdo, &SelectedEntry)> {
        self.pdos
            .iter()
            .flat_map(|pdo| pdo.entries.iter().map(move |entry| (pdo, entry)))
    }

    /// Total effective process-image width in bits.
    #[must_use]
    pub fn bit_len(&self) -> u32 {
        self.pdos.iter().map(SelectedPdo::bit_len).sum()
    }

    /// Validates the selection against the ESI capability model (D-269).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-021` for an undeclared PDO or entry, an edited
    /// entry identity or width, a `Fixed`/`Mandatory`/`Exclude`/`ExcludedSm`
    /// violation, an inferred sync-manager assignment, a persisted constraint
    /// fact that disagrees with the ESI, or a remap the device's
    /// `PdoAssign`/`PdoConfig` capability does not allow.
    pub fn validate(
        &self,
        capabilities: &PdoCapabilities,
        position: u16,
    ) -> Result<(), EngineeringDiagnostic> {
        let mut selected = BTreeSet::new();
        for pdo in &self.pdos {
            if !selected.insert((pdo.direction, pdo.index)) {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    "each declared PDO at most once",
                    &format!(
                        "{} PDO 0x{:04x} selected twice",
                        pdo.direction.as_str(),
                        pdo.index
                    ),
                ));
            }
            let Some(alternative) = capabilities.alternative(pdo.index, pdo.direction) else {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    "a PDO the ESI declares for this direction",
                    &format!(
                        "{} PDO 0x{:04x} is not declared",
                        pdo.direction.as_str(),
                        pdo.index
                    ),
                ));
            };
            let Some(declared_sm) = alternative.sm else {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    "a PDO with an explicit ESI sync-manager declaration",
                    &format!(
                        "PDO 0x{:04x} declares no sync manager; assignment is not inferred",
                        pdo.index
                    ),
                ));
            };
            if pdo.sm != declared_sm {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    &format!("sync manager {declared_sm} for PDO 0x{:04x}", pdo.index),
                    &format!("sync manager {} in the selection", pdo.sm),
                ));
            }
            if pdo.fixed != alternative.fixed
                || pdo.mandatory != alternative.mandatory
                || pdo.exclude != alternative.exclude
                || pdo.excluded_sm != alternative.excluded_sm
            {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    &format!(
                        "the persisted constraints of PDO 0x{:04x} to match the ESI",
                        pdo.index
                    ),
                    "the persisted constraint facts disagree with the accepted ESI",
                ));
            }

            let mut entries = BTreeSet::new();
            for entry in &pdo.entries {
                let key = entry.key();
                if !entries.insert(key) {
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        "each declared entry at most once per PDO",
                        &format!(
                            "entry {} in PDO 0x{:04x} is mapped twice",
                            key.describe(),
                            pdo.index
                        ),
                    ));
                }
                let declarations: Vec<&DeclaredEntry> =
                    capabilities.declarations(key, pdo.direction).collect();
                let Some(declared) = declarations.first() else {
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        "an entry the ESI declares for this device and direction",
                        &format!(
                            "entry {} is not declared for {} PDOs; arbitrary object-dictionary \
                             mapping is not supported",
                            key.describe(),
                            pdo.direction.as_str()
                        ),
                    ));
                };
                if declarations
                    .iter()
                    .any(|declared| declared.bit_len != entry.bit_len)
                    || entry.bit_len == 0
                {
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        &format!(
                            "entry {} to keep its declared bit length {}",
                            key.describe(),
                            declared.bit_len
                        ),
                        &format!("bit length {} in the selection", entry.bit_len),
                    ));
                }
                if (alternative.fixed || alternative.mandatory) && entry.declared_in != pdo.index {
                    let constraint = if alternative.fixed {
                        "fixed"
                    } else {
                        "mandatory"
                    };
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        &format!(
                            "entry {} to stay in {constraint} PDO 0x{:04x}",
                            key.describe(),
                            pdo.index
                        ),
                        &format!("declared in PDO 0x{:04x}", entry.declared_in),
                    ));
                }
            }

            if alternative.fixed {
                let declared: Vec<EntryKey> =
                    alternative.entries.iter().map(|entry| entry.key).collect();
                if pdo.entry_keys() != declared {
                    let observed = pdo
                        .entry_keys()
                        .iter()
                        .map(|key| key.describe())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        &format!(
                            "fixed PDO 0x{:04x} to keep its declared entries in order",
                            pdo.index
                        ),
                        &format!("entries {observed}"),
                    ));
                }
            } else if alternative.mandatory {
                for declared in &alternative.entries {
                    if !entries.contains(&declared.key) {
                        return Err(selection_error(
                            "pdo.validate",
                            &capabilities.type_name,
                            position,
                            &format!(
                                "mandatory entry {} to stay included in PDO 0x{:04x}",
                                declared.key.describe(),
                                pdo.index
                            ),
                            "the entry was omitted",
                        ));
                    }
                }
            }
        }

        let mut sync_directions = BTreeMap::new();
        for pdo in &self.pdos {
            if let Some(previous) = sync_directions.insert(pdo.sm, pdo.direction)
                && previous != pdo.direction
            {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    "one process-data direction per sync manager",
                    &format!(
                        "sync manager {} is used by both {} and {} PDOs",
                        pdo.sm,
                        previous.as_str(),
                        pdo.direction.as_str()
                    ),
                ));
            }
        }

        for alternative in capabilities.alternatives() {
            let key = (alternative.direction, alternative.index);
            if alternative.fixed && !selected.contains(&key) {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    &format!("fixed PDO 0x{:04x} to stay assigned", alternative.index),
                    "the PDO was omitted",
                ));
            }
            if alternative.mandatory && !selected.contains(&key) {
                return Err(selection_error(
                    "pdo.validate",
                    &capabilities.type_name,
                    position,
                    &format!("mandatory PDO 0x{:04x} to stay assigned", alternative.index),
                    "the PDO was omitted",
                ));
            }
        }

        for pdo in &self.pdos {
            let Some(alternative) = capabilities.alternative(pdo.index, pdo.direction) else {
                continue;
            };
            for excluded in &alternative.exclude {
                if selected.contains(&(pdo.direction, *excluded)) {
                    return Err(selection_error(
                        "pdo.validate",
                        &capabilities.type_name,
                        position,
                        &format!(
                            "PDO 0x{:04x} and its excluded PDO 0x{:04x} not both assigned",
                            pdo.index, excluded
                        ),
                        "both PDOs are selected",
                    ));
                }
            }
            for excluded_sm in &alternative.excluded_sm {
                for other in &self.pdos {
                    if other.direction == pdo.direction
                        && other.index != pdo.index
                        && other.sm == *excluded_sm
                    {
                        return Err(selection_error(
                            "pdo.validate",
                            &capabilities.type_name,
                            position,
                            &format!(
                                "PDO 0x{:04x} with sync manager {excluded_sm} excluded",
                                pdo.index
                            ),
                            &format!(
                                "PDO 0x{:04x} uses excluded sync manager {excluded_sm}",
                                other.index
                            ),
                        ));
                    }
                }
            }
        }

        let (assignment_changed, mapping_changed) = self.deviations(capabilities);
        if assignment_changed && !capabilities.mailbox.pdo_assign {
            return Err(selection_error(
                "pdo.validate",
                &capabilities.type_name,
                position,
                "a device that declares `PdoAssign` for a changed PDO assignment",
                "the ESI does not declare `PdoAssign`",
            )
            .with_capability("pdo-assign"));
        }
        if mapping_changed && !capabilities.mailbox.pdo_config {
            return Err(selection_error(
                "pdo.validate",
                &capabilities.type_name,
                position,
                "a device that declares `PdoConfig` for a remapped PDO",
                "the ESI does not declare `PdoConfig`",
            )
            .with_capability("pdo-config"));
        }
        Ok(())
    }

    /// Structural validation without the ESI, using only persisted facts.
    ///
    /// The compiler revalidates these facts offline (Phase 6); the ESI is not
    /// read at build or runtime (D-263).
    ///
    /// # Errors
    ///
    /// Returns `STC-E-IO-ETH-021` for a duplicated PDO or entry, a zero bit
    /// length, or an entry with no declared originating PDO.
    pub fn validate_facts(&self, device: &str, position: u16) -> Result<(), EngineeringDiagnostic> {
        let mut pdos = BTreeSet::new();
        for pdo in &self.pdos {
            let key = (pdo.direction, pdo.index);
            if !pdos.insert(key) {
                return Err(selection_error(
                    "pdo.validate",
                    device,
                    position,
                    "each PDO at most once",
                    &format!("PDO 0x{:04x} appears twice", pdo.index),
                ));
            }
            let mut entries = BTreeSet::new();
            for entry in &pdo.entries {
                if !entries.insert(entry.key()) {
                    return Err(selection_error(
                        "pdo.validate",
                        device,
                        position,
                        "each entry at most once per PDO",
                        &format!(
                            "entry {} in PDO 0x{:04x} is mapped twice",
                            entry.key().describe(),
                            pdo.index
                        ),
                    ));
                }
                if entry.bit_len == 0 {
                    return Err(selection_error(
                        "pdo.validate",
                        device,
                        position,
                        "a declared non-zero bit length",
                        &format!("entry {} has bit length 0", entry.key().describe()),
                    ));
                }
                if entry.declared_in == 0 {
                    return Err(selection_error(
                        "pdo.validate",
                        device,
                        position,
                        "the declared originating PDO for every entry",
                        &format!("entry {} has no declared PDO", entry.key().describe()),
                    ));
                }
            }
        }
        let mut sync_directions = BTreeMap::new();
        for pdo in &self.pdos {
            if let Some(previous) = sync_directions.insert(pdo.sm, pdo.direction)
                && previous != pdo.direction
            {
                return Err(selection_error(
                    "pdo.validate",
                    device,
                    position,
                    "one process-data direction per sync manager",
                    &format!(
                        "sync manager {} is used by both {} and {} PDOs",
                        pdo.sm,
                        previous.as_str(),
                        pdo.direction.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Whether the selection changes PDO assignment and/or mapping relative to
    /// the ESI declaration. Changing either needs the matching CoE capability.
    #[must_use]
    pub fn deviations(&self, capabilities: &PdoCapabilities) -> (bool, bool) {
        let default = capabilities.default_selection();
        let assignment = |selection: &Self| {
            let mut map: BTreeMap<(PdoDirection, u8), Vec<u16>> = BTreeMap::new();
            for pdo in &selection.pdos {
                map.entry((pdo.direction, pdo.sm))
                    .or_default()
                    .push(pdo.index);
            }
            map
        };
        let assignment_changed = assignment(self) != assignment(&default);
        let mut mapping_changed = false;
        for pdo in &self.pdos {
            if let Some(declared) = capabilities.alternative(pdo.index, pdo.direction) {
                let declared_keys: Vec<EntryKey> =
                    declared.entries.iter().map(|entry| entry.key).collect();
                if pdo.entry_keys() != declared_keys {
                    mapping_changed = true;
                }
            }
            if pdo
                .entries
                .iter()
                .any(|entry| entry.declared_in != pdo.index)
            {
                mapping_changed = true;
            }
        }
        (assignment_changed, mapping_changed)
    }

    /// Compares the effective selection with live assignment/mapping read-back.
    ///
    /// A PDO or sync manager with no read-back record is `unverified`, never a
    /// match (D-278).
    #[must_use]
    pub fn verify(&self, live: &LivePdoState) -> PdoVerification {
        let mut checks = Vec::new();
        let mut sync_managers: Vec<(PdoDirection, u8)> = self
            .pdos
            .iter()
            .map(|pdo| (pdo.direction, pdo.sm))
            .collect();
        sync_managers.sort_unstable();
        sync_managers.dedup();
        for (direction, sm) in sync_managers {
            let expected: Vec<u16> = self
                .pdos
                .iter()
                .filter(|pdo| pdo.direction == direction && pdo.sm == sm)
                .map(|pdo| pdo.index)
                .collect();
            let expected_text = render_indices(&expected);
            match live.assignment_for(sm) {
                None => checks.push(PdoCheck {
                    field: "assignment",
                    subject: format!("{} sync manager {sm}", direction.as_str()),
                    result: CheckResult::Unverified,
                    expected: Some(expected_text),
                    observed: None,
                }),
                Some(observed) => {
                    let result = if observed == expected.as_slice() {
                        CheckResult::Match
                    } else {
                        CheckResult::Different
                    };
                    checks.push(PdoCheck {
                        field: "assignment",
                        subject: format!("{} sync manager {sm}", direction.as_str()),
                        result,
                        expected: Some(expected_text),
                        observed: Some(render_indices(observed)),
                    });
                }
            }
        }
        for pdo in &self.pdos {
            let expected = pdo.entry_keys();
            let expected_text = render_keys(&expected);
            match live.mapping_for(pdo.index) {
                None => checks.push(PdoCheck {
                    field: "mapping",
                    subject: format!("PDO 0x{:04x}", pdo.index),
                    result: CheckResult::Unverified,
                    expected: Some(expected_text),
                    observed: None,
                }),
                Some(observed) => {
                    let observed_keys: Vec<EntryKey> =
                        observed.iter().map(|entry| entry.key()).collect();
                    let widths_match = observed
                        .iter()
                        .zip(&pdo.entries)
                        .all(|(live, entry)| live.bit_len == entry.bit_len);
                    let result = if observed_keys == expected && widths_match {
                        CheckResult::Match
                    } else {
                        CheckResult::Different
                    };
                    checks.push(PdoCheck {
                        field: "mapping",
                        subject: format!("PDO 0x{:04x}", pdo.index),
                        result,
                        expected: Some(expected_text),
                        observed: Some(render_live_entries(observed)),
                    });
                }
            }
        }
        let result = checks
            .iter()
            .fold(CheckResult::NotApplicable, |worst, check| {
                worst.worst(check.result)
            });
        PdoVerification { checks, result }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::Array(self.pdos.iter().map(SelectedPdo::to_json).collect())
    }
}

/// One generated operation of the diagnostic CoE view (D-274).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoeOperation {
    /// Zero-based position in the generated view.
    pub sequence: u32,
    /// Object index and subindex the operation would address.
    pub object: EntryKey,
    /// Generated purpose, for example `assignment-clear` or `mapping-count`.
    pub purpose: &'static str,
    /// Value the equivalent operation would write.
    pub value: CoeValue,
}

impl CoeOperation {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("sequence".into(), Json::unsigned(u64::from(self.sequence))),
            ("object".into(), self.object.to_json()),
            ("purpose".into(), Json::string(self.purpose)),
            ("value".into(), self.value.to_json()),
        ])
    }
}

/// A value of one generated CoE operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoeValue {
    /// An unsigned count or PDO index.
    Unsigned(u64),
    /// One packed PDO mapping entry.
    Mapping {
        /// Object index.
        index: u16,
        /// Object subindex.
        subindex: u8,
        /// Declared bit length.
        bit_len: u8,
    },
}

impl CoeValue {
    /// The packed `0x16xx`/`0x1Axx` mapping value.
    #[must_use]
    pub fn packed(self) -> Option<u32> {
        match self {
            Self::Unsigned(_) => None,
            Self::Mapping {
                index,
                subindex,
                bit_len,
            } => Some((u32::from(index) << 16) | (u32::from(subindex) << 8) | u32::from(bit_len)),
        }
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        match self {
            Self::Unsigned(value) => Json::object([
                ("type".into(), Json::string("unsigned")),
                ("value".into(), Json::unsigned(value)),
            ]),
            Self::Mapping {
                index,
                subindex,
                bit_len,
            } => Json::object([
                ("type".into(), Json::string("mapping")),
                ("index".into(), Json::unsigned(u64::from(index))),
                ("subindex".into(), Json::unsigned(u64::from(subindex))),
                ("bit_len".into(), Json::unsigned(u64::from(bit_len))),
                (
                    "packed".into(),
                    Json::unsigned(u64::from(
                        (u32::from(index) << 16) | (u32::from(subindex) << 8) | u32::from(bit_len),
                    )),
                ),
            ]),
        }
    }
}

/// Generated, non-editable diagnostic view of the CoE operations equivalent to
/// a selected PDO configuration (D-274).
///
/// The view is derived from the declarative [`PdoSelection`]. It is not a
/// startup sequence, cannot be authored or edited, and is not persisted as
/// configuration. IgH applies the selection through its native PDO path, and
/// generic startup operations must not own the objects shown here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoeView {
    /// Generated operations in a deterministic order.
    pub operations: Vec<CoeOperation>,
}

impl CoeView {
    /// Generates the diagnostic view from a selected PDO configuration.
    #[must_use]
    pub fn from_selection(selection: &PdoSelection) -> Self {
        let mut groups: BTreeMap<(PdoDirection, u8), Vec<&SelectedPdo>> = BTreeMap::new();
        for pdo in &selection.pdos {
            groups.entry((pdo.direction, pdo.sm)).or_default().push(pdo);
        }
        let mut operations = Vec::new();
        for ((_, sm), mut pdos) in groups {
            pdos.sort_by_key(|pdo| pdo.index);
            // PDO assignment objects start at 0x1C10 for sync manager 0.
            let assignment = EntryKey::new(0x1c10 + u16::from(sm), 0);
            let mut push = |object: EntryKey, purpose: &'static str, value: CoeValue| {
                operations.push(CoeOperation {
                    sequence: u32::try_from(operations.len()).unwrap_or(u32::MAX),
                    object,
                    purpose,
                    value,
                });
            };
            push(assignment, "assignment-clear", CoeValue::Unsigned(0));
            for (position, pdo) in pdos.iter().enumerate() {
                push(
                    EntryKey::new(
                        assignment.index,
                        u8::try_from(position + 1).unwrap_or(u8::MAX),
                    ),
                    "assignment",
                    CoeValue::Unsigned(u64::from(pdo.index)),
                );
            }
            push(
                assignment,
                "assignment-count",
                CoeValue::Unsigned(pdos.len() as u64),
            );
            for pdo in pdos {
                let mapping = EntryKey::new(pdo.index, 0);
                push(mapping, "mapping-clear", CoeValue::Unsigned(0));
                for (position, entry) in pdo.entries.iter().enumerate() {
                    push(
                        EntryKey::new(pdo.index, u8::try_from(position + 1).unwrap_or(u8::MAX)),
                        "mapping",
                        CoeValue::Mapping {
                            index: entry.index,
                            subindex: entry.subindex,
                            bit_len: entry.bit_len,
                        },
                    );
                }
                push(
                    mapping,
                    "mapping-count",
                    CoeValue::Unsigned(pdo.entries.len() as u64),
                );
            }
        }
        Self { operations }
    }

    /// Structured projection. `editable` is always `false`.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("kind".into(), Json::string("pdo-coe-diagnostic")),
            ("editable".into(), Json::Bool(false)),
            (
                "operations".into(),
                Json::Array(self.operations.iter().map(CoeOperation::to_json).collect()),
            ),
        ])
    }

    /// Deterministic document text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.to_json().render()
    }
}

fn render_indices(indices: &[u16]) -> String {
    indices
        .iter()
        .map(|index| format!("0x{index:04x}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_keys(keys: &[EntryKey]) -> String {
    keys.iter()
        .map(|key| key.describe())
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_live_entries(entries: &[LiveEntry]) -> String {
    entries
        .iter()
        .map(|entry| format!("{} ({} bit)", entry.key().describe(), entry.bit_len))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One named field of a live PDO comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoCheck {
    /// Field name: `assignment` or `mapping`.
    pub field: &'static str,
    /// Sync manager or PDO the check concerns.
    pub subject: String,
    /// Check result.
    pub result: CheckResult,
    /// Effective configuration value.
    pub expected: Option<String>,
    /// Live read-back value, when available.
    pub observed: Option<String>,
}

impl PdoCheck {
    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("field".into(), Json::string(self.field)),
            ("subject".into(), Json::string(self.subject.as_str())),
            ("result".into(), self.result.to_json()),
            (
                "expected".into(),
                Json::optional(
                    self.expected
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
            (
                "observed".into(),
                Json::optional(
                    self.observed
                        .as_ref()
                        .map(|value| Json::string(value.as_str())),
                ),
            ),
        ])
    }
}

/// Result of comparing an effective PDO configuration with live read-back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoVerification {
    /// Field checks in a stable order.
    pub checks: Vec<PdoCheck>,
    /// Overall result: the most significant field result.
    pub result: CheckResult,
}

impl PdoVerification {
    /// Whether every check matched.
    #[must_use]
    pub fn is_match(&self) -> bool {
        self.result.is_clean() && !self.checks.is_empty()
    }

    /// Whether any required fact could not be read back.
    #[must_use]
    pub fn is_unverified(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.result == CheckResult::Unverified)
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("result".into(), self.result.to_json()),
            (
                "checks".into(),
                Json::Array(self.checks.iter().map(PdoCheck::to_json).collect()),
            ),
        ])
    }
}

/// One live entry read back from the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveEntry {
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
    /// Live bit length.
    pub bit_len: u8,
}

impl LiveEntry {
    /// Creates a live entry.
    #[must_use]
    pub fn new(index: u16, subindex: u8, bit_len: u8) -> Self {
        Self {
            index,
            subindex,
            bit_len,
        }
    }

    /// Object index and subindex.
    #[must_use]
    pub fn key(self) -> EntryKey {
        EntryKey::new(self.index, self.subindex)
    }
}

/// Live PDO assignment of one sync manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivePdoAssignment {
    /// Sync-manager index.
    pub sm: u8,
    /// Assigned PDO indices, in assignment order.
    pub pdos: Vec<u16>,
}

impl LivePdoAssignment {
    /// Creates an assignment record.
    #[must_use]
    pub fn new(sm: u8, pdos: Vec<u16>) -> Self {
        Self { sm, pdos }
    }
}

/// Live PDO mapping of one PDO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivePdoMapping {
    /// PDO index.
    pub pdo: u16,
    /// Mapped entries, in mapping order.
    pub entries: Vec<LiveEntry>,
}

impl LivePdoMapping {
    /// Creates a mapping record.
    #[must_use]
    pub fn new(pdo: u16, entries: Vec<LiveEntry>) -> Self {
        Self { pdo, entries }
    }
}

/// Observed PDO assignment/mapping for one position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivePdoState {
    /// Configured machine slot.
    pub position: u16,
    /// Assignment records, one per readable sync manager.
    pub assignment: Vec<LivePdoAssignment>,
    /// Mapping records, one per readable PDO.
    pub mapping: Vec<LivePdoMapping>,
}

impl LivePdoState {
    /// Creates an empty observation for one position.
    #[must_use]
    pub fn new(position: u16) -> Self {
        Self {
            position,
            assignment: Vec::new(),
            mapping: Vec::new(),
        }
    }

    /// Models the live state a device accepts a selection as.
    #[must_use]
    pub fn from_selection(position: u16, selection: &PdoSelection) -> Self {
        let mut state = Self::new(position);
        let mut groups: BTreeMap<u8, Vec<u16>> = BTreeMap::new();
        for pdo in &selection.pdos {
            groups.entry(pdo.sm).or_default().push(pdo.index);
            state.set_mapping(LivePdoMapping::new(
                pdo.index,
                pdo.entries
                    .iter()
                    .map(|entry| LiveEntry::new(entry.index, entry.subindex, entry.bit_len))
                    .collect(),
            ));
        }
        for (sm, pdos) in groups {
            state.set_assignment(LivePdoAssignment::new(sm, pdos));
        }
        state
    }

    /// Adds or replaces an assignment record.
    pub fn set_assignment(&mut self, assignment: LivePdoAssignment) {
        match self
            .assignment
            .iter_mut()
            .find(|record| record.sm == assignment.sm)
        {
            Some(existing) => *existing = assignment,
            None => self.assignment.push(assignment),
        }
    }

    /// Adds or replaces a mapping record.
    pub fn set_mapping(&mut self, mapping: LivePdoMapping) {
        match self
            .mapping
            .iter_mut()
            .find(|record| record.pdo == mapping.pdo)
        {
            Some(existing) => *existing = mapping,
            None => self.mapping.push(mapping),
        }
    }

    /// The assignment read back for `sm`, when readable.
    #[must_use]
    pub fn assignment_for(&self, sm: u8) -> Option<&[u16]> {
        self.assignment
            .iter()
            .find(|record| record.sm == sm)
            .map(|record| record.pdos.as_slice())
    }

    /// The mapping read back for `pdo`, when readable.
    #[must_use]
    pub fn mapping_for(&self, pdo: u16) -> Option<&[LiveEntry]> {
        self.mapping
            .iter()
            .find(|record| record.pdo == pdo)
            .map(|record| record.entries.as_slice())
    }

    /// Structured projection.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object([
            ("position".into(), Json::unsigned(u64::from(self.position))),
            (
                "assignment".into(),
                Json::Array(
                    self.assignment
                        .iter()
                        .map(|record| {
                            Json::object([
                                ("sm".into(), Json::unsigned(u64::from(record.sm))),
                                (
                                    "pdos".into(),
                                    Json::Array(
                                        record
                                            .pdos
                                            .iter()
                                            .map(|index| Json::unsigned(u64::from(*index)))
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "mapping".into(),
                Json::Array(
                    self.mapping
                        .iter()
                        .map(|record| {
                            Json::object([
                                ("pdo".into(), Json::unsigned(u64::from(record.pdo))),
                                (
                                    "entries".into(),
                                    Json::Array(
                                        record
                                            .entries
                                            .iter()
                                            .map(|entry| {
                                                Json::object([
                                                    (
                                                        "index".into(),
                                                        Json::unsigned(u64::from(entry.index)),
                                                    ),
                                                    (
                                                        "subindex".into(),
                                                        Json::unsigned(u64::from(entry.subindex)),
                                                    ),
                                                    (
                                                        "bit_len".into(),
                                                        Json::unsigned(u64::from(entry.bit_len)),
                                                    ),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// Whether the selection equals the ESI declaration (no assignment or mapping
/// change) for every PDO.
#[must_use]
pub fn is_declared_default(selection: &PdoSelection, capabilities: &PdoCapabilities) -> bool {
    let (assignment, mapping) = selection.deviations(capabilities);
    !assignment && !mapping
}

/// Builds the diagnostic for a remap the live device refused (D-280).
///
/// The master or device reports the refusal; this function only turns it into
/// the stable `STC-E-IO-ETH-022` payload. Missing read-back is not a refusal
/// and stays [`CheckResult::Unverified`]; it never uses this code.
#[must_use]
pub fn remap_refused(
    device: &str,
    position: u16,
    subject: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::PDO_REMAP_REFUSED,
        "pdo.remap",
        "review the live assignment/mapping and the device state, then retry the \
         declared-entry remap in setup mode",
    )
    .with_device(device)
    .with_position(position)
    .with_expected(format!(
        "the device to accept the declared-entry remap for {subject}"
    ))
    .with_observed(observed)
}

fn selection_error(
    operation: &'static str,
    device: &str,
    position: u16,
    expected: &str,
    observed: &str,
) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::PDO_SELECTION,
        operation,
        "select declared PDOs and remap only entries the accepted ESI declares",
    )
    .with_device(device)
    .with_position(position)
    .with_expected(expected)
    .with_observed(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ethercat::esi::model::{
        DcOpMode, DeviceDescription, DistributedClock, Pdo, PdoEntry, SyncManager,
    };

    fn entry(index: u16, subindex: u8, bit_len: u8) -> PdoEntry {
        PdoEntry {
            index,
            subindex,
            bit_len,
            name: Some(format!("entry {index:04x}")),
            data_type: Some("UINT".into()),
        }
    }

    fn pdo(
        direction: PdoDirection,
        index: u16,
        sm: Option<u8>,
        fixed: bool,
        mandatory: bool,
        entries: Vec<PdoEntry>,
    ) -> Pdo {
        Pdo {
            direction,
            index,
            sm,
            fixed,
            mandatory,
            excluded_sm: Vec::new(),
            exclude: Vec::new(),
            name: Some(format!("pdo {index:04x}")),
            entries,
        }
    }

    fn device() -> DeviceDescription {
        DeviceDescription {
            vendor_id: 6,
            product_code: 0x101,
            revision: 1,
            name: "Servo".into(),
            type_name: "SERVO".into(),
            groups: Vec::new(),
            sync_managers: vec![
                SyncManager {
                    index: 2,
                    start_address: 0x1100,
                    control_byte: 0x64,
                    enable: true,
                    name: Some("Outputs".into()),
                },
                SyncManager {
                    index: 3,
                    start_address: 0x1300,
                    control_byte: 0x20,
                    enable: true,
                    name: Some("Inputs".into()),
                },
            ],
            rx_pdos: vec![
                pdo(
                    PdoDirection::Rx,
                    0x1600,
                    Some(2),
                    false,
                    false,
                    vec![entry(0x7000, 1, 16), entry(0x7000, 2, 16)],
                ),
                pdo(
                    PdoDirection::Rx,
                    0x1601,
                    Some(2),
                    false,
                    false,
                    vec![entry(0x7000, 3, 8)],
                ),
                pdo(
                    PdoDirection::Rx,
                    0x1602,
                    Some(2),
                    true,
                    true,
                    vec![entry(0x7000, 4, 8)],
                ),
            ],
            tx_pdos: vec![
                pdo(
                    PdoDirection::Tx,
                    0x1a00,
                    Some(3),
                    true,
                    true,
                    vec![entry(0x6000, 1, 16), entry(0x6000, 2, 16)],
                ),
                pdo(
                    PdoDirection::Tx,
                    0x1a01,
                    Some(3),
                    false,
                    false,
                    vec![entry(0x6000, 3, 1)],
                ),
            ],
            dc: Some(DistributedClock {
                op_modes: vec![DcOpMode {
                    index: 0,
                    name: "DC Sync0".into(),
                    description: None,
                    assign_activate: 0x0300,
                    sync0_cycle: 0,
                    sync0_factor: Some(2),
                    sync0_shift: 0,
                    sync1_cycle: None,
                    sync1_shift: None,
                }],
            }),
            mailbox: MailboxCapabilities {
                sdo_info: true,
                pdo_assign: true,
                pdo_config: true,
                complete_access: false,
            },
            dictionary: Vec::new(),
            digest: "digest".into(),
        }
    }

    fn capabilities() -> PdoCapabilities {
        PdoCapabilities::from_device(&device())
    }

    fn default_selection() -> PdoSelection {
        capabilities().default_selection()
    }

    #[test]
    fn capability_projection_lists_alternatives_and_entry_pools() {
        let capabilities = capabilities();
        assert_eq!(capabilities.alternatives().count(), 5);
        let rx = capabilities
            .alternative(0x1600, PdoDirection::Rx)
            .expect("rx");
        assert_eq!(rx.sm, Some(2));
        assert_eq!(rx.entries.len(), 2);
        assert_eq!(rx.entries[0].declared_in, 0x1600);
        assert_eq!(
            capabilities
                .declared_entry(EntryKey::new(0x6000, 1), PdoDirection::Tx)
                .map(|entry| entry.bit_len),
            Some(16)
        );
        assert!(
            capabilities
                .declared_entry(EntryKey::new(0x6000, 1), PdoDirection::Rx)
                .is_none()
        );
        let json = capabilities.to_json().render();
        assert!(json.contains("\"fixed\": true"));
        assert!(json.contains("\"entries\""));
        let selection = default_selection();
        assert_eq!(selection.pdos.len(), 5);
        assert_eq!(selection.bit_len(), 16 + 16 + 8 + 8 + 16 + 16 + 1);
        assert!(is_declared_default(&selection, &capabilities));
        assert!(selection.to_json().render().contains("\"declared_in\""));
    }

    #[test]
    fn the_default_selection_is_valid() {
        let capabilities = capabilities();
        default_selection()
            .validate(&capabilities, 0)
            .expect("default valid");
        default_selection()
            .validate_facts("SERVO", 0)
            .expect("facts valid");
    }

    #[test]
    fn optional_entries_may_be_omitted_and_reordered() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries.remove(1);
        selection.validate(&capabilities, 0).expect("omitted");
        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries.swap(0, 1);
        selection.validate(&capabilities, 0).expect("reordered");
    }

    #[test]
    fn an_optional_entry_may_move_between_declared_pdos_of_one_direction() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        let source = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        let moved = source.entries.remove(1);
        let target = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1601)
            .expect("pdo");
        target.entries.push(moved);
        selection.validate(&capabilities, 0).expect("moved");
        let (_, mapping) = selection.deviations(&capabilities);
        assert!(mapping);
    }

    #[test]
    fn fixed_pdos_and_entries_cannot_be_omitted_reordered_or_moved() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        selection.pdos.retain(|pdo| pdo.index != 0x1602);
        assert_eq!(
            selection
                .validate(&capabilities, 0)
                .expect_err("fixed")
                .code,
            codes::PDO_SELECTION
        );

        let mut selection = default_selection();
        selection.pdos.retain(|pdo| pdo.index != 0x1a00);
        assert!(selection.validate(&capabilities, 0).is_err());

        let mut selection = default_selection();
        let fixed = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1a00)
            .expect("pdo");
        fixed.entries.swap(0, 1);
        let error = selection.validate(&capabilities, 0).expect_err("reorder");
        assert!(error.expected.expect("expected").contains("in order"));

        let mut selection = default_selection();
        let fixed = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1602)
            .expect("pdo");
        let moved = fixed.entries.pop().expect("entry");
        let target = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        target.entries.push(moved);
        assert!(selection.validate(&capabilities, 0).is_err());
    }

    #[test]
    fn mandatory_entries_must_stay_included() {
        let mut servo = device();
        servo.rx_pdos[0].mandatory = true;
        let capabilities = PdoCapabilities::from_device(&servo);
        let mut selection = capabilities.default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries.remove(0);
        let error = selection.validate(&capabilities, 0).expect_err("omitted");
        assert!(error.expected.expect("expected").contains("mandatory"));
    }

    #[test]
    fn undeclared_entries_and_edited_widths_are_rejected() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries[0].index = 0x7001;
        let error = selection
            .validate(&capabilities, 0)
            .expect_err("undeclared");
        assert!(
            error
                .observed
                .expect("observed")
                .contains("arbitrary object-dictionary mapping")
        );

        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries[0].bit_len = 32;
        let error = selection.validate(&capabilities, 0).expect_err("width");
        assert!(error.expected.expect("expected").contains("bit length"));

        let mut selection = default_selection();
        selection.pdos.push(SelectedPdo {
            direction: PdoDirection::Rx,
            index: 0x1700,
            sm: 2,
            fixed: false,
            mandatory: false,
            exclude: Vec::new(),
            excluded_sm: Vec::new(),
            entries: Vec::new(),
        });
        assert!(selection.validate(&capabilities, 0).is_err());
    }

    #[test]
    fn duplicate_pdos_and_entries_are_rejected() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        let duplicate = selection.pdos[0].clone();
        selection.pdos.push(duplicate);
        assert!(selection.validate(&capabilities, 0).is_err());

        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        let duplicate = pdo.entries[0];
        pdo.entries.push(duplicate);
        assert!(selection.validate(&capabilities, 0).is_err());
        assert!(selection.validate_facts("SERVO", 0).is_err());
    }

    #[test]
    fn exclusions_are_hard_constraints() {
        let mut servo = device();
        servo.rx_pdos[1].exclude = vec![0x1602];
        let capabilities = PdoCapabilities::from_device(&servo);
        // The fixed PDO 0x1602 stays selected, so 0x1601 must be rejected.
        assert!(default_selection().validate(&capabilities, 0).is_err());

        let mut servo = device();
        servo.rx_pdos[1].excluded_sm = vec![3];
        let capabilities = PdoCapabilities::from_device(&servo);
        assert!(default_selection().validate(&capabilities, 0).is_err());
    }

    #[test]
    fn a_pdo_without_a_declared_sync_manager_is_not_inferred() {
        let mut servo = device();
        servo.rx_pdos[0].sm = None;
        let capabilities = PdoCapabilities::from_device(&servo);
        let error = default_selection()
            .validate(&capabilities, 0)
            .expect_err("no sm");
        assert!(error.observed.expect("observed").contains("inferred"));
    }

    #[test]
    fn pdo_assign_and_pdo_config_gate_remapping() {
        let mut servo = device();
        servo.mailbox.pdo_config = false;
        let capabilities = PdoCapabilities::from_device(&servo);
        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.entries.swap(0, 1);
        let error = selection.validate(&capabilities, 0).expect_err("no config");
        assert_eq!(error.capability.as_deref(), Some("pdo-config"));
        default_selection()
            .validate(&capabilities, 0)
            .expect("default needs no capability");

        let mut servo = device();
        servo.mailbox.pdo_assign = false;
        let capabilities = PdoCapabilities::from_device(&servo);
        let mut selection = default_selection();
        selection.pdos.retain(|pdo| pdo.index != 0x1a01);
        let error = selection.validate(&capabilities, 0).expect_err("no assign");
        assert_eq!(error.capability.as_deref(), Some("pdo-assign"));
    }

    #[test]
    fn persisted_constraint_facts_must_match_the_esi() {
        let capabilities = capabilities();
        let mut selection = default_selection();
        let pdo = selection
            .pdos
            .iter_mut()
            .find(|pdo| pdo.index == 0x1600)
            .expect("pdo");
        pdo.mandatory = true;
        let error = selection.validate(&capabilities, 0).expect_err("facts");
        assert!(
            error
                .observed
                .expect("observed")
                .contains("constraint facts")
        );
    }

    #[test]
    fn live_verification_never_turns_missing_read_back_into_a_match() {
        let selection = default_selection();
        let empty = LivePdoState::new(0);
        let verification = selection.verify(&empty);
        assert_eq!(verification.result, CheckResult::Unverified);
        assert!(!verification.is_match());
        assert!(verification.is_unverified());
        assert!(!verification.checks.is_empty());

        let mut live = LivePdoState::new(0);
        live.set_assignment(LivePdoAssignment::new(2, vec![0x1600, 0x1601, 0x1602]));
        live.set_assignment(LivePdoAssignment::new(3, vec![0x1a00]));
        live.set_mapping(LivePdoMapping::new(
            0x1600,
            vec![LiveEntry::new(0x7000, 1, 16), LiveEntry::new(0x7000, 2, 16)],
        ));
        live.set_mapping(LivePdoMapping::new(
            0x1601,
            vec![LiveEntry::new(0x7000, 3, 8)],
        ));
        live.set_mapping(LivePdoMapping::new(
            0x1602,
            vec![LiveEntry::new(0x7000, 4, 8)],
        ));
        live.set_mapping(LivePdoMapping::new(
            0x1a00,
            vec![LiveEntry::new(0x6000, 1, 16), LiveEntry::new(0x6000, 2, 16)],
        ));
        let verification = selection.verify(&live);
        assert_eq!(verification.result, CheckResult::Different);
        assert!(
            verification
                .checks
                .iter()
                .any(|check| check.subject.contains("0x1a01"))
        );

        live.set_mapping(LivePdoMapping::new(
            0x1a01,
            vec![LiveEntry::new(0x6000, 3, 1)],
        ));
        live.set_assignment(LivePdoAssignment::new(3, vec![0x1a00, 0x1a01]));
        let verification = selection.verify(&live);
        assert_eq!(verification.result, CheckResult::Match);
        assert!(verification.is_match());

        live.set_mapping(LivePdoMapping::new(
            0x1a01,
            vec![LiveEntry::new(0x6000, 3, 8)],
        ));
        assert_eq!(
            selection.verify(&live).result,
            CheckResult::Different,
            "an edited live width is different, not a match"
        );
        assert!(verification.to_json().render().contains("\"mapping\""));
        assert!(live.to_json().render().contains("\"assignment\""));
    }

    #[test]
    fn structural_fact_validation_rejects_missing_declared_pdos_and_widths() {
        let mut selection = default_selection();
        selection.pdos[0].entries[0].bit_len = 0;
        assert!(selection.validate_facts("SERVO", 0).is_err());
        let mut selection = default_selection();
        selection.pdos[0].entries[0].declared_in = 0;
        assert!(selection.validate_facts("SERVO", 0).is_err());
        let mut selection = default_selection();
        selection.pdos.push(selection.pdos[0].clone());
        assert!(selection.validate_facts("SERVO", 0).is_err());
    }

    #[test]
    fn capability_projection_includes_explicit_dc_op_modes() {
        let json = capabilities().to_json().render();
        assert!(json.contains("\"dc\": {"));
        assert!(json.contains("\"op_modes\""));
        assert!(json.contains("\"name\": \"DC Sync0\""));
        assert!(json.contains("\"assign_activate_hex\": \"#x0300\""));
        assert!(json.contains("\"sync0_factor\": 2"));

        let mut servo = device();
        servo.dc = None;
        let json = PdoCapabilities::from_device(&servo).to_json().render();
        assert!(json.contains("\"dc\": null"));
    }

    #[test]
    fn the_coe_view_is_generated_deterministically_from_the_selection() {
        let selection = default_selection();
        let view = CoeView::from_selection(&selection);
        assert_eq!(view.operations.len(), 26);
        assert_eq!(view.operations[0].object, EntryKey::new(0x1c12, 0));
        assert_eq!(view.operations[0].purpose, "assignment-clear");
        assert_eq!(view.operations[0].value, CoeValue::Unsigned(0));
        assert_eq!(view.operations[1].object, EntryKey::new(0x1c12, 1));
        assert_eq!(view.operations[1].purpose, "assignment");
        assert_eq!(view.operations[1].value, CoeValue::Unsigned(0x1600));
        assert_eq!(view.operations[4].object, EntryKey::new(0x1c12, 0));
        assert_eq!(view.operations[4].purpose, "assignment-count");
        assert_eq!(view.operations[4].value, CoeValue::Unsigned(3));

        let mapping = view
            .operations
            .iter()
            .find(|operation| {
                operation.object == EntryKey::new(0x1600, 1) && operation.purpose == "mapping"
            })
            .expect("mapping operation");
        assert_eq!(
            mapping.value,
            CoeValue::Mapping {
                index: 0x7000,
                subindex: 1,
                bit_len: 16,
            }
        );
        assert_eq!(mapping.value.packed(), Some(0x7000_0110));
        assert_eq!(CoeValue::Unsigned(3).packed(), None);

        assert!(view.operations.iter().all(|operation| {
            matches!(
                operation.object.index,
                0x1600..=0x17ff | 0x1a00..=0x1bff | 0x1c10..=0x1c2f
            )
        }));

        let mut reversed = selection.clone();
        reversed.pdos.reverse();
        assert_eq!(view, CoeView::from_selection(&reversed));
        assert!(view.to_text().contains("\"editable\": false"));
        assert!(view.to_text().contains("\"kind\": \"pdo-coe-diagnostic\""));
        assert_eq!(
            CoeView::from_selection(&PdoSelection::default())
                .operations
                .len(),
            0
        );
    }

    #[test]
    fn a_live_remap_refusal_emits_the_stable_diagnostic() {
        let diagnostic =
            remap_refused("Servo", 2, "Rx sync manager 2", "device aborted 0x06040000");
        assert_eq!(diagnostic.code, codes::PDO_REMAP_REFUSED);
        assert_eq!(diagnostic.operation, "pdo.remap");
        assert_eq!(diagnostic.device.as_deref(), Some("Servo"));
        assert_eq!(diagnostic.position, Some(2));
        assert!(
            diagnostic
                .expected
                .as_deref()
                .expect("expected")
                .contains("Rx sync manager 2")
        );
        assert_eq!(
            diagnostic.observed.as_deref(),
            Some("device aborted 0x06040000")
        );
        assert_eq!(
            diagnostic.severity(),
            super::super::diagnostics::Severity::Error
        );
    }

    #[test]
    fn one_sync_manager_cannot_carry_both_directions() {
        let mut servo = device();
        servo.tx_pdos[0].sm = Some(2);
        let capabilities = PdoCapabilities::from_device(&servo);
        let error = capabilities
            .default_selection()
            .validate(&capabilities, 0)
            .expect_err("direction conflict");
        assert_eq!(error.code, codes::PDO_SELECTION);
        assert!(
            error
                .observed
                .expect("observed")
                .contains("used by both rx and tx")
        );
        assert!(
            capabilities
                .default_selection()
                .validate_facts("SERVO", 0)
                .is_err()
        );
    }
}
