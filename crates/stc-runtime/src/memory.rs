//! Host-owned persistent roots, quarantine, and transient module storage.
//!
//! The host allocates every persistent root (Online Change and GOT §4.1) and
//! every transient module block (GOT Artifacts §8.2). Root storage never moves:
//! a preserved root keeps its address across generations, and a removed root is
//! quarantined rather than freed so stale references in preserved data remain
//! addressable until the host restarts or clears the quarantine.
//!
//! Transient blocks are owned by one module generation and are reclaimed on a
//! failed prepare, retirement, or unload. They are never manifest roots and
//! must not be stored in persistent storage.
//!
//! Storage is aligned by over-allocating a byte buffer and choosing an aligned
//! offset inside it. The buffer therefore stays a normal `Box<[u8]>`; no raw
//! allocator calls or unsafe code are required.

use std::collections::BTreeMap;

use stc_got::error::{Error, Result, codes};
use stc_got::{GotId, RetiredCause, RootKind, RootRecord};

/// Host resource limits for persistent and transient allocations.
///
/// GOT Artifacts §2.4 requires a reader to enforce a configured memory budget
/// rather than trusting artifact sizes. The defaults are deliberately generous
/// for the M4 scalar surface and can be lowered or raised by an embedding host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryLimits {
    /// Maximum bytes held by live storage of this kind.
    pub max_total_bytes: u64,
    /// Maximum size of one root or transient block.
    pub max_allocation_bytes: u64,
    /// Maximum alignment accepted for one root or transient block.
    pub max_align: u32,
}

impl MemoryLimits {
    /// Limits for persistent root storage.
    #[must_use]
    pub const fn roots() -> Self {
        Self {
            max_total_bytes: 256 * 1024 * 1024,
            max_allocation_bytes: 64 * 1024 * 1024,
            max_align: 4096,
        }
    }

    /// Limits for transient module allocations.
    #[must_use]
    pub const fn transient() -> Self {
        Self {
            max_total_bytes: 64 * 1024 * 1024,
            max_allocation_bytes: 16 * 1024 * 1024,
            max_align: 4096,
        }
    }
}

impl Default for MemoryLimits {
    fn default() -> Self {
        Self::roots()
    }
}

/// A zeroed host allocation with a stable, aligned address.
#[derive(Debug)]
struct AlignedStorage {
    bytes: Box<[u8]>,
    offset: usize,
    size: usize,
}

impl AlignedStorage {
    /// Allocates at least `max(size, 1)` zeroed bytes aligned to `align`.
    fn allocate(size: u64, align: u32, limits: &MemoryLimits, what: &str) -> Result<Self> {
        if align == 0 || !align.is_power_of_two() {
            return Err(Error::new(
                codes::MANIFEST_INVALID,
                format!("{what} alignment {align} is not a power of two"),
            ));
        }
        if align > limits.max_align {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!(
                    "{what} alignment {align} exceeds the host limit {}",
                    limits.max_align
                ),
            ));
        }
        let amount = size.max(1);
        if amount > limits.max_allocation_bytes {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!(
                    "{what} size {amount} exceeds the host limit {}",
                    limits.max_allocation_bytes
                ),
            ));
        }
        let size = usize::try_from(amount).map_err(|_| {
            Error::new(
                codes::ONLINE_PREPARE,
                format!("{what} size {amount} exceeds the platform limit"),
            )
        })?;
        let align = usize::try_from(align).map_err(|_| {
            Error::new(
                codes::ONLINE_PREPARE,
                format!("{what} alignment exceeds the platform limit"),
            )
        })?;
        let capacity = size.checked_add(align - 1).ok_or_else(|| {
            Error::new(
                codes::ONLINE_PREPARE,
                format!("{what} size and alignment overflow"),
            )
        })?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| {
            Error::new(
                codes::ONLINE_PREPARE,
                format!("{what} of {size} bytes cannot be allocated"),
            )
        })?;
        bytes.resize(capacity, 0);
        let bytes = bytes.into_boxed_slice();
        let base = bytes.as_ptr() as usize;
        let offset = base.wrapping_neg() & (align - 1);
        Ok(Self {
            bytes,
            offset,
            size,
        })
    }

    /// Returns the aligned storage address.
    fn address(&self) -> usize {
        self.bytes.as_ptr() as usize + self.offset
    }

    /// Returns the aligned storage bytes.
    fn as_slice(&self) -> &[u8] {
        &self.bytes[self.offset..self.offset + self.size]
    }

    /// Returns the aligned storage bytes mutably.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes[self.offset..self.offset + self.size]
    }

    /// Returns the storage length.
    fn len(&self) -> usize {
        self.size
    }

    /// Returns the bytes actually allocated, including alignment padding.
    fn allocated_len(&self) -> usize {
        self.bytes.len()
    }
}

/// Host-owned storage for one persistent root.
///
/// The address is stable for the lifetime of the storage, including while the
/// owning registry entry moves between the active and quarantine sets.
#[derive(Debug)]
pub struct RootStorage {
    storage: AlignedStorage,
}

impl RootStorage {
    fn allocate(record: &RootRecord, limits: &MemoryLimits) -> Result<Self> {
        Ok(Self {
            storage: AlignedStorage::allocate(
                record.size,
                record.align,
                limits,
                &format!("root `{}`", record.key),
            )?,
        })
    }

    /// Returns the stable aligned storage address.
    #[must_use]
    pub fn address(&self) -> usize {
        self.storage.address()
    }

    /// Returns the storage bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.storage.as_slice()
    }

    /// Returns the storage bytes mutably.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.storage.as_mut_slice()
    }

    /// Returns the storage length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns the bytes actually allocated, including alignment padding.
    #[must_use]
    pub fn allocated_len(&self) -> usize {
        self.storage.allocated_len()
    }

    /// Returns `false`; roots are always allocated with at least one byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One active persistent root.
#[derive(Debug)]
pub struct ActiveRoot {
    /// The current manifest record, including the current key.
    pub record: RootRecord,
    /// The generation that allocated the root's storage.
    pub generation: u64,
    storage: RootStorage,
}

impl ActiveRoot {
    /// Returns the current record.
    #[must_use]
    pub fn record(&self) -> &RootRecord {
        &self.record
    }

    /// Returns the stable storage address.
    #[must_use]
    pub fn address(&self) -> usize {
        self.storage.address()
    }

    /// Returns the storage bytes.
    #[must_use]
    pub fn storage(&self) -> &[u8] {
        self.storage.as_slice()
    }

    /// Returns the storage bytes mutably.
    #[must_use]
    pub fn storage_mut(&mut self) -> &mut [u8] {
        self.storage.as_mut_slice()
    }
}

/// One quarantined root.
///
/// Quarantined storage stays allocated until the host restarts or explicitly
/// clears the quarantine; it is never matched by a later generation.
#[derive(Debug)]
pub struct QuarantinedRoot {
    /// The record that was active when the root was removed.
    pub record: RootRecord,
    /// The generation that allocated the root's storage.
    pub generation: u64,
    /// Why the root left the active set.
    pub cause: RetiredCause,
    storage: RootStorage,
}

impl QuarantinedRoot {
    /// Returns the retired record.
    #[must_use]
    pub fn record(&self) -> &RootRecord {
        &self.record
    }

    /// Returns the retained storage address.
    #[must_use]
    pub fn address(&self) -> usize {
        self.storage.address()
    }

    /// Returns the retained storage bytes.
    #[must_use]
    pub fn storage(&self) -> &[u8] {
        self.storage.as_slice()
    }
}

/// The host root registry.
///
/// The registry only tracks persistent roots. Direct `AT` variables use direct
/// cells that point into the process image (Online Change and GOT §3.2), so
/// they are rejected here and resolved by the host's process-image binding.
#[derive(Debug)]
pub struct RootRegistry {
    limits: MemoryLimits,
    active: BTreeMap<GotId, ActiveRoot>,
    quarantined: Vec<QuarantinedRoot>,
    reserved_bytes: u64,
}

impl Default for RootRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RootRegistry {
    /// Creates an empty registry with the default root limits.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MemoryLimits::roots())
    }

    /// Creates an empty registry with explicit limits.
    #[must_use]
    pub fn with_limits(limits: MemoryLimits) -> Self {
        Self {
            limits,
            active: BTreeMap::new(),
            quarantined: Vec::new(),
            reserved_bytes: 0,
        }
    }

    /// Returns the configured limits.
    #[must_use]
    pub fn limits(&self) -> MemoryLimits {
        self.limits
    }

    /// Returns the number of active roots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Returns `true` when no root is active.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Returns `true` when `got_id` is an active root.
    #[must_use]
    pub fn contains(&self, got_id: GotId) -> bool {
        self.active.contains_key(&got_id)
    }

    /// Returns the active root for an identity.
    #[must_use]
    pub fn get(&self, got_id: GotId) -> Option<&ActiveRoot> {
        self.active.get(&got_id)
    }

    /// Returns the active root for an identity mutably.
    #[must_use]
    pub fn get_mut(&mut self, got_id: GotId) -> Option<&mut ActiveRoot> {
        self.active.get_mut(&got_id)
    }

    /// Returns the stable storage address for an active root.
    #[must_use]
    pub fn address(&self, got_id: GotId) -> Option<usize> {
        self.active.get(&got_id).map(ActiveRoot::address)
    }

    /// Returns the storage bytes for an active root.
    #[must_use]
    pub fn storage(&self, got_id: GotId) -> Option<&[u8]> {
        self.active.get(&got_id).map(ActiveRoot::storage)
    }

    /// Iterates over active roots in identity order.
    pub fn active(&self) -> impl Iterator<Item = &ActiveRoot> {
        self.active.values()
    }

    /// Returns the quarantined roots.
    #[must_use]
    pub fn quarantined(&self) -> &[QuarantinedRoot] {
        &self.quarantined
    }

    /// Returns the number of bytes reserved by active and quarantined storage.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    /// Allocates a zeroed persistent root and registers it as active.
    ///
    /// # Errors
    ///
    /// Rejects a zero identity, a direct root, a zero size, a non-power-of-two
    /// alignment, a duplicate active identity, or a host limit violation.
    pub fn allocate(&mut self, record: &RootRecord, generation: u64) -> Result<()> {
        let (storage, total) = self.prepare_storage(record, false)?;
        self.reserved_bytes = total;
        self.active.insert(
            record.got_id,
            ActiveRoot {
                record: record.clone(),
                generation,
                storage,
            },
        );
        Ok(())
    }

    /// Validates a root and allocates its storage without changing the active
    /// set. The caller commits the storage only after every check passed.
    fn prepare_storage(
        &self,
        record: &RootRecord,
        allow_existing: bool,
    ) -> Result<(RootStorage, u64)> {
        validate_record(record)?;
        if !allow_existing && self.active.contains_key(&record.got_id) {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!("root `{}` is already active", record.key),
            ));
        }
        let storage = RootStorage::allocate(record, &self.limits)?;
        let amount = u64::try_from(storage.allocated_len()).map_err(|_| {
            Error::new(
                codes::ONLINE_PREPARE,
                "root storage size exceeds the u64 limit",
            )
        })?;
        let total = self
            .reserved_bytes
            .checked_add(amount)
            .ok_or_else(|| Error::new(codes::ONLINE_PREPARE, "root storage total overflows"))?;
        if total > self.limits.max_total_bytes {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!(
                    "root `{}` would reserve {total} bytes, above the host limit {}",
                    record.key, self.limits.max_total_bytes
                ),
            ));
        }
        Ok((storage, total))
    }

    /// Preserves an existing root for a new generation.
    ///
    /// The `GotId`, type key, layout hash, size, alignment, and root kind must
    /// match exactly. Storage is not reinitialized, and the active record is
    /// replaced so a tracked rename updates the current key.
    ///
    /// # Errors
    ///
    /// Rejects a direct root, a missing root, or any storage-shape change.
    pub fn preserve(&mut self, record: &RootRecord) -> Result<()> {
        if record.root_kind == RootKind::Direct {
            return Err(Error::new(
                codes::ONLINE_LAYOUT,
                format!(
                    "direct variable `{}` is per generation and cannot be preserved",
                    record.key
                ),
            ));
        }
        let Some(existing) = self.active.get(&record.got_id) else {
            return Err(Error::new(
                codes::ONLINE_LAYOUT,
                format!(
                    "root `{}` ({}) is not active and cannot be preserved",
                    record.key, record.got_id
                ),
            ));
        };
        if existing.record.root_kind != record.root_kind
            || existing.record.type_key != record.type_key
            || existing.record.layout_hash != record.layout_hash
            || existing.record.size != record.size
            || existing.record.align != record.align
        {
            return Err(Error::new(
                codes::ONLINE_LAYOUT,
                format!(
                    "root `{}` layout changed: `{}`/{} -> `{}`/{}",
                    record.key,
                    existing.record.type_key,
                    existing.record.size,
                    record.type_key,
                    record.size
                ),
            ));
        }
        if let Some(existing) = self.active.get_mut(&record.got_id) {
            existing.record = record.clone();
        }
        Ok(())
    }

    /// Moves an active root into quarantine without freeing its storage.
    ///
    /// # Errors
    ///
    /// Rejects an identity that is not active.
    pub fn quarantine(&mut self, got_id: GotId, cause: RetiredCause) -> Result<()> {
        let Some(active) = self.active.remove(&got_id) else {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!("root {got_id} is not active and cannot be quarantined"),
            ));
        };
        self.quarantined.push(QuarantinedRoot {
            record: active.record,
            generation: active.generation,
            cause,
            storage: active.storage,
        });
        Ok(())
    }

    /// Replaces a root with fresh zeroed storage, quarantining the old one.
    ///
    /// The new storage is allocated before the old root is quarantined, so a
    /// failure leaves the previous root active and intact. M4.5 plans never
    /// emit `Reset`; this primitive exists for the explicit reset policy and
    /// host tests.
    ///
    /// # Errors
    ///
    /// Applies the same checks as [`Self::allocate`].
    pub fn reset(&mut self, record: &RootRecord, generation: u64) -> Result<()> {
        let (storage, total) = self.prepare_storage(record, true)?;
        if self.active.contains_key(&record.got_id) {
            self.quarantine(record.got_id, RetiredCause::Reset)?;
        }
        self.reserved_bytes = total;
        self.active.insert(
            record.got_id,
            ActiveRoot {
                record: record.clone(),
                generation,
                storage,
            },
        );
        Ok(())
    }

    /// Frees the storage of a root that never became active.
    ///
    /// The host uses this to roll back roots allocated by a candidate whose
    /// prepare or load failed. The caller must only discard identities that
    /// were allocated for the failed candidate: discarding a root still visible
    /// to the active generation would invalidate its cells.
    ///
    /// Unlike [`Self::quarantine`], this frees the storage immediately.
    ///
    /// # Errors
    ///
    /// Rejects an identity that is not active.
    pub fn discard(&mut self, got_id: GotId) -> Result<()> {
        let Some(root) = self.active.remove(&got_id) else {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!("root {got_id} is not active and cannot be discarded"),
            ));
        };
        let freed = u64::try_from(root.storage.allocated_len()).unwrap_or(u64::MAX);
        self.reserved_bytes = self.reserved_bytes.saturating_sub(freed);
        Ok(())
    }

    /// Frees every quarantined root.
    ///
    /// Returns the number of roots released. This is the only operation that
    /// invalidates quarantined addresses; the host calls it on restart or under
    /// an explicit reset policy.
    pub fn clear_quarantine(&mut self) -> usize {
        let freed = self.quarantined.iter().fold(0_u64, |total, root| {
            total.saturating_add(u64::try_from(root.storage.allocated_len()).unwrap_or(u64::MAX))
        });
        let count = self.quarantined.len();
        self.quarantined.clear();
        self.reserved_bytes = self.reserved_bytes.saturating_sub(freed);
        count
    }
}

fn validate_record(record: &RootRecord) -> Result<()> {
    if record.got_id.is_zero() {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!("root `{}` has a zero GotId", record.key),
        ));
    }
    if record.root_kind == RootKind::Direct {
        return Err(Error::new(
            codes::MODULE_INVALID,
            format!(
                "direct variable `{}` uses a direct cell, not persistent storage",
                record.key
            ),
        ));
    }
    if record.size == 0 {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!("root `{}` has zero size", record.key),
        ));
    }
    if !record.align.is_power_of_two() {
        return Err(Error::new(
            codes::MANIFEST_INVALID,
            format!(
                "root `{}` alignment {} is not a power of two",
                record.key, record.align
            ),
        ));
    }
    Ok(())
}

/// Host-owned transient allocations for one module generation.
///
/// Blocks are zeroed, aligned, and stable while allocated. `free` accepts only
/// exact base addresses returned by [`Self::alloc`] from this allocator; an
/// unknown, null, or double free is rejected as a module fault. All blocks are
/// reclaimed by [`Self::free_all`] on failed prepare, retirement, or unload.
#[derive(Debug)]
pub struct TransientAllocator {
    generation: u64,
    limits: MemoryLimits,
    blocks: BTreeMap<usize, AlignedStorage>,
    reserved_bytes: u64,
}

impl TransientAllocator {
    /// Creates an allocator for one module generation.
    #[must_use]
    pub fn new(generation: u64) -> Self {
        Self::with_limits(generation, MemoryLimits::transient())
    }

    /// Creates an allocator with explicit limits.
    #[must_use]
    pub fn with_limits(generation: u64, limits: MemoryLimits) -> Self {
        Self {
            generation,
            limits,
            blocks: BTreeMap::new(),
            reserved_bytes: 0,
        }
    }

    /// Returns the owning generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the number of live blocks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Returns `true` when no block is allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Returns the bytes reserved by live blocks.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    /// Allocates a zeroed transient block and returns its address.
    ///
    /// A zero-size request still receives a unique one-byte block so the
    /// returned address is non-null and freeable.
    ///
    /// # Errors
    ///
    /// Rejects an invalid alignment or a host limit violation.
    pub fn alloc(&mut self, size: u64, align: u32) -> Result<usize> {
        let storage = AlignedStorage::allocate(size, align, &self.limits, "transient block")?;
        let amount = u64::try_from(storage.allocated_len()).map_err(|_| {
            Error::new(
                codes::ONLINE_PREPARE,
                "transient allocation size exceeds the u64 limit",
            )
        })?;
        let total = self.reserved_bytes.checked_add(amount).ok_or_else(|| {
            Error::new(
                codes::ONLINE_PREPARE,
                "transient allocation total overflows",
            )
        })?;
        if total > self.limits.max_total_bytes {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!(
                    "transient allocation would reserve {total} bytes, above the host limit {}",
                    self.limits.max_total_bytes
                ),
            ));
        }
        let address = storage.address();
        if self.blocks.contains_key(&address) {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                "transient allocation produced a duplicate address",
            ));
        }
        self.blocks.insert(address, storage);
        self.reserved_bytes = total;
        Ok(address)
    }

    /// Releases a block returned by [`Self::alloc`].
    ///
    /// # Errors
    ///
    /// Rejects null, unknown, interior, and double frees.
    pub fn free(&mut self, address: usize) -> Result<()> {
        let Some(storage) = self.blocks.remove(&address) else {
            return Err(Error::new(
                codes::ONLINE_PREPARE,
                format!("unknown or double free of transient block {address:#x}"),
            ));
        };
        self.reserved_bytes = self
            .reserved_bytes
            .saturating_sub(u64::try_from(storage.allocated_len()).unwrap_or(u64::MAX));
        Ok(())
    }

    /// Returns `true` when `address` is the base of a live block.
    #[must_use]
    pub fn owns(&self, address: usize) -> bool {
        self.blocks.contains_key(&address)
    }

    /// Returns `true` when `[address, address + len)` overlaps a live block.
    ///
    /// The host uses this to reject an attempt to register transient storage as
    /// a persistent root or to publish it as a runtime object.
    #[must_use]
    pub fn overlaps(&self, address: usize, len: u64) -> bool {
        let Some(end) = address.checked_add(usize::try_from(len).unwrap_or(usize::MAX)) else {
            return true;
        };
        self.blocks.iter().any(|(base, storage)| {
            let block_end = base.saturating_add(storage.len());
            address < block_end && *base < end
        })
    }

    /// Releases every block, as on failed prepare, retirement, or unload.
    ///
    /// Returns the number of bytes released.
    pub fn free_all(&mut self) -> u64 {
        let freed = self.reserved_bytes;
        self.blocks.clear();
        self.reserved_bytes = 0;
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::{InitializerKind, Retention};

    fn root(key: &str, id: u8, size: u64, align: u32) -> RootRecord {
        RootRecord {
            got_id: GotId::from_bytes([id; 16]),
            key: key.into(),
            previous_keys: Vec::new(),
            root_kind: RootKind::Global,
            owner_type: None,
            field_path: String::new(),
            type_key: "DINT".into(),
            layout_hash: GotId::from_bytes([0x22; 16]),
            size,
            align,
            retention: Retention::None,
            initializer: InitializerKind::Zero,
            initializer_digest: None,
            source: None,
            slot: 0,
        }
    }

    fn different_layout(record: &RootRecord) -> RootRecord {
        let mut changed = record.clone();
        changed.type_key = "INT".into();
        changed.layout_hash = GotId::from_bytes([0x33; 16]);
        changed.size = 2;
        changed.align = 2;
        changed
    }

    #[test]
    fn allocation_is_zeroed_aligned_and_stable() {
        let mut registry = RootRegistry::new();
        let first = root("_ROOT::A", 1, 8, 8);
        let second = root("_ROOT::B", 2, 4, 4);
        registry.allocate(&first, 1).expect("allocate first");
        registry.allocate(&second, 1).expect("allocate second");

        let address = registry.address(first.got_id).expect("first address");
        assert_eq!(address % 8, 0);
        assert_eq!(registry.storage(first.got_id).unwrap(), &[0; 8]);
        assert_eq!(registry.address(second.got_id).unwrap() % 4, 0);

        registry
            .get_mut(first.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        registry.allocate(&root("_ROOT::C", 3, 64, 16), 1).unwrap();
        assert_eq!(registry.address(first.got_id), Some(address));
        assert_eq!(
            registry.storage(first.got_id).unwrap(),
            &[1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn preserve_keeps_storage_and_updates_the_key() {
        let mut registry = RootRegistry::new();
        let original = root("_ROOT::COUNTER", 1, 4, 4);
        registry.allocate(&original, 1).unwrap();
        let address = registry.address(original.got_id).unwrap();
        registry
            .get_mut(original.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&7_u32.to_le_bytes());

        let mut renamed = original.clone();
        renamed.key = "_ROOT::COUNT".into();
        renamed.previous_keys = vec!["_ROOT::COUNTER".into()];
        registry.preserve(&renamed).expect("preserve");

        assert_eq!(registry.address(original.got_id), Some(address));
        assert_eq!(
            registry.storage(original.got_id).unwrap(),
            &7_u32.to_le_bytes()
        );
        assert_eq!(
            registry.get(original.got_id).unwrap().record.key,
            "_ROOT::COUNT"
        );
    }

    #[test]
    fn preserve_rejects_layout_change_and_missing_root() {
        let mut registry = RootRegistry::new();
        let original = root("_ROOT::COUNTER", 1, 4, 4);
        registry.allocate(&original, 1).unwrap();

        let error = registry
            .preserve(&different_layout(&original))
            .expect_err("layout change");
        assert_eq!(error.code, codes::ONLINE_LAYOUT);

        let error = registry
            .preserve(&root("_ROOT::OTHER", 2, 4, 4))
            .expect_err("missing root");
        assert_eq!(error.code, codes::ONLINE_LAYOUT);
    }

    #[test]
    fn quarantine_retains_storage_until_cleared() {
        let mut registry = RootRegistry::new();
        let record = root("_ROOT::COUNTER", 1, 4, 4);
        registry.allocate(&record, 1).unwrap();
        let address = registry.address(record.got_id).unwrap();
        registry
            .get_mut(record.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&9_u32.to_le_bytes());

        registry
            .quarantine(record.got_id, RetiredCause::Dropped)
            .expect("quarantine");
        assert!(!registry.contains(record.got_id));
        assert_eq!(registry.quarantined().len(), 1);
        assert_eq!(registry.quarantined()[0].address(), address);
        assert_eq!(registry.quarantined()[0].storage(), &9_u32.to_le_bytes());
        assert_eq!(registry.quarantined()[0].cause, RetiredCause::Dropped);
        assert_eq!(registry.reserved_bytes(), 7);

        assert_eq!(registry.clear_quarantine(), 1);
        assert!(registry.quarantined().is_empty());
        assert_eq!(registry.reserved_bytes(), 0);
    }

    #[test]
    fn reset_quarantines_old_storage_and_allocates_zeroed_storage() {
        let mut registry = RootRegistry::new();
        let record = root("_ROOT::COUNTER", 1, 4, 4);
        registry.allocate(&record, 1).unwrap();
        let address = registry.address(record.got_id).unwrap();
        registry
            .get_mut(record.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&[0xFF; 4]);

        registry.reset(&record, 2).expect("reset");

        let new_address = registry.address(record.got_id).unwrap();
        assert_ne!(new_address, address);
        assert_eq!(registry.storage(record.got_id).unwrap(), &[0; 4]);
        assert_eq!(registry.quarantined().len(), 1);
        assert_eq!(registry.quarantined()[0].address(), address);
        assert_eq!(registry.quarantined()[0].storage(), &[0xFF; 4]);
        assert_eq!(registry.quarantined()[0].cause, RetiredCause::Reset);
        assert_eq!(registry.reserved_bytes(), 14);
    }

    #[test]
    fn failed_reset_keeps_the_old_root_active() {
        let limits = MemoryLimits {
            max_total_bytes: 8,
            max_allocation_bytes: 8,
            max_align: 8,
        };
        let mut registry = RootRegistry::with_limits(limits);
        let record = root("_ROOT::COUNTER", 1, 4, 4);
        registry.allocate(&record, 1).unwrap();
        let address = registry.address(record.got_id).unwrap();
        registry
            .get_mut(record.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&7_u32.to_le_bytes());

        let mut larger = record.clone();
        larger.size = 8;
        assert_eq!(
            registry.reset(&larger, 2).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
        assert_eq!(registry.address(record.got_id), Some(address));
        assert_eq!(
            registry.storage(record.got_id).unwrap(),
            &7_u32.to_le_bytes()
        );
        assert!(registry.quarantined().is_empty());
    }

    #[test]
    fn discard_frees_storage_for_a_failed_candidate() {
        let mut registry = RootRegistry::new();
        let preserved = root("_ROOT::A", 1, 4, 4);
        let new = root("_ROOT::B", 2, 4, 4);
        registry.allocate(&preserved, 1).unwrap();
        registry.allocate(&new, 2).unwrap();
        assert_eq!(registry.reserved_bytes(), 14);

        registry.discard(new.got_id).expect("discard");

        assert!(!registry.contains(new.got_id));
        assert!(registry.contains(preserved.got_id));
        assert!(registry.quarantined().is_empty());
        assert_eq!(registry.reserved_bytes(), 7);
        assert_eq!(
            registry.discard(new.got_id).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
    }

    #[test]
    fn rejects_direct_duplicate_and_malformed_roots() {
        let mut registry = RootRegistry::new();
        let mut direct = root("_ROOT::AT", 1, 4, 4);
        direct.root_kind = RootKind::Direct;
        assert_eq!(
            registry.allocate(&direct, 1).unwrap_err().code,
            codes::MODULE_INVALID
        );

        let mut bad_align = root("_ROOT::AT", 1, 4, 3);
        bad_align.root_kind = RootKind::Global;
        assert_eq!(
            registry.allocate(&bad_align, 1).unwrap_err().code,
            codes::MANIFEST_INVALID
        );

        let mut zero = root("_ROOT::ZERO", 2, 4, 4);
        zero.size = 0;
        assert_eq!(
            registry.allocate(&zero, 1).unwrap_err().code,
            codes::MANIFEST_INVALID
        );

        let record = root("_ROOT::COUNTER", 3, 4, 4);
        registry.allocate(&record, 1).unwrap();
        assert_eq!(
            registry.allocate(&record, 2).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
    }

    #[test]
    fn enforces_root_memory_limits() {
        let limits = MemoryLimits {
            max_total_bytes: 16,
            max_allocation_bytes: 8,
            max_align: 8,
        };
        let mut registry = RootRegistry::with_limits(limits);
        registry.allocate(&root("_ROOT::A", 1, 8, 8), 1).unwrap();
        assert_eq!(
            registry
                .allocate(&root("_ROOT::B", 2, 8, 8), 1)
                .unwrap_err()
                .code,
            codes::ONLINE_PREPARE
        );

        let mut registry = RootRegistry::with_limits(MemoryLimits {
            max_total_bytes: 64,
            max_allocation_bytes: 4,
            max_align: 8,
        });
        assert_eq!(
            registry
                .allocate(&root("_ROOT::A", 1, 8, 8), 1)
                .unwrap_err()
                .code,
            codes::ONLINE_PREPARE
        );

        let mut registry = RootRegistry::with_limits(MemoryLimits {
            max_total_bytes: 64,
            max_allocation_bytes: 8,
            max_align: 4,
        });
        assert_eq!(
            registry
                .allocate(&root("_ROOT::A", 1, 8, 8), 1)
                .unwrap_err()
                .code,
            codes::ONLINE_PREPARE
        );
    }

    #[test]
    fn alignment_padding_counts_towards_the_memory_budget() {
        let mut registry = RootRegistry::with_limits(MemoryLimits {
            max_total_bytes: 8,
            max_allocation_bytes: 1,
            max_align: 4096,
        });
        // A one-byte root aligned to 4096 reserves 4096 bytes, not one.
        assert_eq!(
            registry
                .allocate(&root("_ROOT::A", 1, 1, 4096), 1)
                .unwrap_err()
                .code,
            codes::ONLINE_PREPARE
        );
    }

    #[test]
    fn transient_allocations_are_zeroed_aligned_and_stable() {
        let mut allocator = TransientAllocator::new(1);
        let first = allocator.alloc(16, 8).expect("first");
        let second = allocator.alloc(3, 1).expect("second");
        assert_eq!(first % 8, 0);
        assert_ne!(first, second);
        assert!(allocator.owns(first));
        assert_eq!(allocator.len(), 2);
        assert_eq!(allocator.reserved_bytes(), 26);

        // A fresh allocation must not move or reuse live blocks.
        let third = allocator.alloc(32, 16).expect("third");
        assert!(allocator.owns(first));
        assert!(allocator.owns(second));
        assert!(allocator.owns(third));
        assert_eq!(third % 16, 0);
        assert!(allocator.overlaps(first, 16));

        allocator.free(first).expect("free first");
        assert!(!allocator.owns(first));
        assert!(allocator.owns(second));
        assert_eq!(allocator.reserved_bytes(), 50);
    }

    #[test]
    fn zero_size_transient_allocation_is_unique_and_freeable() {
        let mut allocator = TransientAllocator::new(1);
        let first = allocator.alloc(0, 1).expect("zero");
        let second = allocator.alloc(0, 1).expect("zero again");
        assert_ne!(first, second);
        allocator.free(first).expect("free");
        allocator.free(second).expect("free");
        assert!(allocator.is_empty());
    }

    #[test]
    fn rejects_unknown_double_null_and_misaligned_frees() {
        let mut allocator = TransientAllocator::new(1);
        let block = allocator.alloc(8, 8).expect("block");
        allocator.free(block).expect("free block");
        assert_eq!(
            allocator.free(block).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
        assert_eq!(allocator.free(0).unwrap_err().code, codes::ONLINE_PREPARE);
        assert_eq!(
            allocator.free(block + 1).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
        assert_eq!(
            allocator.alloc(8, 3).unwrap_err().code,
            codes::MANIFEST_INVALID
        );
    }

    #[test]
    fn free_all_reclaims_every_block() {
        let mut allocator = TransientAllocator::new(1);
        let first = allocator.alloc(8, 8).expect("first");
        let second = allocator.alloc(16, 8).expect("second");
        assert_eq!(allocator.free_all(), 38);
        assert!(allocator.is_empty());
        assert!(!allocator.owns(first));
        assert!(!allocator.owns(second));
        assert_eq!(allocator.reserved_bytes(), 0);
        allocator.alloc(8, 8).expect("allocation after reclaim");
    }

    #[test]
    fn enforces_transient_memory_limits() {
        let mut allocator = TransientAllocator::with_limits(
            1,
            MemoryLimits {
                max_total_bytes: 16,
                max_allocation_bytes: 16,
                max_align: 8,
            },
        );
        allocator.alloc(8, 8).expect("first");
        assert_eq!(
            allocator.alloc(16, 8).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
        assert_eq!(
            allocator.alloc(8, 16).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
        assert_eq!(
            allocator.alloc(u64::MAX, 1).unwrap_err().code,
            codes::ONLINE_PREPARE
        );

        let mut padded = TransientAllocator::with_limits(
            1,
            MemoryLimits {
                max_total_bytes: 8,
                max_allocation_bytes: 1,
                max_align: 4096,
            },
        );
        assert_eq!(
            padded.alloc(1, 4096).unwrap_err().code,
            codes::ONLINE_PREPARE
        );
    }

    #[test]
    fn root_and_transient_storage_do_not_overlap() {
        let mut registry = RootRegistry::new();
        let mut allocator = TransientAllocator::new(1);
        let record = root("_ROOT::COUNTER", 1, 64, 16);
        registry.allocate(&record, 1).unwrap();
        let root_address = registry.address(record.got_id).unwrap();
        let transient = allocator.alloc(64, 16).unwrap();
        assert!(!allocator.overlaps(root_address, 64));
        assert!(!allocator.overlaps(transient + 64, 1));
        assert!(allocator.overlaps(transient, 64));
    }
}
