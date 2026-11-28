//! Bounded, panic-free ESI reader.
//!
//! The reader accepts the documented ESI subset ([WL-7-Plan.md] §7): only
//! `EtherCATInfo` version 1.2, no DTD/DOCTYPE, no entity declarations, no
//! external resources and no XInclude. File size, nesting depth, element and
//! attribute counts and text length are bounded before anything is retained.
//!
//! [WL-7-Plan.md]: ../../../../../WL-7-Plan.md

use std::fmt::Display;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::model::{
    DcOpMode, DeviceDescription, DistributedClock, EsiDocument, EsiVendor, MailboxCapabilities,
    ObjectEntry, Pdo, PdoDirection, PdoEntry, SyncManager, parse_flag, parse_number, parse_signed,
};
use crate::ethercat::diagnostics::{EngineeringDiagnostic, codes};

/// Maximum accepted ESI file size (128 MiB; includes the measured 70.6 MB
/// corpus file, D-261).
pub const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;
/// Maximum accepted element nesting depth.
pub const DEFAULT_MAX_DEPTH: usize = 64;
/// Maximum accepted element count per document.
pub const DEFAULT_MAX_ELEMENTS: usize = 200_000;
/// Maximum accepted attribute count per document.
pub const DEFAULT_MAX_ATTRIBUTES: usize = 400_000;
/// Maximum accepted text length between two tags.
pub const DEFAULT_MAX_TEXT: usize = 1024 * 1024;
/// Maximum number of unmodelled constructs reported per document.
const MAX_UNSUPPORTED: usize = 64;

/// Resource limits applied by the reader.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum accepted file size in bytes.
    pub max_bytes: usize,
    /// Maximum accepted nesting depth.
    pub max_depth: usize,
    /// Maximum accepted element count.
    pub max_elements: usize,
    /// Maximum accepted attribute count.
    pub max_attributes: usize,
    /// Maximum accepted text length between two tags.
    pub max_text: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_elements: DEFAULT_MAX_ELEMENTS,
            max_attributes: DEFAULT_MAX_ATTRIBUTES,
            max_text: DEFAULT_MAX_TEXT,
        }
    }
}

impl Limits {
    /// Creates the documented defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

const OPERATION: &str = "esi.import";

fn unreadable(source: &Path, observed: impl Display) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_SOURCE_UNREADABLE,
        OPERATION,
        "check the path, permissions, and that it stays inside the project root",
    )
    .with_source(source.display().to_string())
    .with_observed(observed.to_string())
}

fn version_unsupported(source: &Path, observed: impl Display) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_VERSION_UNSUPPORTED,
        OPERATION,
        "provide an ESI with root `EtherCATInfo` version 1.2",
    )
    .with_source(source.display().to_string())
    .with_expected("EtherCATInfo version 1.2")
    .with_observed(observed.to_string())
}

fn construct_unsupported(source: &Path, observed: impl Display) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_CONSTRUCT_UNSUPPORTED,
        OPERATION,
        "remove the construct or provide a description inside the accepted subset",
    )
    .with_source(source.display().to_string())
    .with_observed(observed.to_string())
}

fn malformed(source: &Path, observed: impl Display) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_MALFORMED,
        OPERATION,
        "fix the document so it is well-formed and declares the required elements",
    )
    .with_source(source.display().to_string())
    .with_observed(observed.to_string())
}

fn limit_exceeded(source: &Path, observed: impl Display, limit: usize) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_LIMIT_EXCEEDED,
        OPERATION,
        "reduce the document or raise the documented limits deliberately",
    )
    .with_source(source.display().to_string())
    .with_expected(format!("at most {limit}"))
    .with_observed(observed.to_string())
}

/// Parses an ESI file with the documented default limits.
///
/// # Errors
///
/// Returns an `STC-E-ESI-*` diagnostic for an unreadable source, an unsupported
/// document, a malformed document, or an exceeded resource limit.
pub fn parse_file(source: &Path) -> Result<EsiDocument, EngineeringDiagnostic> {
    parse_file_with(source, &Limits::default())
}

/// Parses an ESI file with explicit limits.
///
/// # Errors
///
/// Returns an `STC-E-ESI-*` diagnostic as [`parse_file`] does.
pub fn parse_file_with(
    source: &Path,
    limits: &Limits,
) -> Result<EsiDocument, EngineeringDiagnostic> {
    let metadata = std::fs::metadata(source).map_err(|error| unreadable(source, error))?;
    if !metadata.is_file() {
        return Err(unreadable(source, "not a regular file"));
    }
    if metadata.len() > limits.max_bytes as u64 {
        return Err(limit_exceeded(
            source,
            format!("{} bytes", metadata.len()),
            limits.max_bytes,
        ));
    }
    let bytes = std::fs::read(source).map_err(|error| unreadable(source, error))?;
    parse_bytes_with(source, &bytes, limits)
}

/// Parses ESI bytes with the documented default limits.
///
/// # Errors
///
/// Returns an `STC-E-ESI-*` diagnostic as [`parse_file`] does.
pub fn parse_bytes(source: &Path, bytes: &[u8]) -> Result<EsiDocument, EngineeringDiagnostic> {
    parse_bytes_with(source, bytes, &Limits::default())
}

/// Parses ESI bytes with explicit limits.
///
/// # Errors
///
/// Returns an `STC-E-ESI-*` diagnostic as [`parse_file`] does.
pub fn parse_bytes_with(
    source: &Path,
    bytes: &[u8],
    limits: &Limits,
) -> Result<EsiDocument, EngineeringDiagnostic> {
    let text = decode(source, bytes, limits)?;
    reject_constructs(source, &text)?;
    Parser::new(source, limits).run(&text)
}

/// Parses arbitrary input without panicking; used by the fuzz target.
///
/// The result is deliberately discarded: the contract under fuzzing is that
/// arbitrary bytes produce a diagnostic and never a panic, hang or unbounded
/// allocation.
pub fn parse_bytes_for_fuzz(bytes: &[u8]) {
    let _ = parse_bytes(Path::new("fuzz.esi.xml"), bytes);
}

fn decode(source: &Path, bytes: &[u8], limits: &Limits) -> Result<String, EngineeringDiagnostic> {
    if bytes.len() > limits.max_bytes {
        return Err(limit_exceeded(
            source,
            format!("{} bytes", bytes.len()),
            limits.max_bytes,
        ));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(construct_unsupported(
            source,
            "UTF-16 byte-order mark (only UTF-8 and ISO-8859-1 are accepted)",
        ));
    }
    let head_length = bytes.len().min(256);
    let head = String::from_utf8_lossy(&bytes[..head_length]).to_ascii_lowercase();
    match declared_encoding(&head).as_deref() {
        None | Some("utf-8") | Some("utf8") => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|error| {
                malformed(
                    source,
                    format!("invalid UTF-8 at byte {}", error.valid_up_to()),
                )
            }),
        Some("iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1") => Ok(decode_latin1(bytes)),
        Some(other) => Err(construct_unsupported(
            source,
            format!("declared encoding `{other}` (only UTF-8 and ISO-8859-1 are accepted)"),
        )),
    }
}

fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Returns the `encoding` pseudo-attribute of the XML declaration, lower-cased.
fn declared_encoding(head: &str) -> Option<String> {
    let start = head.find("<?xml")?;
    let end = head[start..].find("?>")? + start;
    let declaration = &head[start..end];
    let index = declaration.find("encoding")?;
    let rest = declaration[index + "encoding".len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].trim().to_ascii_lowercase())
}

fn reject_constructs(source: &Path, text: &str) -> Result<(), EngineeringDiagnostic> {
    for marker in ["<!doctype", "<!entity", "xinclude"] {
        if find_ascii_case_insensitive(text.as_bytes(), marker.as_bytes()).is_some() {
            return Err(construct_unsupported(
                source,
                format!("`{marker}` (DTD, entity and external-resource constructs are rejected)"),
            ));
        }
    }
    Ok(())
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

#[derive(Default)]
struct DeviceBuilder {
    product_code: Option<u32>,
    revision: Option<u32>,
    type_name: String,
    name: String,
    groups: Vec<String>,
    sync_managers: Vec<SyncManager>,
    rx_pdos: Vec<Pdo>,
    tx_pdos: Vec<Pdo>,
    dc: Option<DistributedClock>,
    mailbox: MailboxCapabilities,
    dictionary: Vec<ObjectEntry>,
}

struct Parser<'a> {
    source: &'a Path,
    limits: Limits,
    path: Vec<String>,
    elements: usize,
    attributes: usize,
    text: String,
    unsupported: Vec<String>,
    vendor_id: Option<u32>,
    vendor_name: Option<String>,
    devices: Vec<DeviceBuilder>,
    device: Option<DeviceBuilder>,
    pdo: Option<Pdo>,
    entry: Option<PdoEntry>,
    op_mode: Option<DcOpMode>,
    co_attributes: Option<MailboxCapabilities>,
    object: Option<ObjectEntry>,
    subitem: Option<ObjectEntry>,
    subitems: Vec<ObjectEntry>,
    saw_root: bool,
}

impl<'a> Parser<'a> {
    fn new(source: &'a Path, limits: &Limits) -> Self {
        Self {
            source,
            limits: *limits,
            path: Vec::new(),
            elements: 0,
            attributes: 0,
            text: String::new(),
            unsupported: Vec::new(),
            vendor_id: None,
            vendor_name: None,
            devices: Vec::new(),
            device: None,
            pdo: None,
            entry: None,
            op_mode: None,
            co_attributes: None,
            object: None,
            subitem: None,
            subitems: Vec::new(),
            saw_root: false,
        }
    }

    fn run(mut self, text: &str) -> Result<EsiDocument, EngineeringDiagnostic> {
        let mut reader = Reader::from_str(text);
        loop {
            match reader.read_event() {
                Ok(Event::Eof) => break,
                Ok(Event::Start(element)) => {
                    self.start(&element)?;
                }
                Ok(Event::Empty(element)) => {
                    self.start(&element)?;
                    self.end()?;
                }
                Ok(Event::End(_)) => self.end()?,
                Ok(Event::Text(value)) => self.push_text(value.as_ref())?,
                Ok(Event::CData(value)) => self.push_text(value.as_ref())?,
                Ok(Event::GeneralRef(reference)) => self.push_reference(&reference)?,
                Ok(Event::DocType(_)) => {
                    return Err(construct_unsupported(self.source, "`<!DOCTYPE>`"));
                }
                Ok(Event::Decl(_) | Event::Comment(_) | Event::PI(_)) => {}
                Err(error) => {
                    return Err(malformed(
                        self.source,
                        format!("{error} at byte {}", reader.buffer_position()),
                    ));
                }
            }
        }
        if !self.path.is_empty() {
            return Err(malformed(
                self.source,
                format!("unclosed element `{}`", self.path.join("/")),
            ));
        }
        self.finish()
    }

    fn start(&mut self, element: &BytesStart<'_>) -> Result<(), EngineeringDiagnostic> {
        self.elements += 1;
        if self.elements > self.limits.max_elements {
            return Err(limit_exceeded(
                self.source,
                format!("{} elements", self.elements),
                self.limits.max_elements,
            ));
        }
        let name = element.name().as_ref().to_owned();
        if self.path.is_empty() {
            if name != "EtherCATInfo" {
                return Err(version_unsupported(
                    self.source,
                    format!("document root `{name}`"),
                ));
            }
            self.saw_root = true;
        }
        self.path.push(name.clone());
        if self.path.len() > self.limits.max_depth {
            return Err(limit_exceeded(
                self.source,
                format!("{} levels", self.path.len()),
                self.limits.max_depth,
            ));
        }
        self.text.clear();
        let attributes = self.attributes(element)?;

        if self.at(&["EtherCATInfo"]) {
            let version = take(&attributes, "Version");
            match version.as_deref() {
                Some("1.2") => {}
                Some(other) => {
                    return Err(version_unsupported(
                        self.source,
                        format!("version `{other}`"),
                    ));
                }
                None => {
                    return Err(version_unsupported(self.source, "missing `Version`"));
                }
            }
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device"]) {
            self.device = Some(DeviceBuilder::default());
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "Type"]) {
            self.note_unknown_attributes(&attributes, &["ProductCode", "RevisionNo"]);
            self.device_mut().product_code = self.required_number(&attributes, "ProductCode")?;
            self.device_mut().revision = self.required_number(&attributes, "RevisionNo")?;
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "Sm"]) {
            self.note_unknown_attributes(&attributes, &["StartAddress", "ControlByte", "Enable"]);
            let index = u8::try_from(
                self.device
                    .as_ref()
                    .map_or(0, |device| device.sync_managers.len()),
            )
            .unwrap_or(u8::MAX);
            let sync = SyncManager {
                index,
                start_address: self
                    .optional_number(&attributes, "StartAddress")?
                    .unwrap_or(0),
                control_byte: u8::try_from(
                    self.optional_number(&attributes, "ControlByte")?
                        .unwrap_or(0),
                )
                .unwrap_or(0),
                enable: self.optional_flag(&attributes, "Enable")?.unwrap_or(false),
                name: None,
            };
            self.device_mut().sync_managers.push(sync);
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"])
            || self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "TxPdo"])
        {
            self.note_unknown_attributes(
                &attributes,
                &["Sm", "Fixed", "Mandatory", "Exclude", "ExcludedSm"],
            );
            let direction =
                if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"]) {
                    PdoDirection::Rx
                } else {
                    PdoDirection::Tx
                };
            let mut pdo = Pdo {
                direction,
                index: 0,
                sm: self
                    .optional_number(&attributes, "Sm")?
                    .and_then(|value| u8::try_from(value).ok()),
                fixed: self.optional_flag(&attributes, "Fixed")?.unwrap_or(false),
                mandatory: self
                    .optional_flag(&attributes, "Mandatory")?
                    .unwrap_or(false),
                excluded_sm: Vec::new(),
                exclude: Vec::new(),
                name: None,
                entries: Vec::new(),
            };
            for value in list_values(take(&attributes, "Exclude").as_deref()) {
                let index = parse_number(value).ok_or_else(|| {
                    construct_unsupported(self.source, format!("PDO `Exclude` value `{value}`"))
                })?;
                pdo.exclude.push(u16::try_from(index).map_err(|_| {
                    construct_unsupported(self.source, format!("PDO `Exclude` value `{value}`"))
                })?);
            }
            for value in list_values(take(&attributes, "ExcludedSm").as_deref()) {
                let index = parse_number(value).ok_or_else(|| {
                    construct_unsupported(self.source, format!("PDO `ExcludedSm` value `{value}`"))
                })?;
                pdo.excluded_sm.push(u8::try_from(index).map_err(|_| {
                    construct_unsupported(self.source, format!("PDO `ExcludedSm` value `{value}`"))
                })?);
            }
            self.pdo = Some(pdo);
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "RxPdo",
            "Entry",
        ]) || self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "TxPdo",
            "Entry",
        ]) {
            self.note_unknown_attributes(&attributes, &[]);
            self.entry = Some(PdoEntry {
                index: 0,
                subindex: 0,
                bit_len: 0,
                name: None,
                data_type: None,
            });
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Dc",
            "OpMode",
        ]) {
            let index = self
                .device
                .as_ref()
                .and_then(|device| device.dc.as_ref())
                .map_or(0, |dc| dc.op_modes.len());
            self.op_mode = Some(DcOpMode {
                index,
                name: String::new(),
                description: None,
                assign_activate: 0,
                sync0_cycle: 0,
                sync0_factor: None,
                sync0_shift: 0,
                sync1_cycle: None,
                sync1_shift: None,
            });
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Dc",
            "OpMode",
            "CycleTimeSync0",
        ]) && name == "CycleTimeSync0"
        {
            self.note_unknown_attributes(&attributes, &["Factor"]);
            if let Some(factor) = self.optional_number(&attributes, "Factor")?
                && let Some(mode) = self.op_mode.as_mut()
            {
                mode.sync0_factor = Some(factor);
            }
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Mailbox",
            "CoE",
        ]) {
            self.note_unknown_attributes(
                &attributes,
                &["SdoInfo", "PdoAssign", "PdoConfig", "CompleteAccess"],
            );
            let mailbox = MailboxCapabilities {
                sdo_info: self.optional_flag(&attributes, "SdoInfo")?.unwrap_or(false),
                pdo_assign: self
                    .optional_flag(&attributes, "PdoAssign")?
                    .unwrap_or(false),
                pdo_config: self
                    .optional_flag(&attributes, "PdoConfig")?
                    .unwrap_or(false),
                complete_access: self
                    .optional_flag(&attributes, "CompleteAccess")?
                    .unwrap_or(false),
            };
            self.co_attributes = Some(mailbox);
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
        ]) {
            self.object = Some(ObjectEntry {
                index: 0,
                subindex: 0,
                name: None,
                data_type: None,
                access: None,
            });
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
            "SubItem",
        ]) {
            self.subitem = Some(ObjectEntry {
                index: 0,
                subindex: 0,
                name: None,
                data_type: None,
                access: None,
            });
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), EngineeringDiagnostic> {
        let name = self
            .path
            .pop()
            .ok_or_else(|| malformed(self.source, "unexpected closing tag"))?;
        let text = self.text.trim().to_owned();
        self.text.clear();

        if self.at(&["EtherCATInfo", "Vendor"]) && name == "Id" {
            self.vendor_id = Some(parse_number(&text).ok_or_else(|| {
                malformed(self.source, format!("vendor id `{text}` is not a number"))
            })?);
        } else if self.at(&["EtherCATInfo", "Vendor"]) && name == "Name" {
            self.vendor_name = Some(text);
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices"]) && name == "Device" {
            let device = self
                .device
                .take()
                .ok_or_else(|| malformed(self.source, "device without a start tag"))?;
            self.devices.push(device);
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device"]) && name == "Type"
        {
            self.device_mut().type_name = text;
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device"]) && name == "Name"
        {
            self.device_mut().name = text;
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "Group"])
            && name == "Type"
        {
            self.device_mut().groups.push(text);
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device"]) && name == "Sm" {
            if !text.is_empty()
                && let Some(sync) = self.device_mut().sync_managers.last_mut()
            {
                sync.name = Some(text);
            }
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device"])
            && (name == "RxPdo" || name == "TxPdo")
        {
            let mut pdo = self
                .pdo
                .take()
                .ok_or_else(|| malformed(self.source, "PDO without a start tag"))?;
            if pdo.index == 0 {
                return Err(malformed(self.source, "PDO without an `<Index>`"));
            }
            match pdo.direction {
                PdoDirection::Rx => self.device_mut().rx_pdos.push(pdo),
                PdoDirection::Tx => {
                    pdo.direction = PdoDirection::Tx;
                    self.device_mut().tx_pdos.push(pdo);
                }
            }
        } else if (self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"])
            || self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "TxPdo"]))
            && name == "Index"
        {
            let index = self.required_u16_text(&text, "PDO index")?;
            if let Some(pdo) = self.pdo.as_mut() {
                pdo.index = index;
            }
        } else if (self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"])
            || self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "TxPdo"]))
            && name == "Name"
        {
            if let Some(pdo) = self.pdo.as_mut() {
                pdo.name = Some(text);
            }
        } else if (self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"])
            || self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "TxPdo"]))
            && name == "Exclude"
        {
            let index = self.required_u16_text(&text, "PDO `Exclude`")?;
            if let Some(pdo) = self.pdo.as_mut() {
                pdo.exclude.push(index);
            }
        } else if (self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "RxPdo"])
            || self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "TxPdo"]))
            && name == "Entry"
        {
            let entry = self
                .entry
                .take()
                .ok_or_else(|| malformed(self.source, "entry without a start tag"))?;
            if entry.bit_len == 0 {
                return Err(malformed(
                    self.source,
                    format!(
                        "PDO entry `0x{:04x}:{}` has no `<BitLen>`",
                        entry.index, entry.subindex
                    ),
                ));
            }
            if let Some(pdo) = self.pdo.as_mut() {
                pdo.entries.push(entry);
            }
        } else if (self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "RxPdo",
            "Entry",
        ]) || self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "TxPdo",
            "Entry",
        ])) && matches!(
            name.as_str(),
            "Index" | "SubIndex" | "BitLen" | "Name" | "DataType"
        ) {
            let index = if name == "Index" {
                Some(self.required_u16_text(&text, "entry index")?)
            } else {
                None
            };
            let subindex = if name == "SubIndex" {
                Some(
                    u8::try_from(self.required_number_text(&text, "subindex")?)
                        .map_err(|_| malformed(self.source, "entry subindex out of range"))?,
                )
            } else {
                None
            };
            let bit_len = if name == "BitLen" {
                Some(
                    u8::try_from(self.required_number_text(&text, "bit length")?)
                        .map_err(|_| malformed(self.source, "entry bit length out of range"))?,
                )
            } else {
                None
            };
            let entry = self
                .entry
                .as_mut()
                .ok_or_else(|| malformed(self.source, "entry field outside an entry"))?;
            if let Some(value) = index {
                entry.index = value;
            }
            if let Some(value) = subindex {
                entry.subindex = value;
            }
            if let Some(value) = bit_len {
                entry.bit_len = value;
            }
            if name == "Name" {
                entry.name = Some(text);
            } else if name == "DataType" {
                entry.data_type = Some(text);
            }
        } else if self.at(&["EtherCATInfo", "Descriptions", "Devices", "Device", "Dc"])
            && name == "OpMode"
        {
            let mut mode = self
                .op_mode
                .take()
                .ok_or_else(|| malformed(self.source, "DC op mode without a start tag"))?;
            if mode.name.is_empty() {
                mode.name = format!("op-mode-{}", mode.index);
            }
            self.device_mut()
                .dc
                .get_or_insert_with(|| DistributedClock {
                    op_modes: Vec::new(),
                })
                .op_modes
                .push(mode);
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Dc",
            "OpMode",
        ]) && matches!(
            name.as_str(),
            "Name"
                | "Desc"
                | "AssignActivate"
                | "CycleTimeSync0"
                | "ShiftTimeSync0"
                | "CycleTimeSync1"
                | "ShiftTimeSync1"
        ) {
            let assign_activate = if name == "AssignActivate" {
                Some(self.required_number_text(&text, "AssignActivate")?)
            } else {
                None
            };
            let sync0_cycle = if name == "CycleTimeSync0" {
                Some(self.required_number_text(&text, "CycleTimeSync0")?)
            } else {
                None
            };
            let sync0_shift = if name == "ShiftTimeSync0" {
                Some(parse_signed(&text).ok_or_else(|| {
                    malformed(
                        self.source,
                        format!("ShiftTimeSync0 `{text}` is not a number"),
                    )
                })?)
            } else {
                None
            };
            let sync1_cycle = if name == "CycleTimeSync1" {
                Some(self.required_number_text(&text, "CycleTimeSync1")?)
            } else {
                None
            };
            let sync1_shift = if name == "ShiftTimeSync1" {
                Some(parse_signed(&text).ok_or_else(|| {
                    malformed(
                        self.source,
                        format!("ShiftTimeSync1 `{text}` is not a number"),
                    )
                })?)
            } else {
                None
            };
            let mode = self
                .op_mode
                .as_mut()
                .ok_or_else(|| malformed(self.source, "DC op-mode field outside an op mode"))?;
            match name.as_str() {
                "Name" => mode.name = text,
                "Desc" => mode.description = Some(text),
                _ => {}
            }
            if let Some(value) = assign_activate {
                mode.assign_activate = value;
            }
            if let Some(value) = sync0_cycle {
                mode.sync0_cycle = value;
            }
            if let Some(value) = sync0_shift {
                mode.sync0_shift = value;
            }
            if let Some(value) = sync1_cycle {
                mode.sync1_cycle = Some(value);
            }
            if let Some(value) = sync1_shift {
                mode.sync1_shift = Some(value);
            }
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Mailbox",
        ]) && name == "CoE"
        {
            if let Some(mailbox) = self.co_attributes.take() {
                self.device_mut().mailbox = mailbox;
            }
        } else if (self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
        ]) || self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
        ])) && name == "Object"
        {
            // A `<SubItem>` declares only its subindex; the object index comes
            // from the enclosing `<Object>`. Without inheriting it, every
            // subindex would be addressed as object 0x0000 (WL-7 phase 4).
            let object_index = self.object.as_ref().map_or(0, |object| object.index);
            if let Some(object) = self.object.take() {
                self.device_mut().dictionary.push(object);
            }
            for mut subitem in std::mem::take(&mut self.subitems) {
                subitem.index = object_index;
                self.device_mut().dictionary.push(subitem);
            }
        } else if self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
        ]) && name == "SubItem"
        {
            if let Some(subitem) = self.subitem.take() {
                self.subitems.push(subitem);
            }
        } else if (self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
        ]) || self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
            "SubItem",
        ])) && matches!(name.as_str(), "Index" | "SubIndex" | "Name" | "Type")
        {
            let is_subitem = self.at(&[
                "EtherCATInfo",
                "Descriptions",
                "Devices",
                "Device",
                "Profile",
                "Dictionary",
                "Objects",
                "Object",
                "SubItem",
            ]);
            let index = if name == "Index" {
                Some(self.required_u16_text(&text, "object index")?)
            } else {
                None
            };
            let subindex = if name == "SubIndex" {
                Some(
                    u8::try_from(self.required_number_text(&text, "object subindex")?)
                        .map_err(|_| malformed(self.source, "object subindex out of range"))?,
                )
            } else {
                None
            };
            let target = if is_subitem {
                self.subitem.as_mut()
            } else {
                self.object.as_mut()
            };
            if let Some(object) = target {
                if let Some(value) = index {
                    object.index = value;
                }
                if let Some(value) = subindex {
                    object.subindex = value;
                }
                if name == "Name" {
                    object.name = Some(text);
                } else if name == "Type" {
                    object.data_type = Some(text);
                }
            }
        } else if (self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
            "Info",
        ]) || self.at(&[
            "EtherCATInfo",
            "Descriptions",
            "Devices",
            "Device",
            "Profile",
            "Dictionary",
            "Objects",
            "Object",
            "SubItem",
            "Info",
        ])) && name == "Access"
        {
            let target = if self.at(&[
                "EtherCATInfo",
                "Descriptions",
                "Devices",
                "Device",
                "Profile",
                "Dictionary",
                "Objects",
                "Object",
                "SubItem",
                "Info",
            ]) {
                self.subitem.as_mut()
            } else {
                self.object.as_mut()
            };
            if let Some(object) = target {
                object.access = Some(text);
            }
        }
        Ok(())
    }

    fn push_text(&mut self, text: &str) -> Result<(), EngineeringDiagnostic> {
        self.text.push_str(text);
        if self.text.len() > self.limits.max_text {
            return Err(limit_exceeded(
                self.source,
                format!("{} characters of text", self.text.len()),
                self.limits.max_text,
            ));
        }
        Ok(())
    }

    fn push_reference(
        &mut self,
        reference: &quick_xml::events::BytesRef<'_>,
    ) -> Result<(), EngineeringDiagnostic> {
        if let Some(character) = reference
            .resolve_char_ref()
            .map_err(|error| malformed(self.source, format!("character reference: {error}")))?
        {
            return self.push_text(&character.to_string());
        }
        let name = reference.as_ref();
        let resolved = quick_xml::escape::resolve_predefined_entity(name)
            .ok_or_else(|| construct_unsupported(self.source, format!("entity `&{name};`")))?;
        self.push_text(resolved)
    }

    fn finish(self) -> Result<EsiDocument, EngineeringDiagnostic> {
        if !self.saw_root {
            return Err(malformed(self.source, "empty document"));
        }
        let vendor_id = self
            .vendor_id
            .ok_or_else(|| malformed(self.source, "missing `<Vendor><Id>`"))?;
        let vendor_name = self
            .vendor_name
            .ok_or_else(|| malformed(self.source, "missing `<Vendor><Name>`"))?;
        let mut devices = Vec::with_capacity(self.devices.len());
        for (index, builder) in self.devices.into_iter().enumerate() {
            let product_code = builder.product_code.ok_or_else(|| {
                malformed(
                    self.source,
                    format!("device {index} has no `<Type ProductCode=\"...\">`"),
                )
            })?;
            let revision = builder.revision.ok_or_else(|| {
                malformed(
                    self.source,
                    format!("device {index} has no `<Type RevisionNo=\"...\">`"),
                )
            })?;
            let name = if builder.name.is_empty() {
                builder.type_name.clone()
            } else {
                builder.name
            };
            let mut device = DeviceDescription {
                vendor_id,
                product_code,
                revision,
                name,
                type_name: builder.type_name,
                groups: builder.groups,
                sync_managers: builder.sync_managers,
                rx_pdos: builder.rx_pdos,
                tx_pdos: builder.tx_pdos,
                dc: builder.dc,
                mailbox: builder.mailbox,
                dictionary: builder.dictionary,
                digest: String::new(),
            };
            device.digest = device.compute_digest();
            devices.push(device);
        }
        Ok(EsiDocument {
            source: self.source.to_path_buf(),
            vendor: EsiVendor {
                id: vendor_id,
                name: vendor_name,
            },
            devices,
            unsupported: self.unsupported,
        })
    }

    fn device_mut(&mut self) -> &mut DeviceBuilder {
        self.device.get_or_insert_with(DeviceBuilder::default)
    }

    fn at(&self, expected: &[&str]) -> bool {
        self.path.len() == expected.len()
            && self
                .path
                .iter()
                .map(String::as_str)
                .eq(expected.iter().copied())
    }

    fn note_unsupported(&mut self, text: String) {
        if self.unsupported.len() < MAX_UNSUPPORTED {
            self.unsupported.push(text);
        }
    }

    fn note_unknown_attributes(&mut self, attributes: &[(String, String)], known: &[&str]) {
        for (key, _) in attributes {
            if !known.iter().any(|known| key.eq_ignore_ascii_case(known)) {
                let text = format!(
                    "attribute `{key}` on `{}` is not modelled",
                    self.path.join("/")
                );
                self.note_unsupported(text);
            }
        }
    }

    fn attributes(
        &mut self,
        element: &BytesStart<'_>,
    ) -> Result<Vec<(String, String)>, EngineeringDiagnostic> {
        let mut attributes = Vec::new();
        for attribute in element.attributes() {
            self.attributes += 1;
            if self.attributes > self.limits.max_attributes {
                return Err(limit_exceeded(
                    self.source,
                    format!("{} attributes", self.attributes),
                    self.limits.max_attributes,
                ));
            }
            let attribute = attribute.map_err(|error| {
                malformed(
                    self.source,
                    format!("attribute is not well-formed: {error}"),
                )
            })?;
            let key = attribute.key.as_ref().to_owned();
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map_err(|error| {
                    malformed(self.source, format!("attribute `{key}` value: {error}"))
                })?
                .into_owned();
            attributes.push((key, value));
        }
        Ok(attributes)
    }

    fn required_number(
        &self,
        attributes: &[(String, String)],
        name: &str,
    ) -> Result<Option<u32>, EngineeringDiagnostic> {
        match take(attributes, name) {
            None => Ok(None),
            Some(raw) => parse_number(&raw).map(Some).ok_or_else(|| {
                malformed(
                    self.source,
                    format!("`{name}` value `{raw}` is not a number"),
                )
            }),
        }
    }

    fn optional_number(
        &self,
        attributes: &[(String, String)],
        name: &str,
    ) -> Result<Option<u32>, EngineeringDiagnostic> {
        self.required_number(attributes, name)
    }

    fn optional_flag(
        &self,
        attributes: &[(String, String)],
        name: &str,
    ) -> Result<Option<bool>, EngineeringDiagnostic> {
        match take(attributes, name) {
            None => Ok(None),
            Some(raw) => parse_flag(&raw).map(Some).ok_or_else(|| {
                construct_unsupported(
                    self.source,
                    format!("`{name}` value `{raw}` (expected 0 or 1)"),
                )
            }),
        }
    }

    fn required_number_text(&self, text: &str, what: &str) -> Result<u32, EngineeringDiagnostic> {
        parse_number(text)
            .ok_or_else(|| malformed(self.source, format!("{what} `{text}` is not a number")))
    }

    fn required_u16_text(&self, text: &str, what: &str) -> Result<u16, EngineeringDiagnostic> {
        u16::try_from(self.required_number_text(text, what)?)
            .map_err(|_| malformed(self.source, format!("{what} `{text}` is out of range")))
    }
}

fn take(attributes: &[(String, String)], name: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_owned())
}

fn list_values(value: Option<&str>) -> Vec<&str> {
    value
        .map(|value| {
            value
                .split([' ', ',', ';'])
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<EtherCATInfo Version="1.2">
  <Vendor><Id>#x00000002</Id><Name>Beckhoff Automation GmbH</Name></Vendor>
  <Descriptions>
    <Devices>
      <Device>
        <Type ProductCode="#x00000002" RevisionNo="#x00000001">EL1008</Type>
        <Name>EL1008 8Ch. Dig. Input</Name>
        <Sm StartAddress="1000" ControlByte="#x26" Enable="1">MBoxOut</Sm>
        <Sm StartAddress="1100" ControlByte="#x20" Enable="1">Inputs</Sm>
        <TxPdo Sm="3" Fixed="1" Mandatory="1">
          <Index>#x1A00</Index>
          <Name>Channel 1</Name>
          <Exclude>#x1A01</Exclude>
          <Entry>
            <Index>#x6000</Index>
            <SubIndex>1</SubIndex>
            <BitLen>1</BitLen>
            <Name>Input</Name>
            <DataType>BOOL</DataType>
          </Entry>
        </TxPdo>
        <Dc>
          <OpMode>
            <Name>DC</Name>
            <Desc>Synchron</Desc>
            <AssignActivate>#x0300</AssignActivate>
            <CycleTimeSync0 Factor="1">1000000</CycleTimeSync0>
            <ShiftTimeSync0>0</ShiftTimeSync0>
            <CycleTimeSync1>2000000</CycleTimeSync1>
            <ShiftTimeSync1>-5</ShiftTimeSync1>
          </OpMode>
        </Dc>
        <Mailbox><CoE SdoInfo="1" PdoAssign="1" PdoConfig="1" CompleteAccess="1" /></Mailbox>
        <Profile>
          <Dictionary>
            <Objects>
              <Object>
                <Index>#x1018</Index>
                <Name>Identity</Name>
                <Type>IDENTITY</Type>
                <Info><Access>ro</Access></Info>
                <SubItem>
                  <SubIndex>1</SubIndex>
                  <Name>Vendor ID</Name>
                  <Info><Access>ro</Access></Info>
                </SubItem>
              </Object>
            </Objects>
          </Dictionary>
        </Profile>
      </Device>
    </Devices>
  </Descriptions>
</EtherCATInfo>
"##;

    fn parse(text: &str) -> Result<EsiDocument, EngineeringDiagnostic> {
        parse_bytes(Path::new("test.esi.xml"), text.as_bytes())
    }

    #[test]
    fn reads_the_supported_subset() {
        let document = parse(MINIMAL).expect("valid");
        assert_eq!(document.vendor.id, 2);
        assert_eq!(document.vendor.name, "Beckhoff Automation GmbH");
        assert_eq!(document.devices.len(), 1);
        let device = &document.devices[0];
        assert_eq!(device.identity(), (2, 2, 1));
        assert_eq!(device.type_name, "EL1008");
        assert_eq!(device.name, "EL1008 8Ch. Dig. Input");
        assert_eq!(device.sync_managers.len(), 2);
        assert_eq!(device.sync_managers[1].name.as_deref(), Some("Inputs"));
        assert_eq!(device.tx_pdos.len(), 1);
        let pdo = &device.tx_pdos[0];
        assert_eq!(pdo.index, 0x1a00);
        assert_eq!(pdo.sm, Some(3));
        assert!(pdo.fixed && pdo.mandatory);
        assert_eq!(pdo.exclude, vec![0x1a01]);
        assert_eq!(pdo.entries[0].index, 0x6000);
        assert_eq!(pdo.entries[0].bit_len, 1);
        let dc = device.dc.as_ref().expect("dc");
        assert_eq!(dc.op_modes[0].name, "DC");
        assert_eq!(dc.op_modes[0].assign_activate, 0x0300);
        assert_eq!(dc.op_modes[0].sync0_cycle, 1_000_000);
        assert_eq!(dc.op_modes[0].sync0_factor, Some(1));
        assert_eq!(dc.op_modes[0].sync1_cycle, Some(2_000_000));
        assert_eq!(dc.op_modes[0].sync1_shift, Some(-5));
        assert!(device.mailbox.sdo_info && device.mailbox.pdo_config);
        assert_eq!(device.dictionary.len(), 2);
        assert_eq!(device.dictionary[0].index, 0x1018);
        assert!(document.unsupported.is_empty());
        assert_eq!(device.digest.len(), 64);
    }

    #[test]
    fn rejects_unsupported_roots_and_versions() {
        for text in [
            r#"<EtherCATInfo Version="1.1"><Vendor><Id>1</Id><Name>V</Name></Vendor></EtherCATInfo>"#,
            r#"<EtherCATInfo><Vendor><Id>1</Id><Name>V</Name></Vendor></EtherCATInfo>"#,
            r#"<Other Version="1.2"/>"#,
        ] {
            let error = parse(text).expect_err("unsupported");
            assert_eq!(error.code, codes::ESI_VERSION_UNSUPPORTED, "{text}");
        }
    }

    #[test]
    fn rejects_dtd_entities_and_external_includes() {
        for text in [
            r#"<?xml version="1.0"?><!DOCTYPE foo SYSTEM "x.dtd"><EtherCATInfo Version="1.2"/>"#,
            r#"<EtherCATInfo Version="1.2"><!ENTITY x "y"/></EtherCATInfo>"#,
            r#"<EtherCATInfo Version="1.2" xmlns:xi="http://www.w3.org/2001/XInclude"/>"#,
            "<EtherCATInfo Version=\"1.2\">\u{FEFF}xinclude</EtherCATInfo>",
        ] {
            let error = parse(text).expect_err("unsupported construct");
            assert_eq!(error.code, codes::ESI_CONSTRUCT_UNSUPPORTED, "{text}");
        }
    }

    #[test]
    fn resolves_predefined_and_character_references() {
        let text = MINIMAL
            .replace("Beckhoff Automation GmbH", "A &amp; B &#x43;")
            .replace(
                "<Name>EL1008 8Ch. Dig. Input</Name>",
                "<Name>A &lt; B</Name>",
            );
        let document = parse(&text).expect("valid");
        assert_eq!(document.vendor.name, "A & B C");
        assert_eq!(document.devices[0].name, "A < B");
        let unknown = MINIMAL.replace("Beckhoff Automation GmbH", "A &nbsp; B");
        assert_eq!(
            parse(&unknown).expect_err("unknown entity").code,
            codes::ESI_CONSTRUCT_UNSUPPORTED
        );
    }

    #[test]
    fn rejects_malformed_documents() {
        for text in [
            "<EtherCATInfo Version=\"1.2\"><Vendor>",
            "",
            "<EtherCATInfo Version=\"1.2\"></EtherCATInfo>",
            "<EtherCATInfo Version=\"1.2\"><Vendor><Id>x</Id><Name>V</Name></Vendor></EtherCATInfo>",
            r#"<EtherCATInfo Version="1.2"><Vendor><Id>1</Id></Vendor></EtherCATInfo>"#,
            r##"<EtherCATInfo Version="1.2"><Vendor><Id>1</Id><Name>V</Name></Vendor><Descriptions><Devices><Device><Type RevisionNo="#x1">T</Type></Device></Devices></Descriptions></EtherCATInfo>"##,
            r##"<EtherCATInfo Version="1.2"><Vendor><Id>1</Id><Name>V</Name></Vendor><Descriptions><Devices><Device><Type ProductCode="#x1">T</Type></Device></Devices></Descriptions></EtherCATInfo>"##,
        ] {
            let error = parse(text).expect_err("malformed");
            assert_eq!(error.code, codes::ESI_MALFORMED, "{text:?}");
        }
    }

    #[test]
    fn enforces_resource_limits() {
        let limits = Limits {
            max_bytes: 64,
            ..Limits::default()
        };
        assert_eq!(
            parse_bytes_with(Path::new("big"), MINIMAL.repeat(4).as_bytes(), &limits)
                .expect_err("bytes")
                .code,
            codes::ESI_LIMIT_EXCEEDED
        );

        let limits = Limits {
            max_elements: 3,
            ..Limits::default()
        };
        assert_eq!(
            parse_bytes_with(Path::new("elements"), MINIMAL.as_bytes(), &limits)
                .expect_err("elements")
                .code,
            codes::ESI_LIMIT_EXCEEDED
        );

        let limits = Limits {
            max_depth: 3,
            ..Limits::default()
        };
        assert_eq!(
            parse_bytes_with(Path::new("depth"), MINIMAL.as_bytes(), &limits)
                .expect_err("depth")
                .code,
            codes::ESI_LIMIT_EXCEEDED
        );

        let limits = Limits {
            max_attributes: 2,
            ..Limits::default()
        };
        assert_eq!(
            parse_bytes_with(Path::new("attributes"), MINIMAL.as_bytes(), &limits)
                .expect_err("attributes")
                .code,
            codes::ESI_LIMIT_EXCEEDED
        );

        let limits = Limits {
            max_text: 8,
            ..Limits::default()
        };
        assert_eq!(
            parse_bytes_with(Path::new("text"), MINIMAL.as_bytes(), &limits)
                .expect_err("text")
                .code,
            codes::ESI_LIMIT_EXCEEDED
        );
    }

    #[test]
    fn decodes_iso_8859_1_and_rejects_other_encodings() {
        let text = MINIMAL
            .replace("Beckhoff Automation GmbH", "B\u{fc}cher")
            .replace("encoding=\"UTF-8\"", "encoding=\"ISO-8859-1\"");
        let mut latin1 = Vec::new();
        for character in text.chars() {
            let code = character as u32;
            if code <= 0xFF {
                latin1.push(u8::try_from(code).expect("latin-1"));
            } else {
                latin1.extend_from_slice(character.to_string().as_bytes());
            }
        }
        let document = parse_bytes(Path::new("latin1"), &latin1).expect("latin-1");
        assert_eq!(document.vendor.name, "B\u{fc}cher");

        let utf16 = [0xFF_u8, 0xFE, b'<', 0, b'a', 0];
        assert_eq!(
            parse_bytes(Path::new("utf16"), &utf16)
                .expect_err("utf16")
                .code,
            codes::ESI_CONSTRUCT_UNSUPPORTED
        );
        let other = MINIMAL.replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\"");
        assert_eq!(
            parse(&other).expect_err("encoding").code,
            codes::ESI_CONSTRUCT_UNSUPPORTED
        );
    }

    #[test]
    fn records_unmodelled_attributes_instead_of_silently_ignoring_them() {
        let text = MINIMAL.replace(
            "<TxPdo Sm=\"3\" Fixed=\"1\" Mandatory=\"1\">",
            "<TxPdo Sm=\"3\" Fixed=\"1\" Mandatory=\"1\" VendorFlag=\"7\">",
        );
        let document = parse(&text).expect("valid");
        assert_eq!(document.unsupported.len(), 1);
        assert!(document.unsupported[0].contains("VendorFlag"));
    }

    #[test]
    fn reports_non_numeric_and_invalid_flags() {
        let bad_index = MINIMAL.replace("<Index>#x1A00</Index>", "<Index>nope</Index>");
        assert_eq!(
            parse(&bad_index).expect_err("index").code,
            codes::ESI_MALFORMED
        );
        let bad_flag = MINIMAL.replace("Mandatory=\"1\"", "Mandatory=\"yes\"");
        assert_eq!(
            parse(&bad_flag).expect_err("flag").code,
            codes::ESI_CONSTRUCT_UNSUPPORTED
        );
        let bad_len = MINIMAL.replace("<BitLen>1</BitLen>", "<BitLen>0</BitLen>");
        assert_eq!(
            parse(&bad_len).expect_err("bit length").code,
            codes::ESI_MALFORMED
        );
        let missing_index = MINIMAL.replace("<Index>#x1A00</Index>", "");
        assert_eq!(
            parse(&missing_index).expect_err("pdo index").code,
            codes::ESI_MALFORMED
        );
        let bad_product = MINIMAL.replace("ProductCode=\"#x00000002\"", "ProductCode=\"two\"");
        assert_eq!(
            parse(&bad_product).expect_err("product").code,
            codes::ESI_MALFORMED
        );
        let bad_activate = MINIMAL.replace(
            "<AssignActivate>#x0300</AssignActivate>",
            "<AssignActivate>x</AssignActivate>",
        );
        assert_eq!(
            parse(&bad_activate).expect_err("activate").code,
            codes::ESI_MALFORMED
        );
        let bad_shift = MINIMAL.replace(
            "<ShiftTimeSync0>0</ShiftTimeSync0>",
            "<ShiftTimeSync0>x</ShiftTimeSync0>",
        );
        assert_eq!(
            parse(&bad_shift).expect_err("shift").code,
            codes::ESI_MALFORMED
        );
        let bad_shift1 = MINIMAL.replace(
            "<ShiftTimeSync1>-5</ShiftTimeSync1>",
            "<ShiftTimeSync1>x</ShiftTimeSync1>",
        );
        assert_eq!(
            parse(&bad_shift1).expect_err("shift1").code,
            codes::ESI_MALFORMED
        );
    }

    #[test]
    fn decode_helpers_handle_declarations_and_case_insensitive_search() {
        assert_eq!(
            declared_encoding("<?xml version=\"1.0\" encoding='ISO-8859-1'?>"),
            Some("iso-8859-1".to_owned())
        );
        assert_eq!(declared_encoding("<?xml version=\"1.0\"?>"), None);
        assert_eq!(
            declared_encoding("<?xml version=\"1.0\" encoding=UTF-8?>"),
            None
        );
        assert_eq!(declared_encoding("no declaration"), None);
        assert_eq!(
            find_ascii_case_insensitive(b"aXIncludeB", b"xinclude"),
            Some(1)
        );
        assert_eq!(find_ascii_case_insensitive(b"abc", b"abcd"), None);
        assert_eq!(find_ascii_case_insensitive(b"abc", b""), None);
        assert_eq!(decode_latin1(&[0x41, 0xFC]), "A\u{fc}");
        assert_eq!(list_values(Some("1, 2;3")), vec!["1", "2", "3"]);
        assert!(list_values(None).is_empty());
    }
}
