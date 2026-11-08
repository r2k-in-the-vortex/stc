//! Default backend registration for the coordinator registry.
//!
//! This is the runtime platform layer for backend selection (D-201). On
//! Linux the `ethercat` and `ethercat-status` kinds use the shim-backed
//! backend; other hosts keep the protocol-neutral phase-1 mock so coordinator
//! tests and non-EtherCAT builds do not need an IgH shim. The `modbus-tcp` and
//! `modbus-tcp-status` kinds share one backend instance on every host.

use crate::coordinator::BackendRegistry;

/// Registers the default `ethercat` and `ethercat-status` factories.
pub(crate) fn register_ethercat(registry: &mut BackendRegistry) {
    #[cfg(target_os = "linux")]
    {
        registry.register("ethercat", "ethercat", || {
            Box::new(crate::io_ethercat::EthercatBackend::new())
        });
        registry.register("ethercat-status", "ethercat", || {
            Box::new(crate::io_ethercat::EthercatBackend::new())
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        registry.register("ethercat", "ethercat", || {
            Box::new(crate::io_mock::MockEthercatBackend::new())
        });
        registry.register("ethercat-status", "ethercat", || {
            Box::new(crate::io_mock::MockEthercatBackend::new())
        });
    }
}

/// Registers the default `modbus-tcp` and `modbus-tcp-status` factories.
pub(crate) fn register_modbus_tcp(registry: &mut BackendRegistry) {
    registry.register("modbus-tcp", "modbus-tcp", || {
        Box::new(crate::io_modbus_tcp::ModbusTcpBackend::new())
    });
    registry.register("modbus-tcp-status", "modbus-tcp", || {
        Box::new(crate::io_modbus_tcp::ModbusTcpBackend::new())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::Coordinator;
    use crate::module_info::{ChannelInfo, DeviceInfo, ModuleInfo};

    const MOCK_CONFIG: &str = r#"{
        "syncs": [
            {
                "index": 1,
                "direction": "input",
                "watchdog": "disable",
                "pdos": [{
                    "index": "0x1a00",
                    "entries": [
                        {"index": "0x3001", "subindex": 1, "bits": 8},
                        {"index": "0x3001", "subindex": 2, "bits": 8}
                    ]
                }]
            },
            {
                "index": 2,
                "direction": "output",
                "watchdog": "enable",
                "pdos": [{
                    "index": "0x1600",
                    "entries": [
                        {"index": "0x3101", "subindex": 1, "bits": 8},
                        {"index": "0x3101", "subindex": 2, "bits": 8}
                    ]
                }]
            }
        ],
        "pdo-channels": [
            {"name": "IN0", "entry": {"index": "0x3001", "subindex": 1}, "bit": 0},
            {"name": "IN1", "entry": {"index": "0x3001", "subindex": 2}, "bit": 0},
            {"name": "OUT0", "entry": {"index": "0x3101", "subindex": 1}, "bit": 0},
            {"name": "OUT1", "entry": {"index": "0x3101", "subindex": 2}, "bit": 0}
        ]
    }"#;

    /// The accepted EtherCAT exclusion is hardware support, not the
    /// configuration kind: non-Linux hosts keep the protocol-neutral mock so
    /// the coordinator validates the configuration and routing without an IgH
    /// shim. Linux uses the shim-backed backend and is covered by the Linux
    /// suite.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_default_registry_uses_the_protocol_neutral_ethercat_mock() {
        let registry = BackendRegistry::with_defaults();
        assert!(registry.contains_kind("ethercat"));
        assert!(registry.contains_kind("ethercat-status"));
        let channels = ["IN0", "IN1", "OUT0", "OUT1"]
            .into_iter()
            .map(|name| ChannelInfo {
                name: name.into(),
                direction: if name.starts_with("IN") {
                    "input".into()
                } else {
                    "output".into()
                },
                width_bits: 8,
            })
            .collect();
        let info = ModuleInfo {
            inputs_bytes: 1,
            outputs_bytes: 1,
            input_defaults: vec![0],
            devices: vec![DeviceInfo {
                name: "diewu".into(),
                kind: "ethercat".into(),
                config: Some(MOCK_CONFIG.into()),
                channels,
            }],
            ..ModuleInfo::default()
        };
        Coordinator::cold_start(&info, &registry).expect("mock backend cold start");
    }
}
