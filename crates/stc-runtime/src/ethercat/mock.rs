//! Host-neutral mock of the engineering host boundary (D-279).
//!
//! Engineering tests must exercise unavailable-capability, ownership-conflict
//! and structured-error cases on every CI host, not only where the IgH shim can
//! be built. The mock implements the same typed contract as the ABI 2 shim; it
//! returns facts and categories, never diagnostic codes.

use std::collections::BTreeMap;

use super::abi::{Capabilities, Capability, OperationResult, ResultCategory, SlaveIdentity};

/// A mock engineering host with capabilities, slaves and injectable failures.
#[derive(Clone, Debug)]
pub struct MockEngineeringHost {
    capabilities: Capabilities,
    slaves: Vec<SlaveIdentity>,
    failures: BTreeMap<Capability, ResultCategory>,
    owner: Option<String>,
}

impl Default for MockEngineeringHost {
    fn default() -> Self {
        Self::new(Capabilities::none())
    }
}

impl MockEngineeringHost {
    /// Creates a host reporting `capabilities`.
    #[must_use]
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            slaves: Vec::new(),
            failures: BTreeMap::new(),
            owner: None,
        }
    }

    /// Creates a host reporting every ABI 2 capability.
    #[must_use]
    pub fn complete() -> Self {
        Self::new(Capabilities::all())
    }

    /// Capabilities the host reports.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Enables or disables one capability.
    pub fn set_capability(&mut self, capability: Capability, enabled: bool) {
        self.capabilities.set(capability, enabled);
    }

    /// Adds or replaces the slave at its position.
    pub fn insert_slave(&mut self, identity: SlaveIdentity) {
        match self
            .slaves
            .iter_mut()
            .find(|slave| slave.position == identity.position)
        {
            Some(existing) => *existing = identity,
            None => self.slaves.push(identity),
        }
    }

    /// Slaves in insertion order.
    #[must_use]
    pub fn slaves(&self) -> &[SlaveIdentity] {
        &self.slaves
    }

    /// Returns the slave at `position`.
    #[must_use]
    pub fn slave(&self, position: u16) -> Option<&SlaveIdentity> {
        self.slaves.iter().find(|slave| slave.position == position)
    }

    /// Makes operations that require `capability` fail with `category`.
    pub fn fail(&mut self, capability: Capability, category: ResultCategory) {
        self.failures.insert(capability, category);
    }

    /// Clears injected failures.
    pub fn clear_failures(&mut self) {
        self.failures.clear();
    }

    /// Claims exclusive ownership for `owner`.
    ///
    /// A second claim fails with `ownership-conflict`; a release is required
    /// before a different owner can claim the host.
    pub fn claim(&mut self, owner: &str) -> OperationResult {
        match self.owner.as_deref() {
            Some(current) if current != owner => {
                OperationResult::failed(None, ResultCategory::OwnershipConflict)
            }
            _ => {
                self.owner = Some(owner.to_owned());
                OperationResult::ok(None)
            }
        }
    }

    /// Releases ownership held by `owner`.
    pub fn release(&mut self, owner: &str) {
        if self.owner.as_deref() == Some(owner) {
            self.owner = None;
        }
    }

    /// Current owner, when the host is claimed.
    #[must_use]
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// Reads identity facts at `position`.
    #[must_use]
    pub fn identity(&self, position: u16) -> OperationResult {
        if !self.capabilities.identity {
            return OperationResult::unavailable(Some(position));
        }
        match self.slave(position) {
            Some(slave) if slave.online => OperationResult::ok(Some(position)),
            Some(_) => OperationResult::failed(Some(position), ResultCategory::NotActive),
            None => OperationResult::unavailable(Some(position)),
        }
    }

    /// Requires one capability for an operation at `position`.
    ///
    /// Missing capability yields `unavailable`; an injected failure is reported
    /// as the configured category. The result never carries a diagnostic code.
    #[must_use]
    pub fn require(&self, capability: Capability, position: Option<u16>) -> OperationResult {
        if !self.capabilities.supports(capability) {
            return OperationResult::unavailable(position);
        }
        match self.failures.get(&capability) {
            Some(category) => OperationResult::failed(position, *category),
            None => OperationResult::ok(position),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slave(position: u16, online: bool) -> SlaveIdentity {
        SlaveIdentity {
            position,
            vendor_id: 2,
            product_code: 2,
            revision: 1,
            serial: None,
            alias: None,
            al_state: Some(8),
            online,
        }
    }

    #[test]
    fn default_host_reports_no_capabilities() {
        let host = MockEngineeringHost::default();
        assert_eq!(host.capabilities(), Capabilities::none());
        assert_eq!(host.identity(0).category, ResultCategory::Unavailable);
        assert!(!host.identity(0).available);
    }

    #[test]
    fn identity_reports_online_offline_and_missing_slaves() {
        let mut host = MockEngineeringHost::complete();
        host.insert_slave(slave(0, true));
        host.insert_slave(slave(1, false));
        assert_eq!(host.identity(0).category, ResultCategory::Ok);
        assert_eq!(host.identity(0).position, Some(0));
        assert_eq!(host.identity(1).category, ResultCategory::NotActive);
        assert_eq!(host.identity(2).category, ResultCategory::Unavailable);
        assert!(host.slave(0).is_some());
        assert!(host.slave(9).is_none());
        assert_eq!(host.slaves().len(), 2);

        // Replacing a position keeps one entry.
        host.insert_slave(slave(0, false));
        assert_eq!(host.slaves().len(), 2);
        assert_eq!(host.identity(0).category, ResultCategory::NotActive);
    }

    #[test]
    fn missing_capability_and_injected_failures_are_structured() {
        let mut host = MockEngineeringHost::complete();
        host.set_capability(Capability::SdoInformation, false);
        let result = host.require(Capability::SdoInformation, Some(1));
        assert_eq!(result.category, ResultCategory::Unavailable);
        assert!(!result.available);
        assert!(!format!("{result:?}").contains("STC-"));

        host.fail(Capability::SdoUpload, ResultCategory::Abort);
        let result = host.require(Capability::SdoUpload, Some(1));
        assert_eq!(result.category, ResultCategory::Abort);
        assert_eq!(result.position, Some(1));

        host.clear_failures();
        assert_eq!(
            host.require(Capability::SdoUpload, Some(1)).category,
            ResultCategory::Ok
        );
        assert_eq!(
            host.require(Capability::DcReadBack, None).category,
            ResultCategory::Ok
        );
    }

    #[test]
    fn ownership_is_exclusive_until_released() {
        let mut host = MockEngineeringHost::complete();
        assert_eq!(host.claim("setup-1").category, ResultCategory::Ok);
        assert_eq!(host.owner(), Some("setup-1"));
        assert_eq!(host.claim("setup-1").category, ResultCategory::Ok);
        assert_eq!(
            host.claim("runtime").category,
            ResultCategory::OwnershipConflict
        );
        host.release("runtime");
        assert_eq!(host.owner(), Some("setup-1"));
        host.release("setup-1");
        assert_eq!(host.owner(), None);
        assert_eq!(host.claim("runtime").category, ResultCategory::Ok);
    }
}
