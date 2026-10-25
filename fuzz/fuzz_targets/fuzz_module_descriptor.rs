#![no_main]

//! Fuzz the module descriptor and process-image metadata parsers.
//!
//! Descriptor blobs are recognized by their magic. Other inputs are also
//! wrapped in a valid descriptor header (declared size recomputed) so arbitrary
//! bytes reach the record decoders, and every input is tried as a UTF-8
//! metadata sidecar. Accepted descriptors must re-encode and round-trip with a
//! stable digest. The target never maps, loads, or executes native module code.

use libfuzzer_sys::fuzz_target;
use stc_got::{DESCRIPTOR_FORMAT, DESCRIPTOR_MAGIC, Descriptor};
use stc_runtime::ModuleInfo;

fn check(descriptor: Descriptor) {
    let digest = descriptor.digest().expect("a decoded descriptor digests");
    let encoded = descriptor
        .encode()
        .expect("a decoded descriptor re-encodes");
    let decoded = Descriptor::decode(&encoded).expect("descriptor bytes round-trip");
    assert_eq!(decoded, descriptor);
    assert_eq!(
        decoded.digest().expect("round-tripped descriptor digests"),
        digest
    );
}

fn wrap_descriptor(body: &[u8]) -> Option<Vec<u8>> {
    let total = 16_usize.checked_add(body.len())?;
    let size = u32::try_from(total).ok()?;
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&DESCRIPTOR_MAGIC);
    bytes.extend_from_slice(&DESCRIPTOR_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(body);
    Some(bytes)
}

fuzz_target!(|data: &[u8]| {
    let descriptor_input = data.starts_with(&DESCRIPTOR_MAGIC);
    if let Ok(descriptor) = Descriptor::decode(data) {
        check(descriptor);
    } else if !descriptor_input
        && let Some(wrapped) = wrap_descriptor(data)
        && let Ok(descriptor) = Descriptor::decode(&wrapped)
    {
        check(descriptor);
    }

    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(info) = ModuleInfo::parse(text)
    {
        let layout = info.image_layout();
        let _ = layout.total_bytes();
        let _ = layout.area(1);
        for binding in &info.bindings {
            let _ = info.binding(binding.got_id);
        }
    }
});
