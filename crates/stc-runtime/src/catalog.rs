//! Host-side descriptor and manifest validation.
//!
//! GOT Artifacts §11.4 pairs a module descriptor with its manifest, and §9.5
//! requires a runtime to validate every field and digest before attaching.
//! This module adds the host policy layer on top of the shared codecs in
//! `stc-got`: the host profile, target, endianness, pointer width, ABI range,
//! and the feature bits the host implements.
//!
//! Manifest data is untrusted. Validation never executes artifact text, never
//! follows a path taken from an artifact, and never allocates from an unchecked
//! count.

use stc_got::error::{Error, Result, codes};
use stc_got::layout::{Endianness, Target};
use stc_got::{Descriptor, Digest32, HOST_ABI_MAJOR, HOST_ABI_MINOR, MODULE_FORMAT, Manifest};

/// The only language profile M4.5 accepts.
pub const PROFILE: &str = "stc-2013-oop-utf8-v1";

/// Feature bits implemented by an M4.5 host.
pub const SUPPORTED_FEATURES: u64 =
    stc_got::descriptor::feature::HOST_ABI | stc_got::descriptor::feature::ONLINE_PLAN;

/// The host's profile, target, ABI range, and implemented feature bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPlatform {
    /// Accepted profile identifier.
    pub profile: String,
    /// Host target.
    pub target: Target,
    /// Host ABI major version.
    pub abi_major: u16,
    /// Host ABI minor version.
    pub abi_minor: u16,
    /// Feature bits the host implements.
    pub supported_features: u64,
}

impl HostPlatform {
    /// Describes the target this runtime was built for.
    ///
    /// # Errors
    ///
    /// Rejects hosts other than the supported x64 Windows MSVC and Linux GNU
    /// targets.
    pub fn current() -> Result<Self> {
        let triple = if cfg!(all(windows, target_env = "msvc")) {
            "x86_64-pc-windows-msvc"
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "x86_64-unknown-linux-gnu"
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            "aarch64-unknown-linux-gnu"
        } else {
            return Err(Error::new(
                codes::MODULE_TARGET,
                "the runtime host target is not supported",
            ));
        };
        Ok(Self::new(
            PROFILE,
            Target::new(triple, Endianness::Little, 8),
            HOST_ABI_MAJOR,
            HOST_ABI_MINOR,
        ))
    }

    /// Creates a host description with the M4.5 feature set.
    #[must_use]
    pub fn new(profile: impl Into<String>, target: Target, abi_major: u16, abi_minor: u16) -> Self {
        Self {
            profile: profile.into(),
            target,
            abi_major,
            abi_minor,
            supported_features: SUPPORTED_FEATURES,
        }
    }

    /// Replaces the implemented feature bits.
    #[must_use]
    pub fn with_supported_features(mut self, features: u64) -> Self {
        self.supported_features = features;
        self
    }

    /// Validates a manifest against host policy.
    ///
    /// # Errors
    ///
    /// Rejects a profile, target, endianness, pointer width, module format, ABI,
    /// or feature-bit mismatch.
    pub fn validate_manifest(&self, manifest: &Manifest) -> Result<()> {
        check_platform(
            self,
            &manifest.header.profile,
            &manifest.header.target_triple,
            manifest.header.endianness,
            manifest.header.pointer_width,
            manifest.header.module_format,
            manifest.header.host_abi_major,
            manifest.header.host_abi_minor,
            manifest.header.required_features,
            "manifest",
        )
    }

    /// Validates a descriptor/manifest pair against host policy.
    ///
    /// The descriptor and manifest are also checked against each other by
    /// [`Descriptor::validate_against_manifest`].
    ///
    /// # Errors
    ///
    /// Rejects either half's host policy violation or any pairing mismatch.
    pub fn validate_pair(&self, descriptor: &Descriptor, manifest: &Manifest) -> Result<()> {
        check_platform(
            self,
            &descriptor.profile,
            &descriptor.target_triple,
            descriptor.endianness,
            descriptor.pointer_width,
            descriptor.module_format,
            descriptor.host_abi_major,
            descriptor.host_abi_minor,
            descriptor.required_features,
            "descriptor",
        )?;
        self.validate_manifest(manifest)?;
        descriptor.validate_against_manifest(manifest)
    }
}

/// A descriptor and manifest that passed host validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPair {
    /// The validated descriptor.
    pub descriptor: Descriptor,
    /// The validated manifest.
    pub manifest: Manifest,
    /// Descriptor digest computed from the received bytes.
    pub descriptor_digest: Digest32,
    /// Manifest digest computed from the received bytes.
    pub manifest_digest: Digest32,
}

impl ValidatedPair {
    /// Parses and validates a descriptor blob and a binary manifest.
    ///
    /// # Errors
    ///
    /// Rejects malformed, tampered, unpaired, or host-incompatible artifacts.
    pub fn from_bytes(
        platform: &HostPlatform,
        descriptor_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<Self> {
        let manifest = Manifest::decode_binary(manifest_bytes)?;
        let descriptor = Descriptor::decode(descriptor_bytes)?;
        Self::from_parts(platform, descriptor, manifest)
    }

    /// Validates already-decoded artifacts as a pair.
    ///
    /// # Errors
    ///
    /// Rejects a host policy violation or a descriptor/manifest mismatch.
    pub fn from_parts(
        platform: &HostPlatform,
        descriptor: Descriptor,
        manifest: Manifest,
    ) -> Result<Self> {
        platform.validate_pair(&descriptor, &manifest)?;
        let descriptor_digest = descriptor.digest()?;
        let manifest_digest = manifest.digest()?;
        Ok(Self {
            descriptor,
            manifest,
            descriptor_digest,
            manifest_digest,
        })
    }
}

/// Parses and validates a binary manifest without a module descriptor.
///
/// This is the path used for a baseline or downloaded manifest.
///
/// # Errors
///
/// Rejects a malformed or host-incompatible manifest.
pub fn validate_manifest_binary(platform: &HostPlatform, bytes: &[u8]) -> Result<Manifest> {
    let manifest = Manifest::decode_binary(bytes)?;
    platform.validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Parses and validates a canonical JSON manifest without a module descriptor.
///
/// # Errors
///
/// Rejects malformed JSON, a digest mismatch, or a host-incompatible manifest.
pub fn validate_manifest_json(platform: &HostPlatform, text: &str) -> Result<Manifest> {
    let manifest = Manifest::decode_json(text)?;
    platform.validate_manifest(&manifest)?;
    Ok(manifest)
}

#[allow(clippy::too_many_arguments)]
fn check_platform(
    platform: &HostPlatform,
    profile: &str,
    target_triple: &str,
    endianness: u8,
    pointer_width: u8,
    module_format: u16,
    abi_major: u16,
    abi_minor: u16,
    required_features: u64,
    what: &str,
) -> Result<()> {
    if profile != platform.profile {
        return Err(Error::new(
            codes::MODULE_TARGET,
            format!(
                "{what} profile `{profile}` does not match the host profile `{}`",
                platform.profile
            ),
        ));
    }
    if target_triple != platform.target.triple {
        return Err(Error::new(
            codes::MODULE_TARGET,
            format!(
                "{what} target `{target_triple}` does not match the host target `{}`",
                platform.target.triple
            ),
        ));
    }
    if endianness != platform.target.endianness.tag() {
        return Err(Error::new(
            codes::MODULE_TARGET,
            format!("{what} byte order does not match the host target"),
        ));
    }
    if pointer_width != platform.target.pointer_width {
        return Err(Error::new(
            codes::MODULE_TARGET,
            format!(
                "{what} pointer width {pointer_width} does not match the host width {}",
                platform.target.pointer_width
            ),
        ));
    }
    if module_format != MODULE_FORMAT {
        return Err(Error::new(
            codes::MODULE_INVALID,
            format!("{what} module format {module_format} is not {MODULE_FORMAT}"),
        ));
    }
    if abi_major != platform.abi_major {
        return Err(Error::new(
            codes::ABI_MISMATCH,
            format!(
                "{what} requires host ABI {abi_major}.{abi_minor}; the host provides {}.{}",
                platform.abi_major, platform.abi_minor
            ),
        ));
    }
    if abi_minor > platform.abi_minor {
        return Err(Error::new(
            codes::ABI_MISMATCH,
            format!(
                "{what} requires host ABI minor {abi_minor}; the host provides {}",
                platform.abi_minor
            ),
        ));
    }
    let missing = required_features & !platform.supported_features;
    if missing != 0 {
        return Err(Error::new(
            codes::MODULE_INVALID,
            format!(
                "{what} requires unsupported feature bits {missing:#x} (host supports {:#x})",
                platform.supported_features
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::{Digest32, Endianness, Target};

    use crate::test_support::{self, artifacts, reseal_pair};

    fn platform() -> HostPlatform {
        HostPlatform::new(
            PROFILE,
            Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8),
            HOST_ABI_MAJOR,
            HOST_ABI_MINOR,
        )
    }

    #[test]
    fn current_host_platform_is_a_supported_target() {
        let platform = HostPlatform::current().expect("supported host");
        assert_eq!(platform.profile, PROFILE);
        assert_eq!(platform.target.pointer_width, 8);
        assert_eq!(platform.abi_major, HOST_ABI_MAJOR);
        assert_eq!(platform.abi_minor, HOST_ABI_MINOR);
        assert_eq!(platform.supported_features, SUPPORTED_FEATURES);
    }

    #[test]
    fn accepts_a_valid_descriptor_and_manifest_pair() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let pair = ValidatedPair::from_bytes(
            &platform(),
            &artifacts.descriptor_bytes,
            &artifacts.manifest_binary,
        )
        .expect("valid pair");
        assert_eq!(pair.descriptor, artifacts.descriptor);
        assert_eq!(pair.manifest, artifacts.manifest);
        assert_eq!(pair.descriptor_digest, artifacts.descriptor_digest);
        assert_eq!(pair.manifest_digest, artifacts.manifest_digest);
    }

    #[test]
    fn rejects_tampered_descriptor_and_manifest_bytes() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let mut descriptor = artifacts.descriptor_bytes.clone();
        descriptor[0] ^= 0xFF;
        let error = ValidatedPair::from_bytes(&platform(), &descriptor, &artifacts.manifest_binary)
            .unwrap_err();
        assert_eq!(error.code, codes::MODULE_INVALID);

        let mut manifest = artifacts.manifest_binary.clone();
        let last = manifest.len() - 1;
        manifest[last] ^= 0xFF;
        let error = ValidatedPair::from_bytes(&platform(), &artifacts.descriptor_bytes, &manifest)
            .unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_INVALID);
    }

    #[test]
    fn rejects_cross_paired_artifacts() {
        let first = artifacts(test_support::SIMPLE, "first");
        let second = artifacts(test_support::SIMPLE_ALTERNATE, "second");
        let error = ValidatedPair::from_bytes(
            &platform(),
            &first.descriptor_bytes,
            &second.manifest_binary,
        )
        .unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PAIRING);
    }

    #[test]
    fn rejects_wrong_profile_target_and_abi() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let bytes = (&artifacts.descriptor_bytes, &artifacts.manifest_binary);

        let wrong_profile = HostPlatform {
            profile: "other-profile".into(),
            ..platform()
        };
        assert_eq!(
            ValidatedPair::from_bytes(&wrong_profile, bytes.0, bytes.1)
                .unwrap_err()
                .code,
            codes::MODULE_TARGET
        );

        let wrong_target = HostPlatform::new(
            PROFILE,
            Target::new("aarch64-unknown-linux-gnu", Endianness::Little, 8),
            HOST_ABI_MAJOR,
            HOST_ABI_MINOR,
        );
        assert_eq!(
            ValidatedPair::from_bytes(&wrong_target, bytes.0, bytes.1)
                .unwrap_err()
                .code,
            codes::MODULE_TARGET
        );

        let wrong_major = HostPlatform::new(
            PROFILE,
            platform().target,
            HOST_ABI_MAJOR + 1,
            HOST_ABI_MINOR,
        );
        assert_eq!(
            ValidatedPair::from_bytes(&wrong_major, bytes.0, bytes.1)
                .unwrap_err()
                .code,
            codes::ABI_MISMATCH
        );
    }

    #[test]
    fn rejects_a_module_minor_newer_than_the_host() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let mut descriptor = artifacts.descriptor.clone();
        let mut manifest = artifacts.manifest.clone();
        descriptor.host_abi_minor = HOST_ABI_MINOR + 1;
        manifest.header.host_abi_minor = HOST_ABI_MINOR + 1;
        let (descriptor_bytes, manifest_bytes) =
            reseal_pair(&mut descriptor, &mut manifest).expect("reseal");

        let error =
            ValidatedPair::from_bytes(&platform(), &descriptor_bytes, &manifest_bytes).unwrap_err();
        assert_eq!(error.code, codes::ABI_MISMATCH);

        let newer_host = HostPlatform::new(
            PROFILE,
            platform().target,
            HOST_ABI_MAJOR,
            HOST_ABI_MINOR + 1,
        );
        ValidatedPair::from_bytes(&newer_host, &descriptor_bytes, &manifest_bytes)
            .expect("a newer host accepts the module");
    }

    #[test]
    fn rejects_feature_bits_the_host_lacks() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let mut descriptor = artifacts.descriptor.clone();
        let mut manifest = artifacts.manifest.clone();
        descriptor.required_features |= stc_got::descriptor::feature::DEBUG_HOOKS;
        manifest.header.required_features = descriptor.required_features;
        let (descriptor_bytes, manifest_bytes) =
            reseal_pair(&mut descriptor, &mut manifest).expect("reseal");

        let error =
            ValidatedPair::from_bytes(&platform(), &descriptor_bytes, &manifest_bytes).unwrap_err();
        assert_eq!(error.code, codes::MODULE_INVALID);

        let debug_host = platform().with_supported_features(
            SUPPORTED_FEATURES | stc_got::descriptor::feature::DEBUG_HOOKS,
        );
        ValidatedPair::from_bytes(&debug_host, &descriptor_bytes, &manifest_bytes)
            .expect("a host implementing the feature accepts the module");
    }

    #[test]
    fn parses_binary_and_json_manifests_identically() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let binary =
            validate_manifest_binary(&platform(), &artifacts.manifest_binary).expect("binary");
        let json = validate_manifest_json(&platform(), &artifacts.manifest_json).expect("json");
        assert_eq!(binary, json);
    }

    #[test]
    fn rejects_tampered_json_manifests() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let marker = "\"digest\": \"0x";
        let index = artifacts.manifest_json.find(marker).expect("digest") + marker.len();
        let mut text = artifacts.manifest_json.clone();
        let digit = text.as_bytes()[index];
        text.replace_range(index..index + 1, if digit == b'0' { "1" } else { "0" });
        let error = validate_manifest_json(&platform(), &text).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_INVALID);
    }

    #[test]
    fn rejects_a_manifest_with_a_stale_descriptor_digest() {
        let artifacts = artifacts(test_support::SIMPLE, "simple");
        let mut descriptor = artifacts.descriptor.clone();
        let mut manifest = artifacts.manifest.clone();
        manifest.header.descriptor_digest = Digest32::from_bytes([0x88; 32]);
        descriptor.manifest_digest = manifest.digest().expect("manifest digest");
        let error = ValidatedPair::from_parts(&platform(), descriptor, manifest).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PAIRING);
    }
}
