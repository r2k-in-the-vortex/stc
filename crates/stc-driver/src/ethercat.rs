//! `stc ethercat`: EtherCAT engineering commands (WL-7 phase 1).
//!
//! The handler only wires the typed engineering API to the terminal. All
//! engineering behavior, validation and diagnostics live in `stc-runtime`, so a
//! later TUI/GUI calls the same operations without duplicating logic (D-259,
//! D-270).

use std::path::Path;

use stc_got::json::Json;
use stc_runtime::ethercat::api::{DescriptionSummary, EngineeringService, SourceInspection};
use stc_runtime::ethercat::diagnostics::EngineeringDiagnostic;

use crate::cli::{EthercatAction, Invocation};

/// Runs one `stc ethercat` action and returns the process exit code.
///
/// Exit codes: `0` success, `1` the requested engineering operation was
/// rejected (with an `STC-E-ESI-*` diagnostic), `2` a usage error.
pub(crate) fn run(invocation: &Invocation) -> u8 {
    let Some(action) = invocation.ethercat.action.as_ref() else {
        eprintln!("stc: `ethercat` needs an action");
        return 2;
    };
    let service = match &invocation.ethercat.esi_dir {
        Some(directory) => EngineeringService::open(Path::new(directory)),
        None => Ok(EngineeringService::standalone()),
    };
    let service = match service {
        Ok(service) => service,
        Err(diagnostic) => return reject(&diagnostic),
    };
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
    }
}

fn reject(diagnostic: &EngineeringDiagnostic) -> u8 {
    eprintln!("error[{}]: {}", diagnostic.code, diagnostic.message());
    1
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
