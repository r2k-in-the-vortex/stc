//! Host-neutral mock of the engineering host boundary (D-279).
//!
//! Engineering tests must exercise unavailable-capability, ownership-conflict
//! and structured-error cases on every CI host, not only where the IgH shim can
//! be built. The mock implements the same typed contract as the ABI 2 shim; it
//! returns facts and categories, never diagnostic codes.

use std::collections::{BTreeMap, BTreeSet};

use super::abi::{Capabilities, Capability, OperationResult, ResultCategory, SlaveIdentity};
use super::pdo::{LivePdoState, PdoSelection};

/// A mock engineering host with capabilities, slaves and injectable failures.
#[derive(Clone, Debug)]
pub struct MockEngineeringHost {
    capabilities: Capabilities,
    master: u16,
    slaves: Vec<SlaveIdentity>,
    failures: BTreeMap<Capability, ResultCategory>,
    owner: Option<String>,
    live_pdos: BTreeMap<u16, LivePdoState>,
    remap_refusals: BTreeSet<u16>,
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
            master: 0,
            slaves: Vec::new(),
            failures: BTreeMap::new(),
            owner: None,
            live_pdos: BTreeMap::new(),
            remap_refusals: BTreeSet::new(),
        }
    }

    /// The managed master index the host reports.
    #[must_use]
    pub fn master(&self) -> u16 {
        self.master
    }

    /// Selects the managed master index the host reports.
    pub fn set_master(&mut self, master: u16) {
        self.master = master;
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

    /// Records the live PDO assignment/mapping read back at `position`.
    pub fn set_live_pdos(&mut self, state: LivePdoState) {
        self.live_pdos.insert(state.position, state);
    }

    /// The live PDO state at `position`, when the host can read it back.
    ///
    /// A host without the `pdo-read-back` capability reports no facts at all;
    /// callers must treat a missing record as `unverified`, never a match.
    #[must_use]
    pub fn live_pdos(&self, position: u16) -> Option<&LivePdoState> {
        if !self.capabilities.pdo_read_back {
            return None;
        }
        self.live_pdos.get(&position)
    }

    /// Makes the live device refuse every remap at `position`.
    pub fn fail_remap(&mut self, position: u16) {
        self.remap_refusals.insert(position);
    }

    /// Applies a PDO selection to the mock device (D-269, D-279).
    ///
    /// The mock reports a structured result only: a missing capability is
    /// `unavailable`, an injected failure or a refused remap is its category,
    /// and success updates the readable live state.
    pub fn apply_pdos(&mut self, position: u16, selection: &PdoSelection) -> OperationResult {
        if !self.capabilities.pdo_configuration {
            return OperationResult::unavailable(Some(position));
        }
        if let Some(category) = self.failures.get(&Capability::PdoConfiguration) {
            return OperationResult::failed(Some(position), *category);
        }
        if self.remap_refusals.contains(&position) {
            return OperationResult::failed(Some(position), ResultCategory::RemapRefused);
        }
        self.live_pdos
            .insert(position, LivePdoState::from_selection(position, selection));
        OperationResult::ok(Some(position))
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
        assert_eq!(host.master(), 0);
        host.set_master(1);
        assert_eq!(host.master(), 1);
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

    #[test]
    fn pdo_read_back_and_apply_are_structured() {
        use super::super::pdo::{
            EntryKey, LiveEntry, LivePdoAssignment, LivePdoMapping, SelectedEntry, SelectedPdo,
        };

        let mut host = MockEngineeringHost::complete();
        assert!(host.live_pdos(0).is_none());
        let mut state = LivePdoState::new(0);
        state.set_assignment(LivePdoAssignment::new(2, vec![0x1600]));
        state.set_mapping(LivePdoMapping::new(
            0x1600,
            vec![LiveEntry::new(0x7000, 1, 16)],
        ));
        host.set_live_pdos(state);
        assert!(host.live_pdos(0).is_some());

        host.set_capability(Capability::PdoReadBack, false);
        assert!(host.live_pdos(0).is_none());
        host.set_capability(Capability::PdoReadBack, true);

        let selection = PdoSelection {
            pdos: vec![SelectedPdo {
                direction: super::super::esi::model::PdoDirection::Rx,
                index: 0x1600,
                sm: 2,
                fixed: false,
                mandatory: false,
                exclude: Vec::new(),
                excluded_sm: Vec::new(),
                entries: vec![SelectedEntry {
                    index: 0x7000,
                    subindex: 1,
                    bit_len: 16,
                    declared_in: 0x1600,
                }],
            }],
        };
        host.fail_remap(1);
        assert_eq!(
            host.apply_pdos(1, &selection).category,
            ResultCategory::RemapRefused
        );
        assert_eq!(host.apply_pdos(0, &selection).category, ResultCategory::Ok);
        assert_eq!(
            host.live_pdos(0)
                .and_then(|state| state.mapping_for(0x1600))
                .map(|entries| entries[0].key()),
            Some(EntryKey::new(0x7000, 1))
        );
        host.set_capability(Capability::PdoConfiguration, false);
        assert_eq!(
            host.apply_pdos(0, &selection).category,
            ResultCategory::Unavailable
        );
        assert!(!format!("{selection:?}").contains("STC-"));
    }
}
