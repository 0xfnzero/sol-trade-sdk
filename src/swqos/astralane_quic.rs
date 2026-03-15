//! 内联自 [Astralane/astralane-quic-client](https://github.com/Astralane/astralane-quic-client)，
//! 用于向 Astralane QUIC TPU 提交交易，不依赖外部 crate，便于审计与安全可控。

use anyhow::{Context, Result};
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Connection, Endpoint, IdleTimeout, TransportConfig};
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// ALPN protocol identifier for Astralane TPU.
const ALPN_ASTRALANE_TPU: &[u8] = b"astralane-tpu";

/// Maximum Solana transaction size.
pub const MAX_TRANSACTION_SIZE: usize = 1232;

/// QUIC application error codes returned by the server.
pub mod error_code {
    pub const OK: u32 = 0;
    pub const UNKNOWN_API_KEY: u32 = 1;
    pub const CONNECTION_LIMIT: u32 = 2;

    pub fn describe(code: u32) -> &'static str {
        match code {
            OK => "OK",
            UNKNOWN_API_KEY => "Unknown API key",
            CONNECTION_LIMIT => "Connection limit exceeded",
            _ => "Unknown error",
        }
    }
}

/// QUIC client for sending transactions to Astralane's TPU endpoint.
pub struct AstralaneQuicClient {
    endpoint: Endpoint,
    connection: Mutex<Connection>,
    server_addr: SocketAddr,
    #[allow(dead_code)]
    api_key: String,
}

impl AstralaneQuicClient {
    /// Connect to an Astralane QUIC server.
    /// Generates a self-signed TLS certificate with the API key as the Common Name (CN).
    pub async fn connect(server_addr: &str, api_key: &str) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let addr = SocketAddr::from_str(server_addr)
            .or_else(|_| {
                use std::net::ToSocketAddrs;
                server_addr
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addrs| addrs.next())
                    .ok_or_else(|| anyhow::anyhow!("Cannot resolve address: {}", server_addr))
            })
            .context("Invalid server address")?;

        info!("[astralane-quic] Building TLS config (CN = api_key)");
        let client_config = Self::build_client_config(api_key)?;

        let mut endpoint =
            Endpoint::client("0.0.0.0:0".parse()?).context("Failed to create QUIC endpoint")?;
        endpoint.set_default_client_config(client_config);

        info!("[astralane-quic] Connecting to {} ...", addr);
        let connection = endpoint
            .connect(addr, "astralane")?
            .await
            .context("Failed to connect to Astralane QUIC server")?;

        info!("[astralane-quic] Connected at {}", addr);

        Ok(Self {
            endpoint,
            connection: Mutex::new(connection),
            server_addr: addr,
            api_key: api_key.to_string(),
        })
    }

    /// Send a single bincode-serialized `VersionedTransaction`.
    /// Fire-and-forget; automatically reconnects if the connection is dead.
    pub async fn send_transaction(&self, transaction_bytes: &[u8]) -> Result<()> {
        if transaction_bytes.len() > MAX_TRANSACTION_SIZE {
            anyhow::bail!(
                "Transaction too large: {} bytes (max {})",
                transaction_bytes.len(),
                MAX_TRANSACTION_SIZE
            );
        }

        let conn = {
            let mut guard = self.connection.lock().await;
            if let Some(reason) = guard.close_reason() {
                if let quinn::ConnectionError::ApplicationClosed(ref info) = reason {
                    let code = info.error_code.into_inner();
                    if code != error_code::OK as u64 {
                        anyhow::bail!(
                            "Server closed connection: {} (code {})",
                            error_code::describe(code as u32),
                            code
                        );
                    }
                }
                warn!("[astralane-quic] Connection dead, reconnecting to {} ...", self.server_addr);
                *guard = self
                    .endpoint
                    .connect(self.server_addr, "astralane")?
                    .await
                    .context("Failed to reconnect to Astralane QUIC server")?;
                info!("[astralane-quic] Reconnected to {}", self.server_addr);
            }
            guard.clone()
        };

        let mut send_stream =
            conn.open_uni().await.context("Failed to open unidirectional stream")?;

        send_stream
            .write_all(transaction_bytes)
            .await
            .context("Failed to write transaction data")?;

        send_stream.finish().context("Failed to finish stream")?;
        info!("[astralane-quic] Transaction sent ({} bytes)", transaction_bytes.len());

        Ok(())
    }

    /// Reconnect to the server if the connection was closed.
    pub async fn reconnect(&self) -> Result<()> {
        let mut guard = self.connection.lock().await;
        if guard.close_reason().is_some() {
            info!("[astralane-quic] Reconnecting at {}", self.server_addr);
            *guard = self
                .endpoint
                .connect(self.server_addr, "astralane")?
                .await
                .context("Failed to reconnect to Astralane QUIC server")?;
            info!("[astralane-quic] Reconnected to {}", self.server_addr);
        }
        Ok(())
    }

    /// Check if the connection is still alive.
    pub async fn is_connected(&self) -> bool {
        self.connection.lock().await.close_reason().is_none()
    }

    /// Close the connection gracefully.
    pub async fn close(&self) {
        self.connection.lock().await.close(error_code::OK.into(), b"client closing");
    }

    fn build_client_config(api_key: &str) -> Result<ClientConfig> {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let mut cert_params = CertificateParams::new(vec![])?;
        cert_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, rcgen::DnValue::Utf8String(api_key.to_string()));
        let cert = cert_params.self_signed(&key_pair)?;

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

        let mut crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_client_auth_cert(vec![cert_der], key_der)
            .context("Failed to set client certificate")?;

        crypto.alpn_protocols = vec![ALPN_ASTRALANE_TPU.to_vec()];

        let mut transport = TransportConfig::default();
        transport.max_idle_timeout(Some(IdleTimeout::try_from(Duration::from_secs(30)).unwrap()));
        transport.keep_alive_interval(Some(Duration::from_secs(25)));

        let mut client_config =
            ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto).unwrap()));
        client_config.transport_config(Arc::new(transport));

        Ok(client_config)
    }
}

impl Drop for AstralaneQuicClient {
    fn drop(&mut self) {
        self.connection.get_mut().close(error_code::OK.into(), b"client closing");
    }
}

/// Skip server certificate verification (Astralane server may use self-signed cert).
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
