//! `stc ethercat`: EtherCAT engineering commands (WL-7 phases 1-2).
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

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use stc_got::json::Json;
use stc_runtime::ethercat::api::{DescriptionSummary, EngineeringService, SourceInspection};
use stc_runtime::ethercat::diagnostics::{EngineeringDiagnostic, codes};
use stc_runtime::ethercat::mock::MockEngineeringHost;
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
                "    {} 0x{:04x} sm={} fixed={} mandatory={} entries={}",
                direction.as_str(),
                pdo.index,
                pdo.sm.map_or_else(|| "-".to_owned(), |sm| sm.to_string()),
                pdo.fixed,
                pdo.mandatory,
                pdo.entries.len()
            );
        }
        match &device.dc {
            None => println!("  dc: none declared"),
            Some(dc) => {
                let names: Vec<&str> = dc.op_modes.iter().map(|mode| mode.name.as_str()).collect();
                println!("  dc op modes: {}", names.join(", "));
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
