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
