//! HTTPS client that pins a self-signed peer by the SHA-256 fingerprint of its certificate
//! (`pinned-agent.js`).
//!
//! Proxmox and the UDM use self-signed certificates, so chain validation cannot work. Instead of switching
//! verification off, the chain check is replaced by an exact match on the peer certificate: the credentials
//! are never sent to anything else. Fails closed: without a fingerprint no client is built, unless the
//! operator explicitly asks for unverified TLS (`*_TLS_INSECURE=1`).

use std::sync::Arc;
use std::time::Duration;

use rustls::DigitallySignedStruct;
use rustls::SignatureScheme;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{self, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};

/// `aa:bb:..` or `AABB..` to lowercase hex without colons.
pub fn normalize_fingerprint(f: &str) -> String {
    f.replace(':', "").to_lowercase()
}

pub fn sha256_hex(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug)]
struct Pin {
    label: String,
    /// `None` means unverified TLS was asked for explicitly.
    want: Option<String>,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(want) = &self.want {
            let got = sha256_hex(end_entity.as_ref());
            if &got != want {
                return Err(rustls::Error::General(format!(
                    "{}: TLS fingerprint mismatch (got {})",
                    self.label,
                    if got.is_empty() { "none" } else { &got }
                )));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Build the client. `label` names the peer in messages ("proxmox", "unifi").
pub fn build_client(
    label: &str,
    fingerprint: &str,
    allow_insecure: bool,
) -> Result<reqwest::Client, String> {
    let want = normalize_fingerprint(fingerprint);
    let want = if want.is_empty() {
        if !allow_insecure {
            return Err(format!(
                "{label}: TLS cert fingerprint required \u{2014} configure the cert pin (or set the matching *_TLS_INSECURE=1 to explicitly allow unverified TLS). Refusing to send credentials over an unpinned connection."
            ));
        }
        eprintln!(
            "[{label}] *_TLS_INSECURE=1 \u{2014} TLS peer verification DISABLED; credentials are exposed to LAN MITM."
        );
        None
    } else {
        Some(want)
    };
    let provider: Arc<CryptoProvider> = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Pin {
        label: label.to_string(),
        want,
        algs: provider.signature_verification_algorithms,
    };
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("{label}: TLS setup: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    reqwest::Client::builder()
        .use_preconfigured_tls(cfg)
        .http1_only()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("{label}: {e}"))
}

/// The message of a transport failure: the original only has "fetch failed", the cause is added here.
pub fn transport_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = String::from("fetch failed");
    let mut cur: Option<&dyn Error> = e.source();
    while let Some(c) = cur {
        msg.push_str(": ");
        msg.push_str(&c.to_string());
        cur = c.source();
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_forms_are_equal() {
        assert_eq!(normalize_fingerprint("AA:bb:0C"), "aabb0c");
        assert_eq!(sha256_hex(b"abc").len(), 64);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn no_pin_fails_closed_unless_insecure_is_asked() {
        let e = build_client("proxmox", "", false).expect_err("must refuse");
        assert!(
            e.starts_with("proxmox: TLS cert fingerprint required"),
            "{e}"
        );
        assert!(build_client("proxmox", "", true).is_ok());
        assert!(build_client("proxmox", "AA:BB", false).is_ok());
    }
}
