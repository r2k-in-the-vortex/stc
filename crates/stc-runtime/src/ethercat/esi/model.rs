//! Typed ESI capability model.
//!
//! These types describe what a device *supports*. They are deliberately not a
//! desired configuration: an imported description never selects PDOs, a DC
//! operating mode or a PLC channel (D-263, D-269, D-275).

use std::fmt::Write as _;
use std::path::PathBuf;

use stc_got::sha256::sha256;

use stc_got::json::Json;

/// Direction of a process-data object as declared by the ESI.
///
/// `RxPdo` describes data the master writes to the slave (PLC outputs);
/// `TxPdo` describes data the slave returns (PLC inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PdoDirection {
    /// Master-to-slave process data (`RxPdo`).
    Rx,
    /// Slave-to-master process data (`TxPdo`).
    Tx,
}

impl PdoDirection {
    /// Short stable spelling used in structured output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rx => "rx",
            Self::Tx => "tx",
        }
    }

    /// PLC-facing channel direction implied by the ESI direction.
    #[must_use]
    pub fn channel_direction(self) -> &'static str {
        match self {
            Self::Rx => "output",
            Self::Tx => "input",
        }
    }
}

/// Vendor identity and display name from the ESI document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsiVendor {
    /// Numeric vendor id.
    pub id: u32,
    /// Vendor display name.
    pub name: String,
}

/// One parsed ESI document (one file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsiDocument {
    /// Source file the document was read from.
    pub source: PathBuf,
    /// Declared vendor.
    pub vendor: EsiVendor,
    /// Device descriptions declared by the document, in declaration order.
    pub devices: Vec<DeviceDescription>,
    /// Constructs the accepted subset does not model, reported for review.
    pub unsupported: Vec<String>,
}

impl EsiDocument {
    /// Returns the device description at `index`.
    #[must_use]
    pub fn device(&self, index: usize) -> Option<&DeviceDescription> {
        self.devices.get(index)
    }
}

/// Capabilities and identity facts of one device description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDescription {
    /// Numeric vendor id (from the document's `<Vendor><Id>`).
    pub vendor_id: u32,
    /// Product code from `<Type ProductCode="...">`.
    pub product_code: u32,
    /// Revision number from `<Type RevisionNo="...">`.
    pub revision: u32,
    /// Device display name.
    pub name: String,
    /// Device type name (the `<Type>` text).
    pub type_name: String,
    /// Declared groups, in declaration order.
    pub groups: Vec<String>,
    /// Declared sync managers, in declaration order.
    pub sync_managers: Vec<SyncManager>,
    /// Declared `RxPdo` objects (master to slave).
    pub rx_pdos: Vec<Pdo>,
    /// Declared `TxPdo` objects (slave to master).
    pub tx_pdos: Vec<Pdo>,
    /// Distributed-clock alternatives, when the ESI declares a `<Dc>` section.
    pub dc: Option<DistributedClock>,
    /// Mailbox/CoE capability flags.
    pub mailbox: MailboxCapabilities,
    /// Object-dictionary metadata the ESI declares; may be incomplete.
    pub dictionary: Vec<ObjectEntry>,
    /// SHA-256 digest of the canonical accepted description.
    pub digest: String,
}

impl DeviceDescription {
    /// Returns the exact identity tuple `(vendor, product, revision)`.
    #[must_use]
    pub fn identity(&self) -> (u32, u32, u32) {
        (self.vendor_id, self.product_code, self.revision)
    }

    /// Iterates the declared PDOs with their direction, in declaration order.
    pub fn pdos(&self) -> impl Iterator<Item = (PdoDirection, &Pdo)> {
        self.rx_pdos
            .iter()
            .map(|pdo| (PdoDirection::Rx, pdo))
            .chain(self.tx_pdos.iter().map(|pdo| (PdoDirection::Tx, pdo)))
    }

    /// Canonical, deterministic rendering of the accepted description.
    ///
    /// The digest covers identity, names, sync managers, PDOs, entries, DC
    /// alternatives, CoE flags and the declared dictionary. It is used to tell
    /// two descriptions of one identity apart (D-263, D-267).
    #[must_use]
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        out.push_str("stc-esi-device/v1\n");
        let _ = writeln!(
            out,
            "vendor={:08x}\nproduct={:08x}\nrevision={:08x}",
            self.vendor_id, self.product_code, self.revision
        );
        let _ = writeln!(out, "type={}", self.type_name);
        let _ = writeln!(out, "name={}", self.name);
        for group in &self.groups {
            let _ = writeln!(out, "group={group}");
        }
        for sync in &self.sync_managers {
            let _ = writeln!(
                out,
                "sm={};start={};control={};enable={};name={}",
                sync.index,
                sync.start_address,
                sync.control_byte,
                sync.enable,
                sync.name.as_deref().unwrap_or("")
            );
        }
        for (direction, pdo) in self.pdos() {
            let _ = writeln!(
                out,
                "pdo={};index={:04x};sm={};fixed={};mandatory={};exclude={:?};excluded_sm={:?};name={}",
                direction.as_str(),
                pdo.index,
                pdo.sm.map_or_else(|| "-".to_owned(), |sm| sm.to_string()),
                pdo.fixed,
                pdo.mandatory,
                pdo.exclude,
                pdo.excluded_sm,
                pdo.name.as_deref().unwrap_or("")
            );
            for entry in &pdo.entries {
                let _ = writeln!(
                    out,
                    "entry={:04x}:{};bits={};name={};type={}",
                    entry.index,
                    entry.subindex,
                    entry.bit_len,
                    entry.name.as_deref().unwrap_or(""),
                    entry.data_type.as_deref().unwrap_or("")
                );
            }
        }
        match &self.dc {
            None => out.push_str("dc=none\n"),
            Some(dc) => {
                out.push_str("dc=declared\n");
                for mode in &dc.op_modes {
                    let _ = writeln!(
                        out,
                        "opmode={};name={};activate={:#06x};sync0={};factor={:?};shift0={};sync1={:?};shift1={:?}",
                        mode.index,
                        mode.name,
                        mode.assign_activate,
                        mode.sync0_cycle,
                        mode.sync0_factor,
                        mode.sync0_shift,
                        mode.sync1_cycle,
                        mode.sync1_shift
                    );
                }
            }
        }
        let _ = writeln!(
            out,
            "coe=sdo_info:{};pdo_assign:{};pdo_config:{};complete_access:{}",
            self.mailbox.sdo_info,
            self.mailbox.pdo_assign,
            self.mailbox.pdo_config,
            self.mailbox.complete_access
        );
        for object in &self.dictionary {
            let _ = writeln!(
                out,
                "object={:04x}:{};name={};type={};access={}",
                object.index,
                object.subindex,
                object.name.as_deref().unwrap_or(""),
                object.data_type.as_deref().unwrap_or(""),
                object.access.as_deref().unwrap_or("")
            );
        }
        out
    }

    /// Digest of [`Self::canonical`] as lower-case hex.
    #[must_use]
    pub fn compute_digest(&self) -> String {
        let digest = sha256(self.canonical().as_bytes());
        let mut text = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(text, "{byte:02x}");
        }
        text
    }
}

/// One declared sync manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncManager {
    /// Sync-manager index (`Sm` attribute of the PDO, or the `<Sm>` index).
    pub index: u8,
    /// Physical start address declared by the ESI.
    pub start_address: u32,
    /// Control byte declared by the ESI.
    pub control_byte: u8,
    /// Whether the sync manager is enabled.
    pub enable: bool,
    /// Optional sync-manager name.
    pub name: Option<String>,
}

/// One declared process-data object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pdo {
    /// Direction implied by the declaring element.
    pub direction: PdoDirection,
    /// PDO index (`0x1600`-style).
    pub index: u16,
    /// Sync manager the ESI assigns this PDO to, when declared.
    pub sm: Option<u8>,
    /// `Fixed="1"` prohibits reassignment, omission, reorder and move.
    pub fixed: bool,
    /// `Mandatory="1"` requires the PDO to remain assigned.
    pub mandatory: bool,
    /// Sync managers this PDO excludes when it is assigned.
    pub excluded_sm: Vec<u8>,
    /// PDO indices this PDO excludes when it is assigned.
    pub exclude: Vec<u16>,
    /// Optional PDO name.
    pub name: Option<String>,
    /// Declared entries, in declaration order.
    pub entries: Vec<PdoEntry>,
}

/// One declared PDO entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoEntry {
    /// Object index (`0x6000`-style).
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
    /// Declared bit length.
    pub bit_len: u8,
    /// Optional entry name.
    pub name: Option<String>,
    /// Optional ESI data-type reference.
    pub data_type: Option<String>,
}

/// Declared distributed-clock alternatives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributedClock {
    /// Declared `<OpMode>` entries, in declaration order.
    pub op_modes: Vec<DcOpMode>,
}

/// One declared distributed-clock operating mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DcOpMode {
    /// Declaration index, which disambiguates duplicate names.
    pub index: usize,
    /// Declared name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// `AssignActivate` word; never synthesised or edited.
    pub assign_activate: u32,
    /// `CycleTimeSync0` value, or `0` when the ESI declares none.
    pub sync0_cycle: u32,
    /// `CycleTimeSync0` factor, when declared.
    pub sync0_factor: Option<u32>,
    /// `ShiftTimeSync0` value in nanoseconds.
    pub sync0_shift: i32,
    /// `CycleTimeSync1` value, when declared.
    pub sync1_cycle: Option<u32>,
    /// `ShiftTimeSync1` value in nanoseconds, when declared.
    pub sync1_shift: Option<i32>,
}

/// Mailbox and CoE capability flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailboxCapabilities {
    /// Device offers SDO Information services.
    pub sdo_info: bool,
    /// Device supports PDO assignment objects.
    pub pdo_assign: bool,
    /// Device supports PDO mapping configuration.
    pub pdo_config: bool,
    /// Device supports complete-access transfers.
    pub complete_access: bool,
}

impl MailboxCapabilities {
    /// Structured projection.
    #[must_use]
    pub fn to_json(self) -> Json {
        Json::object([
            ("sdo_info".into(), Json::Bool(self.sdo_info)),
            ("pdo_assign".into(), Json::Bool(self.pdo_assign)),
            ("pdo_config".into(), Json::Bool(self.pdo_config)),
            ("complete_access".into(), Json::Bool(self.complete_access)),
        ])
    }
}

/// Object-dictionary metadata declared by the ESI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectEntry {
    /// Object index.
    pub index: u16,
    /// Object or sub-item subindex.
    pub subindex: u8,
    /// Optional declared name.
    pub name: Option<String>,
    /// Optional declared data type.
    pub data_type: Option<String>,
    /// Optional declared access rights.
    pub access: Option<String>,
}

/// Parses an ESI numeric value (`#x00000002`, `0x02`, or decimal).
#[must_use]
pub fn parse_number(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (digits, radix) =
        if let Some(rest) = text.strip_prefix("#x").or_else(|| text.strip_prefix("#X")) {
            (rest, 16)
        } else if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (rest, 16)
        } else {
            (text, 10)
        };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

/// Parses a signed ESI value used by shift-time fields.
#[must_use]
pub fn parse_signed(text: &str) -> Option<i32> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('-') {
        parse_number(rest)
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| -value)
    } else {
        parse_number(text).and_then(|value| i32::try_from(value).ok())
    }
}

/// Parses an ESI boolean flag (`1`/`0`, `true`/`false`).
#[must_use]
pub fn parse_flag(text: &str) -> Option<bool> {
    match text.trim() {
        "1" | "true" | "TRUE" | "True" => Some(true),
        "0" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

/// Structured projection of a device description's capabilities.
#[must_use]
pub fn device_capabilities_json(device: &DeviceDescription) -> Json {
    Json::object([
        (
            "vendor_id".into(),
            Json::unsigned(u64::from(device.vendor_id)),
        ),
        (
            "product_code".into(),
            Json::unsigned(u64::from(device.product_code)),
        ),
        (
            "product_code_hex".into(),
            Json::string(format!("#x{:08x}", device.product_code)),
        ),
        (
            "revision".into(),
            Json::unsigned(u64::from(device.revision)),
        ),
        (
            "revision_hex".into(),
            Json::string(format!("#x{:08x}", device.revision)),
        ),
        ("name".into(), Json::string(device.name.as_str())),
        ("type".into(), Json::string(device.type_name.as_str())),
        ("digest".into(), Json::string(device.digest.as_str())),
        (
            "groups".into(),
            Json::Array(
                device
                    .groups
                    .iter()
                    .map(|group| Json::string(group.as_str()))
                    .collect(),
            ),
        ),
        (
            "sync_managers".into(),
            Json::Array(
                device
                    .sync_managers
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
        ("rx_pdos".into(), pdo_json(&device.rx_pdos)),
        ("tx_pdos".into(), pdo_json(&device.tx_pdos)),
        (
            "dc".into(),
            match &device.dc {
                None => Json::Null,
                Some(dc) => Json::object([(
                    "op_modes".into(),
                    Json::Array(
                        dc.op_modes
                            .iter()
                            .map(|mode| {
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
                                        Json::optional(
                                            mode.sync1_shift
                                                .map(|value| Json::signed(i64::from(value))),
                                        ),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                )]),
            },
        ),
        ("mailbox".into(), device.mailbox.to_json()),
        (
            "dictionary_count".into(),
            Json::unsigned(device.dictionary.len() as u64),
        ),
    ])
}

fn pdo_json(pdos: &[Pdo]) -> Json {
    Json::Array(
        pdos.iter()
            .map(|pdo| {
                Json::object([
                    ("direction".into(), Json::string(pdo.direction.as_str())),
                    (
                        "channel_direction".into(),
                        Json::string(pdo.direction.channel_direction()),
                    ),
                    ("index".into(), Json::string(format!("0x{:04x}", pdo.index))),
                    (
                        "sm".into(),
                        Json::optional(pdo.sm.map(|sm| Json::unsigned(u64::from(sm)))),
                    ),
                    ("fixed".into(), Json::Bool(pdo.fixed)),
                    ("mandatory".into(), Json::Bool(pdo.mandatory)),
                    (
                        "exclude".into(),
                        Json::Array(
                            pdo.exclude
                                .iter()
                                .map(|index| Json::string(format!("0x{index:04x}")))
                                .collect(),
                        ),
                    ),
                    (
                        "excluded_sm".into(),
                        Json::Array(
                            pdo.excluded_sm
                                .iter()
                                .map(|sm| Json::unsigned(u64::from(*sm)))
                                .collect(),
                        ),
                    ),
                    (
                        "name".into(),
                        Json::optional(pdo.name.as_ref().map(|name| Json::string(name.as_str()))),
                    ),
                    (
                        "entries".into(),
                        Json::Array(
                            pdo.entries
                                .iter()
                                .map(|entry| {
                                    Json::object([
                                        (
                                            "index".into(),
                                            Json::string(format!("0x{:04x}", entry.index)),
                                        ),
                                        (
                                            "subindex".into(),
                                            Json::unsigned(u64::from(entry.subindex)),
                                        ),
                                        (
                                            "bit_len".into(),
                                            Json::unsigned(u64::from(entry.bit_len)),
                                        ),
                                        (
                                            "name".into(),
                                            Json::optional(
                                                entry
                                                    .name
                                                    .as_ref()
                                                    .map(|name| Json::string(name.as_str())),
                                            ),
                                        ),
                                        (
                                            "data_type".into(),
                                            Json::optional(
                                                entry.data_type.as_ref().map(|data_type| {
                                                    Json::string(data_type.as_str())
                                                }),
                                            ),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceDescription {
        let mut device = DeviceDescription {
            vendor_id: 2,
            product_code: 0x0000_0002,
            revision: 1,
            name: "EL1008".into(),
            type_name: "EL1008".into(),
            groups: vec!["Inputs".into()],
            sync_managers: vec![SyncManager {
                index: 3,
                start_address: 0x1100,
                control_byte: 0x20,
                enable: true,
                name: Some("Inputs".into()),
            }],
            rx_pdos: Vec::new(),
            tx_pdos: vec![Pdo {
                direction: PdoDirection::Tx,
                index: 0x1a00,
                sm: Some(3),
                fixed: true,
                mandatory: true,
                excluded_sm: Vec::new(),
                exclude: Vec::new(),
                name: Some("Channel 1".into()),
                entries: vec![PdoEntry {
                    index: 0x6000,
                    subindex: 1,
                    bit_len: 1,
                    name: Some("Input".into()),
                    data_type: Some("BOOL".into()),
                }],
            }],
            dc: None,
            mailbox: MailboxCapabilities::default(),
            dictionary: Vec::new(),
            digest: String::new(),
        };
        device.digest = device.compute_digest();
        device
    }

    #[test]
    fn numbers_accept_esi_hex_and_decimal() {
        assert_eq!(parse_number("#x0000_0002"), None);
        assert_eq!(parse_number("#x00000002"), Some(2));
        assert_eq!(parse_number("#X0000000A"), Some(10));
        assert_eq!(parse_number("0x10"), Some(16));
        assert_eq!(parse_number(" 42 "), Some(42));
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("#x"), None);
        assert_eq!(parse_number("nope"), None);
        assert_eq!(parse_signed("-10"), Some(-10));
        assert_eq!(parse_signed("10"), Some(10));
        assert_eq!(parse_signed("-nope"), None);
        assert_eq!(parse_flag("1"), Some(true));
        assert_eq!(parse_flag("false"), Some(false));
        assert_eq!(parse_flag("maybe"), None);
    }

    #[test]
    fn digest_is_stable_and_covers_content() {
        let first = device();
        let second = device();
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.digest.len(), 64);

        let mut changed = device();
        changed.revision = 2;
        assert_ne!(first.digest, changed.compute_digest());

        let mut changed = device();
        changed.tx_pdos[0].entries[0].bit_len = 2;
        assert_ne!(first.digest, changed.compute_digest());

        let mut changed = device();
        changed.mailbox.sdo_info = true;
        assert_ne!(first.digest, changed.compute_digest());

        let mut changed = device();
        changed.dc = Some(DistributedClock {
            op_modes: vec![DcOpMode {
                index: 0,
                name: "DC".into(),
                description: None,
                assign_activate: 0x0300,
                sync0_cycle: 1000,
                sync0_factor: Some(1),
                sync0_shift: 0,
                sync1_cycle: None,
                sync1_shift: None,
            }],
        });
        assert_ne!(first.digest, changed.compute_digest());
    }

    #[test]
    fn canonical_rendering_covers_groups_pdos_and_dictionary() {
        let mut value = device();
        value.dictionary.push(ObjectEntry {
            index: 0x1018,
            subindex: 0,
            name: Some("Identity".into()),
            data_type: Some("IDENTITY".into()),
            access: Some("ro".into()),
        });
        value.dc = Some(DistributedClock {
            op_modes: vec![DcOpMode {
                index: 1,
                name: "DC".into(),
                description: Some("declared".into()),
                assign_activate: 0x0300,
                sync0_cycle: 500_000,
                sync0_factor: None,
                sync0_shift: -10,
                sync1_cycle: Some(1_000_000),
                sync1_shift: Some(5),
            }],
        });
        let canonical = value.canonical();
        assert!(canonical.contains("group=Inputs"));
        assert!(canonical.contains("pdo=tx;index=1a00;sm=3;fixed=true"));
        assert!(canonical.contains("entry=6000:1;bits=1"));
        assert!(canonical.contains("dc=declared"));
        assert!(canonical.contains("opmode=1;name=DC"));
        assert!(canonical.contains("shift0=-10"));
        assert!(canonical.contains("object=1018:0"));
        assert!(canonical.contains("coe=sdo_info:false"));
        assert_eq!(value.pdos().count(), 1);
    }

    #[test]
    fn capabilities_json_exposes_structured_capabilities_only() {
        let mut value = device();
        value.dc = Some(DistributedClock {
            op_modes: vec![DcOpMode {
                index: 0,
                name: "FreeRunCompatible".into(),
                description: None,
                assign_activate: 0x0300,
                sync0_cycle: 1000,
                sync0_factor: Some(2),
                sync0_shift: 0,
                sync1_cycle: None,
                sync1_shift: None,
            }],
        });
        let json = device_capabilities_json(&value).render();
        assert!(json.contains("\"product_code_hex\": \"#x00000002\""));
        assert!(json.contains("\"direction\": \"tx\""));
        assert!(json.contains("\"channel_direction\": \"input\""));
        assert!(json.contains("\"assign_activate_hex\": \"#x0300\""));
        assert!(json.contains("\"sync0_factor\": 2"));
        assert!(json.contains("\"dictionary_count\": 0"));
        assert!(!json.contains("STC-"));
        assert_eq!(value.identity(), (2, 2, 1));
        assert_eq!(PdoDirection::Rx.channel_direction(), "output");
        assert!(value.mailbox.to_json().render().contains("sdo_info"));
    }

    #[test]
    fn documents_and_elements_keep_declaration_order() {
        let document = EsiDocument {
            source: PathBuf::from("esi/a.xml"),
            vendor: EsiVendor {
                id: 2,
                name: "Vendor".into(),
            },
            devices: vec![device()],
            unsupported: vec!["attribute `Foo`".into()],
        };
        assert_eq!(document.device(0).map(|value| value.revision), Some(1));
        assert!(document.device(1).is_none());
        assert_eq!(document.devices[0].name, "EL1008");
        assert_eq!(document.unsupported.len(), 1);
    }
}
