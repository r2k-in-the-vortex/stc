//! `stc ethercat`: EtherCAT engineering commands (WL-7 phases 1-4).
//!
//! The handler only wires the typed engineering API to the terminal. All
//! engineering behavior, validation and diagnostics live in `stc-runtime`, so a
//! later TUI/GUI calls the same operations without duplicating logic (D-259,
//! D-270).
//!
//! Exit codes: `0` success, `1` the requested engineering operation was
//! rejected or a verification reported a non-clean result, `2` a usage error.
//! A verification result is never terminal text in the API: the CLI prints the
//! structured values it receives.

// The structured diagnostic payload (D-280) is the API contract: code,
// severity, device/position, expected/observed, capability and remedy. It is
// only constructed when an operation is rejected, never on a cycle path, so
// the large `Err` variant is intentional (see `stc_runtime::ethercat`).
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use stc_got::json::Json;
use stc_runtime::ethercat::api::{
    DescriptionSummary, EngineeringService, SdoReadResult, SdoWriteResult, SourceInspection,
};
use stc_runtime::ethercat::config::{ConfigurationReport, DcChoice, EngineeringConfiguration};
use stc_runtime::ethercat::diagnostics::{EngineeringDiagnostic, codes};
use stc_runtime::ethercat::mock::MockEngineeringHost;
use stc_runtime::ethercat::pdo::LivePdoState;
use stc_runtime::ethercat::sdo::{self, ObjectAddress, ObjectDictionary, SdoData, SdoType};
use stc_runtime::ethercat::setup::{DEFAULT_SESSION_FILE, SetupSession, SetupStatus};
use stc_runtime::ethercat::topology::{
    CaptureOptions, FieldCheck, ObservedTopology, SlaveInspection, Topology, TopologyComparison,
};

use crate::cli::{EthercatAction, Invocation};

/// Runs one `stc ethercat` action and returns the process exit code.
pub(crate) fn run(invocation: &Invocation) -> u8 {
    let Some(action) = invocation.ethercat.action.as_ref() else {
        eprintln!("stc: `ethercat` needs an action");
        return 2;
    };
    let mut service = match &invocation.ethercat.esi_dir {
        Some(directory) => match EngineeringService::open(Path::new(directory)) {
            Ok(service) => service,
            Err(diagnostic) => return reject(&diagnostic),
        },
        None => EngineeringService::standalone(),
    };
    // The ABI 2 shim binding is Phase 7 work (D-279). Until then the CLI host
    // reports no capabilities, so a hardware operation fails closed with
    // `STC-E-IO-ETH-043` instead of claiming an empty or configured bus.
    service.set_host(MockEngineeringHost::default());
    match action {
        EthercatAction::List | EthercatAction::Search { .. } => {
            let descriptions = match action {
                EthercatAction::Search { query } => service.search(query),
                _ => service.list(),
            };
            match descriptions {
                Ok(descriptions) => {
                    emit_descriptions(invocation, &service, &descriptions);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::InspectSource { source } => {
            match service.inspect_source(Path::new(source)) {
                Ok(inspection) => {
                    emit_inspection(invocation, &inspection);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::InspectIdentity => {
            let (Some(vendor), Some(product)) =
                (invocation.ethercat.vendor, invocation.ethercat.product)
            else {
                eprintln!(
                    "stc: `stc ethercat esi inspect` needs `--vendor <id>` and `--product <id>`"
                );
                return 2;
            };
            match service.inspect_identity(vendor, product, invocation.ethercat.revision) {
                Ok(inspection) => {
                    emit_inspection(invocation, &inspection);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::TopologyShow => {
            let topology = match load_topology(invocation) {
                Ok(topology) => topology,
                Err(diagnostic) => return reject(&diagnostic),
            };
            match topology.validate() {
                Ok(()) => {
                    emit_topology(invocation, &topology, None);
                    0
                }
                Err(diagnostic) => {
                    emit_topology(invocation, &topology, Some(&diagnostic));
                    reject(&diagnostic)
                }
            }
        }
        EthercatAction::TopologyScan => match service.scan(session(invocation).as_ref()) {
            Ok(observed) => {
                emit_observed(invocation, &observed);
                0
            }
            Err(diagnostic) => reject(&diagnostic),
        },
        EthercatAction::TopologyDiff => {
            let topology = match load_topology(invocation) {
                Ok(topology) => topology,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let observed = match load_observed(invocation, &service) {
                Ok(observed) => observed,
                Err(diagnostic) => return reject(&diagnostic),
            };
            match service.compare(&topology, &observed) {
                Ok(comparison) => {
                    emit_comparison(invocation, &comparison);
                    if comparison.is_match() { 0 } else { 1 }
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::TopologyCapture { output } => {
            let base = match &invocation.ethercat.topology {
                Some(_) => match load_topology(invocation) {
                    Ok(topology) => Some(topology),
                    Err(diagnostic) => return reject(&diagnostic),
                },
                None => None,
            };
            let observed = match load_observed(invocation, &service) {
                Ok(observed) => observed,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let options = CaptureOptions {
                pin_serial: invocation.ethercat.pin_serial,
                pin_alias: invocation.ethercat.pin_alias,
            };
            match service.capture(base.as_ref(), &observed, options) {
                Ok(topology) => {
                    if let Err(diagnostic) = write_topology(Path::new(output), &topology) {
                        return reject(&diagnostic);
                    }
                    emit_capture(invocation, output, &topology);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SlaveInspect { position } => {
            let topology = match &invocation.ethercat.topology {
                Some(_) => match load_topology(invocation) {
                    Ok(topology) => Some(topology),
                    Err(diagnostic) => return reject(&diagnostic),
                },
                None => None,
            };
            let observed = match load_observed(invocation, &service) {
                Ok(observed) => observed,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let inspection = stc_runtime::ethercat::topology::inspect_position(
                *position,
                &observed,
                topology.as_ref(),
            );
            emit_slave(invocation, &inspection);
            if inspection.result.is_clean() { 0 } else { 1 }
        }
        EthercatAction::SlavePdos { position } => {
            match service.slave_pdos(*position, session(invocation).as_ref()) {
                Ok(live) => {
                    emit_live_pdos(invocation, &live);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SdoList { position } => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let (device, identity) = match device_identity(&configuration, *position, "sdo.list") {
                Ok(device) => device,
                Err(diagnostic) => return reject(&diagnostic),
            };
            match service.object_dictionary(*position, Some(identity), session(invocation).as_ref())
            {
                Ok(dictionary) => {
                    emit_dictionary(invocation, &device, &dictionary);
                    emit_warnings(&dictionary.warnings);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SdoInspect {
            position,
            index,
            subindex,
        } => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let (device, identity) = match device_identity(&configuration, *position, "sdo.inspect")
            {
                Ok(device) => device,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let dictionary = match service.object_dictionary(
                *position,
                Some(identity),
                session(invocation).as_ref(),
            ) {
                Ok(dictionary) => dictionary,
                Err(diagnostic) => return reject(&diagnostic),
            };
            emit_warnings(&dictionary.warnings);
            match dictionary.object(*index, *subindex) {
                Some(object) => {
                    if invocation.format.is_json() {
                        print!(
                            "{}",
                            Json::object([
                                ("position".into(), Json::unsigned(u64::from(*position))),
                                ("device".into(), Json::string(device.as_str())),
                                ("object".into(), object.to_json()),
                            ])
                            .render()
                        );
                    } else {
                        println!("position: {position} ({device})");
                        println!(
                            "{} `{}` type={} access={} source={} online-only={}",
                            object.describe(),
                            object.name.as_deref().unwrap_or("-"),
                            object.data_type.as_deref().unwrap_or("-"),
                            object.access.as_deref().unwrap_or("-"),
                            object.source.as_str(),
                            object.online_only
                        );
                        if let Some(description) = &object.description {
                            println!("  description: {description}");
                        }
                        for conflict in &object.conflicts {
                            println!(
                                "  conflict {}: esi `{}` online `{}`",
                                conflict.field, conflict.esi, conflict.online
                            );
                        }
                    }
                    0
                }
                None => reject(&sdo::object_unavailable(
                    "sdo.inspect",
                    &device,
                    *position,
                    *index,
                    *subindex,
                )),
            }
        }
        EthercatAction::SdoRead {
            position,
            index,
            subindex,
        } => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let (device, identity) = match device_identity(&configuration, *position, "sdo.read") {
                Ok(device) => device,
                Err(diagnostic) => return reject(&diagnostic),
            };
            match service.sdo_read(
                &device,
                ObjectAddress::new(*position, *index, *subindex),
                Some(identity),
                session(invocation).as_ref(),
            ) {
                Ok(result) => {
                    emit_sdo_read(invocation, &result);
                    emit_warnings(&result.warnings);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SdoWrite {
            position,
            index,
            subindex,
            value,
        } => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let (device, identity) = match device_identity(&configuration, *position, "sdo.write") {
                Ok(device) => device,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let dictionary = match service.object_dictionary(
                *position,
                Some(identity),
                session(invocation).as_ref(),
            ) {
                Ok(dictionary) => dictionary,
                Err(diagnostic) => return reject(&diagnostic),
            };
            emit_warnings(&dictionary.warnings);
            let object = match dictionary.object(*index, *subindex) {
                Some(object) => object,
                None => {
                    return reject(&sdo::object_unavailable(
                        "sdo.write",
                        &device,
                        *position,
                        *index,
                        *subindex,
                    ));
                }
            };
            let ty = SdoType::from_declaration(object.data_type.as_deref());
            let parsed = match SdoData::parse(value, ty) {
                Ok(parsed) => parsed,
                Err(message) => {
                    return reject(&sdo::value_error(
                        "sdo.write",
                        &device,
                        *position,
                        *index,
                        *subindex,
                        "a value matching the declared type",
                        &message,
                    ));
                }
            };
            match service.sdo_write(
                &device,
                ObjectAddress::new(*position, *index, *subindex),
                &parsed,
                Some(identity),
                session(invocation).as_ref(),
            ) {
                Ok(result) => {
                    emit_sdo_write(invocation, &result);
                    emit_warnings(&result.warnings);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::ConfigShow => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            emit_config(invocation, &configuration);
            0
        }
        EthercatAction::ConfigValidate => {
            let configuration = match load_config(invocation) {
                Ok(configuration) => configuration,
                Err(diagnostic) => return reject(&diagnostic),
            };
            let report = service.validate_configuration(
                &configuration,
                &bound_channels(invocation),
                invocation.ethercat.cycle_ns.unwrap_or(1_000_000),
            );
            emit_config_report(invocation, &report);
            if report.is_valid() { 0 } else { 1 }
        }
        EthercatAction::SetupEnter => {
            let master = invocation
                .ethercat
                .master
                .unwrap_or_else(|| service.master());
            let path = session_path(invocation);
            match service.setup_enter(&path, master, now_unix()) {
                Ok(status) => {
                    emit_setup(invocation, &status);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SetupLeave => {
            let path = session_path(invocation);
            match service.setup_leave(&path) {
                Ok(status) => {
                    emit_setup(invocation, &status);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
        EthercatAction::SetupStatus => {
            let path = session_path(invocation);
            match service.setup_status(&path) {
                Ok(status) => {
                    emit_setup(invocation, &status);
                    0
                }
                Err(diagnostic) => reject(&diagnostic),
            }
        }
    }
}

fn reject(diagnostic: &EngineeringDiagnostic) -> u8 {
    eprintln!("error[{}]: {}", diagnostic.code, diagnostic.message());
    1
}

fn emit_warnings(warnings: &[EngineeringDiagnostic]) {
    for warning in warnings {
        eprintln!("warning[{}]: {}", warning.code, warning.message());
    }
}

fn device_identity(
    configuration: &EngineeringConfiguration,
    position: u16,
    operation: &'static str,
) -> Result<(String, (u32, u32, u32)), EngineeringDiagnostic> {
    configuration
        .devices
        .iter()
        .find(|device| device.position == position)
        .map(|device| {
            (
                device.name.clone(),
                (
                    device.identity.vendor_id,
                    device.identity.product_code,
                    device.identity.revision,
                ),
            )
        })
        .ok_or_else(|| {
            EngineeringDiagnostic::new(
                codes::MODE_INVALID,
                operation,
                "pass a configuration that defines the device at this position",
            )
            .with_position(position)
            .with_expected("a configured device at this position")
            .with_observed("no device in the engineering configuration")
        })
}

fn emit_dictionary(invocation: &Invocation, device: &str, dictionary: &ObjectDictionary) {
    if invocation.format.is_json() {
        let json = Json::object([
            ("device".into(), Json::string(device)),
            ("dictionary".into(), dictionary.to_json()),
        ]);
        print!("{}", json.render());
        return;
    }
    println!(
        "position: {} ({device}), {} object entr{}",
        dictionary.position,
        dictionary.objects.len(),
        if dictionary.objects.len() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    for object in &dictionary.objects {
        println!(
            "{} `{}` type={} access={} source={} online-only={}",
            object.describe(),
            object.name.as_deref().unwrap_or("-"),
            object.data_type.as_deref().unwrap_or("-"),
            object.access.as_deref().unwrap_or("-"),
            object.source.as_str(),
            object.online_only
        );
        if let Some(description) = &object.description {
            println!("  description: {description}");
        }
        for conflict in &object.conflicts {
            println!(
                "  conflict {}: esi `{}` online `{}`",
                conflict.field, conflict.esi, conflict.online
            );
        }
    }
    if dictionary.is_empty() {
        println!("no object metadata available");
    }
}

fn emit_sdo_read(invocation: &Invocation, result: &SdoReadResult) {
    if invocation.format.is_json() {
        print!("{}", result.to_json().render());
        return;
    }
    println!("position: {} ({})", result.position, result.device);
    println!(
        "object: 0x{:04x}:{} `{}` type={} source={}",
        result.index,
        result.subindex,
        result.object_name.as_deref().unwrap_or("-"),
        result.declared_type.as_deref().unwrap_or("-"),
        result.source.as_str()
    );
    println!("value: {}", sdo_value_text(&result.value));
    println!("raw: {}", hex_bytes(&result.raw));
    println!(
        "al_state: {}",
        result
            .al_state
            .map_or_else(|| "-".to_owned(), |state| state.to_string())
    );
}

fn emit_sdo_write(invocation: &Invocation, result: &SdoWriteResult) {
    if invocation.format.is_json() {
        print!("{}", result.to_json().render());
        return;
    }
    println!("position: {} ({})", result.position, result.device);
    println!(
        "object: 0x{:04x}:{} `{}`",
        result.index,
        result.subindex,
        result.object_name.as_deref().unwrap_or("-")
    );
    println!("wrote: {}", sdo_value_text(&result.value));
    println!("raw: {}", hex_bytes(&result.raw));
    match result.verified {
        Some(true) => println!("read-back: match"),
        Some(false) => println!("read-back: different"),
        None => println!("read-back: unverified (unavailable, never reported as a match)"),
    }
    println!("persisted: false (a commissioning write is not a startup setting)");
}

fn sdo_value_text(value: &SdoData) -> String {
    match value {
        SdoData::Bool(value) => value.to_string(),
        SdoData::Unsigned(value) => value.to_string(),
        SdoData::Signed(value) => value.to_string(),
        SdoData::Float(value) => value.to_string(),
        SdoData::Text(value) => format!("\"{value}\""),
        SdoData::Bytes(bytes) => hex_bytes(bytes),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn session_path(invocation: &Invocation) -> PathBuf {
    PathBuf::from(
        invocation
            .ethercat
            .session
            .as_deref()
            .unwrap_or(DEFAULT_SESSION_FILE),
    )
}

/// The recorded setup session, when one exists and parses.
fn session(invocation: &Invocation) -> Option<SetupSession> {
    SetupSession::read_lossy(&session_path(invocation))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn load_config(invocation: &Invocation) -> Result<EngineeringConfiguration, EngineeringDiagnostic> {
    let path = invocation
        .ethercat
        .config
        .as_deref()
        .expect("validated by the parser");
    let text = std::fs::read_to_string(path).map_err(|error| {
        EngineeringDiagnostic::new(
            codes::MODE_INVALID,
            "config.parse",
            "pass a readable engineering configuration draft",
        )
        .with_source(path)
        .with_observed(format!("cannot read the configuration: {error}"))
    })?;
    EngineeringConfiguration::parse(&text)
}

fn bound_channels(invocation: &Invocation) -> BTreeMap<String, Vec<String>> {
    let mut bound: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &invocation.ethercat.bound_channels {
        if let Some((device, channel)) = entry.split_once(':') {
            bound
                .entry(device.to_owned())
                .or_default()
                .push(channel.to_owned());
        }
    }
    bound
}

fn load_topology(invocation: &Invocation) -> Result<Topology, EngineeringDiagnostic> {
    let path = invocation
        .ethercat
        .topology
        .as_deref()
        .expect("validated by the parser");
    let text = std::fs::read_to_string(path).map_err(|error| {
        EngineeringDiagnostic::new(
            codes::TOPOLOGY_POSITION,
            "topology.show",
            "pass a readable topology draft, or capture one with `topology capture`",
        )
        .with_source(path)
        .with_observed(format!("cannot read the draft: {error}"))
    })?;
    Topology::parse(&text)
}

fn write_topology(path: &Path, topology: &Topology) -> Result<(), EngineeringDiagnostic> {
    std::fs::write(path, topology.to_text()).map_err(|error| {
        EngineeringDiagnostic::new(
            codes::TOPOLOGY_POSITION,
            "topology.capture",
            "pass a writable output path for the draft",
        )
        .with_source(path.display().to_string())
        .with_observed(format!("cannot write the draft: {error}"))
    })
}

/// Reads a recorded observation, or scans the bus when none was supplied.
fn load_observed(
    invocation: &Invocation,
    service: &EngineeringService,
) -> Result<ObservedTopology, EngineeringDiagnostic> {
    match &invocation.ethercat.observed {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|error| {
                EngineeringDiagnostic::new(
                    codes::TOPOLOGY_POSITION,
                    "topology.observe",
                    "pass a readable observation file, or scan the bus without `--observed`",
                )
                .with_source(path.as_str())
                .with_observed(format!("cannot read the observation: {error}"))
            })?;
            ObservedTopology::parse(&text)
        }
        None => service.scan(session(invocation).as_ref()),
    }
}

fn emit_descriptions(
    invocation: &Invocation,
    service: &EngineeringService,
    descriptions: &[DescriptionSummary],
) {
    if invocation.format.is_json() {
        let json = Json::object([
            (
                "descriptions".into(),
                Json::Array(
                    descriptions
                        .iter()
                        .map(DescriptionSummary::to_json)
                        .collect(),
                ),
            ),
            (
                "problems".into(),
                Json::Array(
                    service
                        .problems()
                        .iter()
                        .map(EngineeringDiagnostic::to_json)
                        .collect(),
                ),
            ),
        ]);
        print!("{}", json.render());
    } else {
        for problem in service.problems() {
            eprintln!("warning: {}", problem.message());
        }
        if descriptions.is_empty() {
            println!("no ESI device descriptions found");
            return;
        }
        for description in descriptions {
            println!(
                "{}#{}: {} ({}) vendor=#x{:08x} product=#x{:08x} revision=#x{:08x}",
                description.source.display(),
                description.device_index,
                description.type_name,
                description.name,
                description.vendor_id,
                description.product_code,
                description.revision
            );
        }
    }
}

fn emit_inspection(invocation: &Invocation, inspection: &SourceInspection) {
    if invocation.format.is_json() {
        print!("{}", inspection.to_json().render());
        return;
    }
    println!("source: {}", inspection.source.display());
    println!(
        "vendor: #x{:08x} ({})",
        inspection.vendor_id, inspection.vendor_name
    );
    if !inspection.pinned && !inspection.devices.is_empty() {
        println!("revision: unpinned (highest declared revision selected)");
    }
    for device in &inspection.devices {
        println!(
            "device: {} ({}) vendor=#x{:08x} product=#x{:08x} revision=#x{:08x}",
            device.type_name, device.name, device.vendor_id, device.product_code, device.revision
        );
        println!("  digest: {}", device.digest);
        println!("  sync managers: {}", device.sync_managers.len());
        println!(
            "  pdos: rx={} tx={}",
            device.rx_pdos.len(),
            device.tx_pdos.len()
        );
        for (direction, pdo) in device.pdos() {
            println!(
                "    {} 0x{:04x} sm={} fixed={} mandatory={} exclude={:?} excluded-sm={:?} entries={}",
                direction.as_str(),
                pdo.index,
                pdo.sm.map_or_else(|| "-".to_owned(), |sm| sm.to_string()),
                pdo.fixed,
                pdo.mandatory,
                pdo.exclude,
                pdo.excluded_sm,
                pdo.entries.len()
            );
            for entry in &pdo.entries {
                println!(
                    "      entry 0x{:04x}:{} bits={} name={} type={}",
                    entry.index,
                    entry.subindex,
                    entry.bit_len,
                    entry.name.as_deref().unwrap_or("-"),
                    entry.data_type.as_deref().unwrap_or("-")
                );
            }
        }
        match &device.dc {
            None => println!("  dc: none declared"),
            Some(dc) => {
                for mode in &dc.op_modes {
                    println!(
                        "  dc op mode {} `{}` assign-activate=#x{:04x} sync0={} factor={:?} shift0={} sync1={:?} shift1={:?}",
                        mode.index,
                        mode.name,
                        mode.assign_activate,
                        mode.sync0_cycle,
                        mode.sync0_factor,
                        mode.sync0_shift,
                        mode.sync1_cycle,
                        mode.sync1_shift
                    );
                    if let Some(description) = &mode.description {
                        println!("      description: {description}");
                    }
                }
            }
        }
        println!(
            "  coe: sdo-info={} pdo-assign={} pdo-config={} complete-access={}",
            device.mailbox.sdo_info,
            device.mailbox.pdo_assign,
            device.mailbox.pdo_config,
            device.mailbox.complete_access
        );
        println!("  dictionary objects: {}", device.dictionary.len());
    }
    for unsupported in &inspection.unsupported {
        println!("unsupported: {unsupported}");
    }
}

fn emit_topology(
    invocation: &Invocation,
    topology: &Topology,
    diagnostic: Option<&EngineeringDiagnostic>,
) {
    if invocation.format.is_json() {
        let json = Json::object([
            ("topology".into(), topology.to_json()),
            ("valid".into(), Json::Bool(diagnostic.is_none())),
            (
                "diagnostics".into(),
                Json::Array(
                    diagnostic
                        .iter()
                        .map(|diagnostic| diagnostic.to_json())
                        .collect(),
                ),
            ),
        ]);
        print!("{}", json.render());
        return;
    }
    println!(
        "topology: master {}, {} device(s)",
        topology.master,
        topology.devices.len()
    );
    for device in &topology.devices {
        println!(
            "  position {}: {} identity={}",
            device.position,
            device.name,
            device.identity.describe()
        );
        println!(
            "    serial: {}  alias: {}",
            device
                .serial
                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
            device
                .alias
                .map_or_else(|| "none".to_owned(), |value| value.to_string())
        );
    }
    match diagnostic {
        Some(diagnostic) => eprintln!("error[{}]: {}", diagnostic.code, diagnostic.message()),
        None => println!("valid: dense positions with exact identities"),
    }
}

fn emit_observed(invocation: &Invocation, observed: &ObservedTopology) {
    if invocation.format.is_json() {
        print!("{}", observed.to_json().render());
        return;
    }
    println!("master: {}", observed.master);
    if observed.slaves.is_empty() {
        println!("no slaves respond on the bus");
        return;
    }
    for slave in &observed.slaves {
        println!(
            "position {}: online={} al_state={} vendor=#x{:08x} product=#x{:08x} revision=#x{:08x}",
            slave.position,
            slave.online,
            slave
                .al_state
                .map_or_else(|| "-".to_owned(), |state| state.to_string()),
            slave.vendor_id,
            slave.product_code,
            slave.revision
        );
        println!(
            "  serial: {}  alias: {}",
            slave
                .serial
                .map_or_else(|| "not readable".to_owned(), |value| value.to_string()),
            slave
                .alias
                .map_or_else(|| "not readable".to_owned(), |value| value.to_string())
        );
    }
}

fn emit_comparison(invocation: &Invocation, comparison: &TopologyComparison) {
    if invocation.format.is_json() {
        print!("{}", comparison.to_json().render());
        return;
    }
    println!("master: {}", comparison.master);
    for position in &comparison.positions {
        println!(
            "position {} ({}): {}",
            position.position,
            position.device,
            position.result.as_str()
        );
        for check in &position.checks {
            println!("  {}", check_line(check));
        }
    }
    for slave in &comparison.extra {
        println!(
            "extra slave at position {}: vendor=#x{:08x} product=#x{:08x} revision=#x{:08x}",
            slave.position, slave.vendor_id, slave.product_code, slave.revision
        );
    }
    for diagnostic in &comparison.diagnostics {
        println!("diagnostic: {}", diagnostic.message());
    }
    let matched = comparison
        .positions
        .iter()
        .filter(|position| position.result.is_clean())
        .count();
    println!(
        "summary: {matched} of {} positions match, {} extra slave(s)",
        comparison.positions.len(),
        comparison.extra.len()
    );
    if comparison.is_match() {
        println!("result: match");
    } else {
        println!("result: different");
    }
}

fn emit_capture(invocation: &Invocation, output: &str, topology: &Topology) {
    if invocation.format.is_json() {
        let json = Json::object([
            ("output".into(), Json::string(output)),
            ("topology".into(), topology.to_json()),
        ]);
        print!("{}", json.render());
        return;
    }
    println!(
        "captured {} device(s) into {output}",
        topology.devices.len()
    );
    println!("serial and alias are captured only with `--pin-serial` and `--pin-alias`");
}

fn emit_slave(invocation: &Invocation, inspection: &SlaveInspection) {
    if invocation.format.is_json() {
        print!("{}", inspection.to_json().render());
        return;
    }
    println!("position: {}", inspection.position);
    match &inspection.observed {
        None => println!("observed: no slave responds at this position"),
        Some(slave) => {
            println!(
                "observed: online={} al_state={} vendor=#x{:08x} product=#x{:08x} revision=#x{:08x}",
                slave.online,
                slave
                    .al_state
                    .map_or_else(|| "-".to_owned(), |state| state.to_string()),
                slave.vendor_id,
                slave.product_code,
                slave.revision
            );
            println!(
                "  serial: {}  alias: {}",
                slave
                    .serial
                    .map_or_else(|| "not readable".to_owned(), |value| value.to_string()),
                slave
                    .alias
                    .map_or_else(|| "not readable".to_owned(), |value| value.to_string())
            );
        }
    }
    match &inspection.configured {
        None => println!("configured: no project device at this position"),
        Some(device) => println!(
            "configured: {} identity={}",
            device.name,
            device.identity.describe()
        ),
    }
    println!("result: {}", inspection.result.as_str());
    for check in &inspection.checks {
        println!("  {}", check_line(check));
    }
    for diagnostic in &inspection.diagnostics {
        println!("diagnostic: {}", diagnostic.message());
    }
}

fn check_line(check: &FieldCheck) -> String {
    let mut line = format!("{}: {}", check.field, check.result.as_str());
    match (&check.expected, &check.observed) {
        (Some(expected), Some(observed)) => {
            line.push_str(&format!(" (expected {expected}, observed {observed})"));
        }
        (None, Some(observed)) => line.push_str(&format!(" ({observed})")),
        (Some(expected), None) => line.push_str(&format!(" ({expected})")),
        (None, None) => {}
    }
    line
}

fn emit_live_pdos(invocation: &Invocation, live: &LivePdoState) {
    if invocation.format.is_json() {
        print!("{}", live.to_json().render());
        return;
    }
    println!("position: {}", live.position);
    if live.assignment.is_empty() && live.mapping.is_empty() {
        println!("read-back: unavailable (verification reports `unverified`, never a match)");
        return;
    }
    for assignment in &live.assignment {
        let pdos = assignment
            .pdos
            .iter()
            .map(|index| format!("0x{index:04x}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("sync manager {}: {pdos}", assignment.sm);
    }
    for mapping in &live.mapping {
        println!("pdo 0x{:04x}:", mapping.pdo);
        for entry in &mapping.entries {
            println!(
                "  entry 0x{:04x}:{} bits={}",
                entry.index, entry.subindex, entry.bit_len
            );
        }
    }
}

fn emit_config(invocation: &Invocation, configuration: &EngineeringConfiguration) {
    if invocation.format.is_json() {
        print!("{}", configuration.to_json().render());
        return;
    }
    println!(
        "configuration: master {}, {} device(s)",
        configuration.master,
        configuration.devices.len()
    );
    for device in &configuration.devices {
        println!(
            "  position {}: {} identity={} selected={} modes={}",
            device.position,
            device.name,
            device.identity.describe(),
            device.selected,
            device.modes.len()
        );
        if let Some(esi) = &device.esi {
            println!("    esi: {} ({})", esi.source, esi.digest);
        } else {
            println!("    esi: none persisted");
        }
        for mode in &device.modes {
            let marker = if mode.name == device.selected {
                "*"
            } else {
                " "
            };
            let dc = match &mode.dc {
                DcChoice::FreeRun => "free-run".to_owned(),
                DcChoice::OpMode(timing) => {
                    format!("op-mode `{}` sync0={} ns", timing.name, timing.sync0_cycle)
                }
            };
            println!(
                "    {marker} {}: {} PDO(s), {} channel(s), {} startup op(s), dc {dc}",
                mode.name,
                mode.pdos.pdos.len(),
                mode.channels.len(),
                mode.startup.len()
            );
        }
    }
}

fn emit_config_report(invocation: &Invocation, report: &ConfigurationReport) {
    if invocation.format.is_json() {
        print!("{}", report.to_json().render());
        return;
    }
    println!("master: {}", report.master);
    for device in &report.devices {
        println!(
            "position {} ({}): {} selected={} esi-resolved={}",
            device.position,
            device.name,
            if device.result.is_some() {
                "invalid"
            } else {
                "valid"
            },
            device.selected.as_deref().unwrap_or("none"),
            device.esi_resolved
        );
    }
    for diagnostic in &report.diagnostics {
        println!("diagnostic: {}", diagnostic.message());
    }
    println!(
        "result: {}",
        if report.is_valid() {
            "valid"
        } else {
            "invalid"
        }
    );
}

fn emit_setup(invocation: &Invocation, status: &SetupStatus) {
    if invocation.format.is_json() {
        print!("{}", status.to_json().render());
        return;
    }
    match &status.session {
        None => println!("setup: not active"),
        Some(session) => println!(
            "setup: active (session {}, pid {}, master {}, started {})",
            session.session, session.pid, session.master, session.started_unix
        ),
    }
    match (status.master_held, &status.master_owner) {
        (Some(true), Some(owner)) => println!("master: held by `{owner}`"),
        (Some(false), Some(owner)) => {
            println!("master: owned by `{owner}` (ownership conflict; no automatic stop)");
        }
        (_, Some(owner)) => println!("master: owned by `{owner}`"),
        (_, None) => println!("master: no owner reported (idle or not observable on this host)"),
    }
    if status.recovered {
        println!("recovered: an unreadable session record was discarded");
    }
    if status.session.is_some() && status.master_held.is_none() {
        println!(
            "note: the master reservation lapses with the owning process; run `stc ethercat \
             setup leave` to clear a session left by an interrupted invocation"
        );
    }
}
