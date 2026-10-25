#![no_main]

//! Fuzz the persistent GOT manifest codecs.
//!
//! Binary manifests are recognized by their magic; everything else is treated
//! as canonical JSON. Inputs that are not already binary are also wrapped in a
//! valid binary header (length and digest recomputed) so arbitrary bytes reach
//! the section and record decoders instead of stopping at the header checks.
//! A decoded manifest is re-projected through the other encoding and must
//! round-trip. The target never loads a module or executes generated code.

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use stc_got::manifest::MANIFEST_DOMAIN;
use stc_got::sha256::domain_hash;
use stc_got::{Digest32, MANIFEST_FORMAT, MANIFEST_MAGIC, Manifest};
use stc_runtime::HostPlatform;

fn decode(data: &[u8]) -> Option<Manifest> {
    if data.starts_with(&MANIFEST_MAGIC) {
        Manifest::decode_binary(data).ok()
    } else {
        std::str::from_utf8(data)
            .ok()
            .and_then(|text| Manifest::decode_json(text).ok())
    }
}

fn wrap_binary(body: &[u8]) -> Option<Vec<u8>> {
    let length = u64::try_from(body.len()).ok()?;
    let digest = Digest32::from_bytes(domain_hash(MANIFEST_DOMAIN, &[body]));
    let mut bytes = Vec::with_capacity(8 + 2 + 2 + 8 + 32 + body.len());
    bytes.extend_from_slice(&MANIFEST_MAGIC);
    bytes.extend_from_slice(&MANIFEST_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(body);
    Some(bytes)
}

fn host_platform() -> Option<&'static HostPlatform> {
    static PLATFORM: OnceLock<Option<HostPlatform>> = OnceLock::new();
    PLATFORM
        .get_or_init(|| HostPlatform::current().ok())
        .as_ref()
}

fn check(manifest: Manifest) -> Vec<u8> {
    // Decoding validates the format; the accepted path must stay usable.
    let digest = manifest.digest().expect("a decoded manifest digests");
    let _ = host_platform().map(|platform| platform.validate_manifest(&manifest));

    let json = manifest
        .to_json_text()
        .expect("a decoded manifest projects to canonical JSON");
    let from_json = Manifest::decode_json(&json).expect("canonical JSON round-trips");
    assert_eq!(from_json, manifest);
    assert_eq!(from_json.digest().expect("JSON projection digests"), digest);

    let binary = manifest
        .encode_binary()
        .expect("a decoded manifest encodes as binary");
    let from_binary = Manifest::decode_binary(&binary).expect("binary round-trips");
    assert_eq!(from_binary, manifest);
    binary
}

fuzz_target!(|data: &[u8]| {
    let binary_input = data.starts_with(&MANIFEST_MAGIC);
    if let Some(manifest) = decode(data) {
        let binary = check(manifest);
        if binary_input {
            // A binary input that decoded cleanly must re-encode byte-for-byte.
            assert_eq!(binary, data);
        }
    }

    if !binary_input
        && let Some(wrapped) = wrap_binary(data)
        && let Ok(manifest) = Manifest::decode_binary(&wrapped)
    {
        check(manifest);
    }
});
