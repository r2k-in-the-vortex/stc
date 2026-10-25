//! Process-image reconfiguration and direct-variable remapping.
//!
//! Direct `AT` variables are per-generation cells that point into the host's
//! contiguous process image (Runtime §2.4). An online change may move a
//! variable, resize an area, or change the image layout, so at the cycle
//! boundary the host snapshots the old image, applies the candidate layout,
//! preserves the values of direct identities that exist in both generations,
//! and rebinds the candidate's direct cells (Online Change §6.2, §8.1).
//!
//! Values are preserved by `GotId`, not by raw byte address: a variable that
//! moves keeps its value, a removed variable's bytes are dropped, and a new
//! variable starts from the configured image content. Moved variables are
//! reported through `STC-W-ONLINE-IO-001`. If any step fails, the caller
//! restores the snapshot and the active generation keeps running unchanged.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::GotId;
use stc_got::error::{Error, Result, codes};

use crate::module_info::{DirectBinding, ImageLayout, ModuleInfo};

/// Largest process image the host configures from module metadata.
///
/// Metadata is untrusted, so the host refuses a layout it cannot allocate
/// rather than letting a malformed sidecar exhaust process memory.
pub const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;

/// A direct binding whose process-image location changed between generations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectRemap {
    /// Persistent identity of the direct variable.
    pub got_id: GotId,
    /// Binding in the active generation.
    pub previous: DirectBinding,
    /// Binding in the candidate generation.
    pub current: DirectBinding,
}

impl DirectRemap {
    /// Returns `true` when the area, offset, or width changed.
    #[must_use]
    pub fn moved(&self) -> bool {
        self.previous.area != self.current.area
            || self.previous.offset_bits != self.current.offset_bits
            || self.previous.width_bits != self.current.width_bits
    }
}

/// A captured process image used for rollback and value preservation.
pub(crate) struct ImageSnapshot {
    bytes: Vec<u8>,
    layout: ImageLayout,
}

/// Captures the current process image.
pub(crate) fn snapshot() -> ImageSnapshot {
    let (bytes, inputs_bytes, outputs_bytes, markers_bytes) = crate::image_snapshot();
    ImageSnapshot {
        bytes,
        layout: ImageLayout {
            inputs_bytes,
            outputs_bytes,
            markers_bytes,
        },
    }
}

/// Restores a captured process image.
///
/// Direct cells that point into the image must be rebound afterwards; the
/// restored buffer may have a different base address.
pub(crate) fn restore(snapshot: &ImageSnapshot) {
    crate::restore_image((
        snapshot.bytes.clone(),
        snapshot.layout.inputs_bytes,
        snapshot.layout.outputs_bytes,
        snapshot.layout.markers_bytes,
    ));
}

/// Validates an image layout and every direct binding against it.
///
/// # Errors
///
/// Rejects overflowing or oversized layouts, unknown areas, zero widths,
/// unaligned multi-byte bindings, duplicate identities, and ranges outside
/// their area.
pub(crate) fn validate_layout(layout: ImageLayout, bindings: &[DirectBinding]) -> Result<()> {
    let total = layout.total_bytes().ok_or_else(|| {
        Error::new(
            codes::ONLINE_IO,
            "process-image area sizes overflow the u64 range",
        )
    })?;
    if total > MAX_IMAGE_BYTES {
        return Err(Error::new(
            codes::ONLINE_IO,
            format!("process image of {total} bytes exceeds the host limit {MAX_IMAGE_BYTES}"),
        ));
    }
    if usize::try_from(total).is_err() {
        return Err(Error::new(
            codes::ONLINE_IO,
            format!("process image of {total} bytes exceeds the platform limit"),
        ));
    }
    let mut seen = BTreeSet::new();
    for binding in bindings {
        if !seen.insert(binding.got_id) {
            return Err(Error::new(
                codes::ONLINE_IO,
                format!(
                    "direct variable {} has more than one process-image binding",
                    binding.got_id
                ),
            ));
        }
        binding_byte_range(layout, binding)?;
    }
    Ok(())
}

/// Returns the absolute image byte offset and byte count of one binding.
fn binding_byte_range(layout: ImageLayout, binding: &DirectBinding) -> Result<(u64, u64)> {
    if binding.width_bits == 0 {
        return Err(Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} has a zero-width process-image binding",
                binding.got_id
            ),
        ));
    }
    let (base, length) = layout.area(binding.area).ok_or_else(|| {
        Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} uses unknown process-image area {}",
                binding.got_id, binding.area
            ),
        )
    })?;
    if binding.width_bits > 1 && !binding.offset_bits.is_multiple_of(8) {
        return Err(Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} starts at bit {} but occupies more than one bit",
                binding.got_id, binding.offset_bits
            ),
        ));
    }
    let byte = binding.offset_bits / 8;
    let bytes = binding.width_bits.div_ceil(8).max(1);
    let offset = base.checked_add(byte).ok_or_else(|| {
        Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} overflows the process-image offset",
                binding.got_id
            ),
        )
    })?;
    let end = offset.checked_add(bytes).ok_or_else(|| {
        Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} overflows the process-image range",
                binding.got_id
            ),
        )
    })?;
    if end > base.saturating_add(length) {
        return Err(Error::new(
            codes::ONLINE_IO,
            format!(
                "direct variable {} lies outside process-image area {}",
                binding.got_id, binding.area
            ),
        ));
    }
    Ok((offset, bytes))
}

/// Computes provisional direct-cell values for a candidate before activation.
///
/// The image is not reconfigured until the cycle boundary, so these addresses
/// are not dereferenced before the host rebinds them. They exist only to keep
/// every direct slot bound while the candidate is prepared.
pub(crate) fn planned_bindings(
    layout: ImageLayout,
    bindings: &[DirectBinding],
) -> Result<BTreeMap<GotId, usize>> {
    validate_layout(layout, bindings)?;
    let base = crate::image_base_address();
    let mut addresses = BTreeMap::new();
    for binding in bindings {
        let (offset, _) = binding_byte_range(layout, binding)?;
        let offset = usize::try_from(offset).map_err(|_| {
            Error::new(
                codes::ONLINE_IO,
                format!(
                    "direct variable {} exceeds the platform address range",
                    binding.got_id
                ),
            )
        })?;
        let address = base.checked_add(offset).ok_or_else(|| {
            Error::new(
                codes::ONLINE_IO,
                format!(
                    "direct variable {} overflows the image address",
                    binding.got_id
                ),
            )
        })?;
        addresses.insert(binding.got_id, address);
    }
    Ok(addresses)
}

/// Resolves every direct binding against the configured process image.
pub(crate) fn current_bindings(info: &ModuleInfo) -> Result<BTreeMap<GotId, usize>> {
    let layout = info.image_layout();
    validate_layout(layout, &info.bindings)?;
    let mut addresses = BTreeMap::new();
    for binding in &info.bindings {
        let byte = binding.offset_bits / 8;
        let bytes = binding.width_bits.div_ceil(8).max(1);
        let address = crate::image_area_address(binding.area, byte, bytes).ok_or_else(|| {
            Error::new(
                codes::ONLINE_IO,
                format!(
                    "direct variable {} has no address in the configured process image",
                    binding.got_id
                ),
            )
        })?;
        addresses.insert(binding.got_id, address);
    }
    Ok(addresses)
}

/// Returns the direct bindings that moved between two generations.
#[must_use]
pub(crate) fn remaps(previous: &ModuleInfo, current: &ModuleInfo) -> Vec<DirectRemap> {
    let previous_bindings: BTreeMap<GotId, &DirectBinding> = previous
        .bindings
        .iter()
        .map(|binding| (binding.got_id, binding))
        .collect();
    let mut remaps: Vec<DirectRemap> = current
        .bindings
        .iter()
        .filter_map(|binding| {
            let old = *previous_bindings.get(&binding.got_id)?;
            let remap = DirectRemap {
                got_id: binding.got_id,
                previous: old.clone(),
                current: binding.clone(),
            };
            remap.moved().then_some(remap)
        })
        .collect();
    remaps.sort_by_key(|remap| remap.got_id);
    remaps
}

/// Reconfigures the process image for a candidate layout.
///
/// Values of direct identities present in both generations are captured from
/// `previous` before the image is cleared and written back at their new
/// locations. Returns the candidate's direct-cell addresses.
pub(crate) fn reconfigure(
    previous: &ImageSnapshot,
    previous_info: Option<&ModuleInfo>,
    current: &ModuleInfo,
) -> Result<BTreeMap<GotId, usize>> {
    let layout = current.image_layout();
    validate_layout(layout, &current.bindings)?;
    let captured = capture_values(previous, previous_info, &current.bindings)?;
    crate::stc_rt_io_configure(
        layout.inputs_bytes,
        layout.outputs_bytes,
        layout.markers_bytes,
    );
    for binding in &current.bindings {
        if let Some(value) = captured.get(&binding.got_id) {
            apply_value(value, layout, binding)?;
        }
    }
    current_bindings(current)
}

/// Rebinds direct cells when the image layout is unchanged but bindings moved.
///
/// Moved values are captured first and written afterwards, so two variables
/// that exchange addresses do not overwrite each other.
pub(crate) fn remap_in_place(
    previous: &ImageSnapshot,
    previous_info: Option<&ModuleInfo>,
    current: &ModuleInfo,
) -> Result<BTreeMap<GotId, usize>> {
    let layout = current.image_layout();
    if layout != previous.layout {
        return Err(Error::new(
            codes::ONLINE_IO,
            "an in-place direct remap requires an unchanged process-image layout",
        ));
    }
    validate_layout(layout, &current.bindings)?;
    let previous_bindings: BTreeMap<GotId, &DirectBinding> = previous_info
        .map(|info| info.bindings.iter().map(|b| (b.got_id, b)).collect())
        .unwrap_or_default();
    let mut moved = Vec::new();
    for binding in &current.bindings {
        let Some(old) = previous_bindings.get(&binding.got_id) else {
            continue;
        };
        let remap = DirectRemap {
            got_id: binding.got_id,
            previous: (*old).clone(),
            current: binding.clone(),
        };
        if !remap.moved() {
            continue;
        }
        if let Some(value) = capture_value(&previous.bytes, previous.layout, old)? {
            moved.push((binding, value));
        }
    }
    for (binding, value) in moved {
        apply_value(&value, layout, binding)?;
    }
    current_bindings(current)
}

/// One direct-variable value captured from the old image.
enum Captured {
    /// A one-bit value; its new bit may differ from the old bit.
    Bit(bool),
    /// A byte string; a narrower new binding keeps the prefix.
    Bytes(Vec<u8>),
}

fn capture_values(
    previous: &ImageSnapshot,
    previous_info: Option<&ModuleInfo>,
    bindings: &[DirectBinding],
) -> Result<BTreeMap<GotId, Captured>> {
    let previous_bindings: BTreeMap<GotId, &DirectBinding> = previous_info
        .map(|info| info.bindings.iter().map(|b| (b.got_id, b)).collect())
        .unwrap_or_default();
    let mut captured = BTreeMap::new();
    for binding in bindings {
        let Some(old) = previous_bindings.get(&binding.got_id) else {
            continue;
        };
        if let Some(value) = capture_value(&previous.bytes, previous.layout, old)? {
            captured.insert(binding.got_id, value);
        }
    }
    Ok(captured)
}

fn capture_value(
    bytes: &[u8],
    layout: ImageLayout,
    binding: &DirectBinding,
) -> Result<Option<Captured>> {
    let (offset, count) = binding_byte_range(layout, binding)?;
    let start = usize::try_from(offset).map_err(|_| range_error(binding, offset))?;
    if binding.width_bits == 1 {
        let Some(byte) = bytes.get(start) else {
            return Err(range_error(binding, offset));
        };
        let bit = u8::try_from(binding.offset_bits % 8).unwrap_or(0);
        return Ok(Some(Captured::Bit(byte & (1 << bit) != 0)));
    }
    let count = usize::try_from(count).map_err(|_| range_error(binding, offset))?;
    let end = start
        .checked_add(count)
        .ok_or_else(|| range_error(binding, offset))?;
    let Some(value) = bytes.get(start..end) else {
        return Err(range_error(binding, offset));
    };
    Ok(Some(Captured::Bytes(value.to_vec())))
}

fn apply_value(value: &Captured, layout: ImageLayout, binding: &DirectBinding) -> Result<()> {
    let (offset, count) = binding_byte_range(layout, binding)?;
    let start = usize::try_from(offset).map_err(|_| range_error(binding, offset))?;
    match value {
        Captured::Bit(value) => {
            let bit = u8::try_from(binding.offset_bits % 8).unwrap_or(0);
            let written = crate::image_with(|image| {
                let Some(byte) = image.get_mut(start) else {
                    return false;
                };
                if *value {
                    *byte |= 1 << bit;
                } else {
                    *byte &= !(1 << bit);
                }
                true
            });
            if !written {
                return Err(range_error(binding, offset));
            }
        }
        Captured::Bytes(bytes) => {
            let count = usize::try_from(count)
                .unwrap_or(usize::MAX)
                .min(bytes.len());
            let end = start
                .checked_add(count)
                .ok_or_else(|| range_error(binding, offset))?;
            let written = crate::image_with(|image| {
                let Some(target) = image.get_mut(start..end) else {
                    return false;
                };
                target.copy_from_slice(&bytes[..count]);
                true
            });
            if !written {
                return Err(range_error(binding, offset));
            }
        }
    }
    Ok(())
}

fn range_error(binding: &DirectBinding, offset: u64) -> Error {
    Error::new(
        codes::ONLINE_IO,
        format!(
            "direct variable {} lies outside the captured process image at byte {offset}",
            binding.got_id
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(got_id: u8, area: u8, offset_bits: u64, width_bits: u64) -> DirectBinding {
        let mut bytes = [0_u8; 16];
        bytes[15] = got_id;
        DirectBinding {
            got_id: GotId::from_bytes(bytes),
            area,
            offset_bits,
            width_bits,
        }
    }

    fn info(layout: ImageLayout, bindings: Vec<DirectBinding>) -> ModuleInfo {
        ModuleInfo {
            inputs_bytes: layout.inputs_bytes,
            outputs_bytes: layout.outputs_bytes,
            markers_bytes: layout.markers_bytes,
            bindings,
            text: String::new(),
        }
    }

    #[test]
    fn layout_validation_accepts_aligned_bindings() {
        let layout = ImageLayout {
            inputs_bytes: 4,
            outputs_bytes: 0,
            markers_bytes: 4,
        };
        let bindings = vec![binding(1, 1, 16, 16), binding(2, 3, 0, 32)];
        validate_layout(layout, &bindings).expect("valid layout");
    }

    #[test]
    fn layout_validation_rejects_overflow_and_oversized_images() {
        let bindings = Vec::new();
        let overflow = ImageLayout {
            inputs_bytes: u64::MAX,
            outputs_bytes: 1,
            markers_bytes: 0,
        };
        assert_eq!(
            validate_layout(overflow, &bindings).unwrap_err().code,
            codes::ONLINE_IO
        );
        let oversized = ImageLayout {
            inputs_bytes: MAX_IMAGE_BYTES + 1,
            outputs_bytes: 0,
            markers_bytes: 0,
        };
        assert_eq!(
            validate_layout(oversized, &bindings).unwrap_err().code,
            codes::ONLINE_IO
        );
    }

    #[test]
    fn layout_validation_rejects_bad_bindings() {
        let layout = ImageLayout {
            inputs_bytes: 2,
            outputs_bytes: 0,
            markers_bytes: 4,
        };
        for bindings in [
            vec![binding(1, 1, 8, 16)],
            vec![binding(1, 1, 0, 32)],
            vec![binding(1, 9, 0, 8)],
            vec![binding(1, 1, 0, 0)],
            vec![binding(1, 1, 1, 16)],
            vec![binding(1, 1, 0, 8), binding(1, 1, 8, 8)],
        ] {
            let error = validate_layout(layout, &bindings).unwrap_err();
            assert_eq!(error.code, codes::ONLINE_IO, "{bindings:?}");
        }
    }

    #[test]
    fn direct_remaps_report_only_moved_bindings() {
        let previous = info(
            ImageLayout {
                inputs_bytes: 4,
                outputs_bytes: 0,
                markers_bytes: 4,
            },
            vec![binding(1, 1, 0, 16), binding(2, 3, 0, 32)],
        );
        let current = info(
            ImageLayout {
                inputs_bytes: 4,
                outputs_bytes: 0,
                markers_bytes: 4,
            },
            vec![binding(1, 1, 16, 16), binding(2, 3, 0, 32)],
        );
        let remaps = remaps(&previous, &current);
        assert_eq!(remaps.len(), 1);
        assert_eq!(remaps[0].got_id, binding(1, 1, 0, 16).got_id);
        assert!(remaps[0].moved());
    }
}
