//! TLS and credential material for the `mqtt` I/O backend (WL-15 phase 4).
//!
//! TLS uses the approved `rustls` stack with a user-supplied CA file. There is
//! no bundled, system, or web trust store: a device either configures a CA
//! file and verifies the broker certificate and server name, or explicitly
//! disables verification with `verify = false` and accepts the documented
//! warning code. Client certificates and private keys are optional and use the
//! same user-supplied material.
//!
//! Credentials are loaded once at cold start. A user name is UTF-8 text and a
//! password is opaque bytes; both files may end in CR/LF, which is trimmed.
//! Empty or unreadable material fails closed before any socket is opened. No
//! function in this module puts credential or key bytes into an error, a
//! `Debug` rendering, a diagnostic, or a log message.

use std::fmt;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{self, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use stc_got::error::Error;

use super::config::{self, CredentialsPlan, MqttDevicePlan, TlsPlan};

/// One prepared TLS client configuration and its verification mode.
pub struct PreparedTls {
    /// Client configuration shared by every reconnect of the device.
    pub config: Arc<ClientConfig>,
    /// Server name sent in SNI and used for certificate verification.
    pub server_name: ServerName<'static>,
    /// Whether the broker certificate and name are verified.
    pub verify: bool,
}

impl fmt::Debug for PreparedTls {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTls")
            .field("server_name", &self.server_name)
            .field("verify", &self.verify)
            .finish_non_exhaustive()
    }
}

/// Credentials loaded from private files.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    username: Option<String>,
    password: Option<Vec<u8>>,
}

impl Credentials {
    /// Builds credentials from already-loaded values.
    #[cfg(test)]
    pub(crate) fn new(username: Option<String>, password: Option<Vec<u8>>) -> Self {
        Self { username, password }
    }

    /// Returns the user name, when configured.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns the password bytes, when configured.
    #[must_use]
    pub fn password(&self) -> Option<&[u8]> {
        self.password.as_deref()
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Prepared, secret-safe security material for one data device.
#[derive(Debug)]
pub struct PreparedSecurity {
    /// TLS client configuration, when the device enables TLS.
    pub tls: Option<PreparedTls>,
    /// Credentials, when the device configures them.
    pub credentials: Option<Credentials>,
}

/// Loads and validates all TLS and credential material for one device.
///
/// This runs at cold start, before any session thread or socket exists, so
/// missing, malformed, or mismatched material fails closed with
/// [`config::codes::MQTT_TLS_MATERIAL`] or
/// [`config::codes::MQTT_CREDENTIAL_MATERIAL`].
pub fn prepare(plan: &MqttDevicePlan) -> Result<PreparedSecurity, Error> {
    let tls = plan
        .tls
        .as_ref()
        .map(|tls| prepare_tls(plan, tls))
        .transpose()?;
    let credentials = plan
        .credentials
        .as_ref()
        .map(|credentials| load_credentials(plan, credentials))
        .transpose()?;
    Ok(PreparedSecurity { tls, credentials })
}

fn prepare_tls(plan: &MqttDevicePlan, tls: &TlsPlan) -> Result<PreparedTls, Error> {
    let ca_bytes = read_material(plan, &tls.ca_file, "TLS CA file")?;
    let mut roots = RootCertStore::empty();
    for certificate in rustls_pemfile::certs(&mut Cursor::new(&ca_bytes)) {
        let certificate = certificate
            .map_err(|error| tls_error(plan, &tls.ca_file, format!("is not valid PEM: {error}")))?;
        roots.add(certificate).map_err(|error| {
            tls_error(
                plan,
                &tls.ca_file,
                format!("contains a certificate rustls rejected: {error}"),
            )
        })?;
    }
    if roots.is_empty() {
        return Err(tls_error(plan, &tls.ca_file, "contains no CA certificate"));
    }

    let name = tls.server_name.clone().unwrap_or_else(|| plan.host.clone());
    let server_name = ServerName::try_from(name.clone()).map_err(|_| {
        tls_error(
            plan,
            &tls.ca_file,
            format!("cannot use `{name}` as the TLS server name"),
        )
    })?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier_algorithms = provider.signature_verification_algorithms;
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| {
            tls_error(
                plan,
                &tls.ca_file,
                format!("cannot select TLS protocol versions: {error}"),
            )
        })?;
    let builder = if tls.verify {
        builder.with_root_certificates(roots)
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                algorithms: verifier_algorithms,
            }))
    };

    let config = match (&tls.client_cert_file, &tls.client_key_file) {
        (Some(cert_file), Some(key_file)) => {
            let chain = read_certificate_chain(plan, cert_file)?;
            let key = read_private_key(plan, key_file)?;
            builder.with_client_auth_cert(chain, key).map_err(|error| {
                tls_error(
                    plan,
                    cert_file,
                    format!("client certificate and key were rejected: {error}"),
                )
            })?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => {
            return Err(tls_error(
                plan,
                &tls.ca_file,
                "configures only one of client-cert-file and client-key-file",
            ));
        }
    };

    Ok(PreparedTls {
        config: Arc::new(config),
        server_name,
        verify: tls.verify,
    })
}

fn read_certificate_chain(
    plan: &MqttDevicePlan,
    path: &Path,
) -> Result<Vec<CertificateDer<'static>>, Error> {
    let bytes = read_material(plan, path, "TLS client certificate")?;
    let chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut Cursor::new(&bytes))
        .collect::<Result<_, _>>()
        .map_err(|error| {
            tls_error(
                plan,
                path,
                format!("is not a valid client certificate PEM: {error}"),
            )
        })?;
    if chain.is_empty() {
        return Err(tls_error(plan, path, "contains no client certificate"));
    }
    Ok(chain)
}

fn read_private_key(plan: &MqttDevicePlan, path: &Path) -> Result<PrivateKeyDer<'static>, Error> {
    let bytes = read_material(plan, path, "TLS client key")?;
    let key = rustls_pemfile::private_key(&mut Cursor::new(&bytes)).map_err(|error| {
        tls_error(
            plan,
            path,
            format!("is not a valid private-key PEM: {error}"),
        )
    })?;
    key.ok_or_else(|| tls_error(plan, path, "contains no private key"))
}

fn read_material(plan: &MqttDevicePlan, path: &Path, description: &str) -> Result<Vec<u8>, Error> {
    std::fs::read(path).map_err(|error| {
        Error::new(
            config::codes::MQTT_TLS_MATERIAL,
            format!(
                "MQTT device `{}` cannot read {description} `{}`: {error}",
                plan.name,
                path.display()
            ),
        )
    })
}

fn tls_error(plan: &MqttDevicePlan, path: &Path, message: impl fmt::Display) -> Error {
    Error::new(
        config::codes::MQTT_TLS_MATERIAL,
        format!(
            "MQTT device `{}` TLS material `{}` {message}",
            plan.name,
            path.display()
        ),
    )
}

fn load_credentials(
    plan: &MqttDevicePlan,
    credentials: &CredentialsPlan,
) -> Result<Credentials, Error> {
    let username = credentials
        .username_file
        .as_deref()
        .map(|path| load_username(plan, path))
        .transpose()?;
    let password = credentials
        .password_file
        .as_deref()
        .map(|path| load_password(plan, path))
        .transpose()?;
    Ok(Credentials { username, password })
}

fn load_username(plan: &MqttDevicePlan, path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path).map_err(|error| {
        credential_error(plan, path, format!("cannot read user name file: {error}"))
    })?;
    let bytes = trim_line_end(bytes);
    let username = String::from_utf8(bytes).map_err(|_| {
        credential_error(plan, path, "user name file is not valid UTF-8".to_owned())
    })?;
    if username.is_empty() {
        return Err(credential_error(
            plan,
            path,
            "user name file is empty".to_owned(),
        ));
    }
    Ok(username)
}

fn load_password(plan: &MqttDevicePlan, path: &Path) -> Result<Vec<u8>, Error> {
    let bytes = std::fs::read(path).map_err(|error| {
        credential_error(plan, path, format!("cannot read password file: {error}"))
    })?;
    Ok(trim_line_end(bytes))
}

fn trim_line_end(mut bytes: Vec<u8>) -> Vec<u8> {
    while bytes
        .last()
        .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
    {
        bytes.pop();
    }
    bytes
}

fn credential_error(plan: &MqttDevicePlan, path: &Path, message: impl fmt::Display) -> Error {
    Error::new(
        config::codes::MQTT_CREDENTIAL_MATERIAL,
        format!(
            "MQTT device `{}` credential material `{}`: {message}",
            plan.name,
            path.display()
        ),
    )
}

/// A verifier used only for an explicit `verify = false`.
///
/// It accepts any broker certificate and skips hostname checks, but still
/// verifies the handshake signature against the presented certificate so the
/// connection cannot be silently corrupted. Cold start emits
/// [`config::codes::MQTT_TLS_VERIFICATION_DISABLED`] for this mode.
#[derive(Debug)]
struct InsecureVerifier {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io_mqtt::config::{CredentialsPlan, LastWillPlan, MqttChannelPlan, TlsPlan};
    use crate::io_mqtt::session::SessionConfig;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct TempFile {
        path: PathBuf,
    }

    impl TempFile {
        fn new(name: &str, contents: impl AsRef<[u8]>) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "stc-mqtt-security-{}-{unique}-{name}",
                std::process::id()
            ));
            std::fs::write(&path, contents).expect("write temp material");
            Self { path }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn device(tls: Option<TlsPlan>, credentials: Option<CredentialsPlan>) -> MqttDevicePlan {
        MqttDevicePlan {
            name: "broker0".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 8883,
            tls,
            client_id: None,
            session: SessionConfig::new(
                Duration::from_secs(30),
                Duration::from_secs(1),
                vec![Duration::from_millis(100)],
            )
            .expect("session config"),
            clean_start: true,
            session_expiry: 0,
            stale_timeout: Duration::from_secs(1),
            credentials,
            last_will: None::<LastWillPlan>,
            channels: Vec::<MqttChannelPlan>::new(),
            unbound: Vec::new(),
        }
    }

    #[test]
    fn credentials_trim_line_endings_and_keep_the_password_bytes() {
        let username = TempFile::new("user", "operator\r\n");
        let password = TempFile::new("password", b"p\0a\xffss\r\n");
        let plan = device(
            None,
            Some(CredentialsPlan {
                username_file: Some(username.path.clone()),
                password_file: Some(password.path.clone()),
            }),
        );
        let prepared = prepare(&plan).expect("credentials load");
        let credentials = prepared.credentials.expect("credentials");
        assert_eq!(credentials.username(), Some("operator"));
        assert_eq!(credentials.password(), Some(b"p\0a\xffss".as_slice()));
    }

    #[test]
    fn credential_debug_never_renders_secret_bytes() {
        let credentials = Credentials {
            username: Some("operator".to_owned()),
            password: Some(b"hunter2".to_vec()),
        };
        let rendered = format!("{credentials:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("operator"));
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn an_empty_user_name_fails_closed_without_opening_a_socket() {
        let username = TempFile::new("empty-user", "\r\n");
        let plan = device(
            None,
            Some(CredentialsPlan {
                username_file: Some(username.path.clone()),
                password_file: None,
            }),
        );
        let error = prepare(&plan).expect_err("empty user name");
        assert_eq!(error.code, config::codes::MQTT_CREDENTIAL_MATERIAL);
    }

    #[test]
    fn missing_credential_file_reports_the_code_not_a_secret() {
        let plan = device(
            None,
            Some(CredentialsPlan {
                username_file: Some(PathBuf::from("does-not-exist")),
                password_file: None,
            }),
        );
        let error = prepare(&plan).expect_err("missing user name file");
        assert_eq!(error.code, config::codes::MQTT_CREDENTIAL_MATERIAL);
        assert!(error.message.contains("does-not-exist"));
    }

    #[test]
    fn invalid_ca_file_fails_closed() {
        let ca = TempFile::new("invalid-ca.pem", "not a certificate");
        let plan = device(
            Some(TlsPlan {
                ca_file: ca.path.clone(),
                server_name: None,
                verify: true,
                client_cert_file: None,
                client_key_file: None,
            }),
            None,
        );
        let error = prepare(&plan).expect_err("invalid CA");
        assert_eq!(error.code, config::codes::MQTT_TLS_MATERIAL);
        assert!(error.message.contains("no CA certificate"));
    }

    #[test]
    fn an_empty_ca_file_fails_closed() {
        let ca = TempFile::new("empty-ca.pem", "");
        let plan = device(
            Some(TlsPlan {
                ca_file: ca.path.clone(),
                server_name: None,
                verify: true,
                client_cert_file: None,
                client_key_file: None,
            }),
            None,
        );
        let error = prepare(&plan).expect_err("empty CA");
        assert_eq!(error.code, config::codes::MQTT_TLS_MATERIAL);
    }

    /// Path of a committed test-only PEM fixture.
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mqtt")
            .join(name)
    }

    fn tls_plan(ca: &str, verify: bool, cert: Option<&str>, key: Option<&str>) -> TlsPlan {
        TlsPlan {
            ca_file: fixture(ca),
            server_name: Some("localhost".to_owned()),
            verify,
            client_cert_file: cert.map(fixture),
            client_key_file: key.map(fixture),
        }
    }

    #[test]
    fn invalid_server_name_fails_closed() {
        let plan = device(
            Some(TlsPlan {
                ca_file: fixture("ca.pem"),
                server_name: Some("not a dns name!".to_owned()),
                verify: true,
                client_cert_file: None,
                client_key_file: None,
            }),
            None,
        );
        let error = prepare(&plan).expect_err("invalid server name");
        assert_eq!(error.code, config::codes::MQTT_TLS_MATERIAL);
        assert!(error.message.contains("server name"));
    }

    #[test]
    fn verified_tls_prepares_with_the_supplied_ca() {
        let prepared = prepare(&device(Some(tls_plan("ca.pem", true, None, None)), None))
            .expect("verified TLS material");
        assert!(prepared.tls.expect("TLS configuration").verify);
    }

    #[test]
    fn verification_can_only_be_disabled_explicitly() {
        let prepared = prepare(&device(Some(tls_plan("ca.pem", false, None, None)), None))
            .expect("insecure TLS material");
        assert!(!prepared.tls.expect("TLS configuration").verify);
    }

    #[test]
    fn a_matching_client_certificate_and_key_prepare() {
        let prepared = prepare(&device(
            Some(tls_plan(
                "ca.pem",
                true,
                Some("client.pem"),
                Some("client-key.pem"),
            )),
            None,
        ))
        .expect("client certificate material");
        assert!(prepared.tls.is_some());
    }

    #[test]
    fn a_mismatched_client_key_fails_closed_without_leaking_the_key() {
        let plan = device(
            Some(tls_plan(
                "ca.pem",
                true,
                Some("client.pem"),
                Some("mismatched-key.pem"),
            )),
            None,
        );
        let error = prepare(&plan).expect_err("mismatched client key");
        assert_eq!(error.code, config::codes::MQTT_TLS_MATERIAL);
        assert!(!error.message.contains("BEGIN PRIVATE KEY"));
        assert!(!error.message.contains("PRIVATE KEY"));
    }

    #[test]
    fn a_client_certificate_without_a_key_fails_closed() {
        let plan = device(
            Some(tls_plan("ca.pem", true, Some("client.pem"), None)),
            None,
        );
        let error = prepare(&plan).expect_err("missing client key");
        assert_eq!(error.code, config::codes::MQTT_TLS_MATERIAL);
    }

    #[test]
    fn a_plain_device_prepares_no_tls_and_no_credentials() {
        let prepared = prepare(&device(None, None)).expect("plain device");
        assert!(prepared.tls.is_none());
        assert!(prepared.credentials.is_none());
    }

    #[test]
    fn insecure_verifier_accepts_any_name_and_signature_schemes() {
        let verifier = InsecureVerifier {
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        };
        assert!(!verifier.supported_verify_schemes().is_empty());
        assert!(!verifier.requires_raw_public_keys());
    }
}
