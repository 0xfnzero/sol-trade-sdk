//! Node1 QUIC SWQOS client.
//!
//! Protocol: first bi stream = auth (16-byte UUID); each transaction uses a new bi stream.
//! Request body = bincode(VersionedTransaction); response = 2 bytes status (BE) + 4 bytes msg_len (BE) + msg.
//! Reuses a single authenticated connection; reconnects and re-auth when connection is closed.

use anyhow::{Context, Result};
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Connection, Endpoint, IdleTimeout, RecvStream, TransportConfig};
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use uuid::Uuid;

use crate::common::SolanaRpcClient;
use crate::constants::swqos::NODE1_TIP_ACCOUNTS;
use crate::swqos::common::poll_transaction_confirmation;
use crate::swqos::{SwqosClientTrait, SwqosType, TradeType};
use rand::seq::IndexedRandom;
use solana_sdk::transaction::VersionedTransaction;
use std::time::Instant;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TX_SIZE: usize = 1232;

/// Node1 QUIC client: one authenticated connection, reuse for all transactions.
pub struct Node1QuicClient {
    endpoint: Endpoint,
    connection: Mutex<Connection>,
    server_addr: String,
    server_name: String,
    api_key_uuid: [u8; 16],
    rpc_client: Arc<SolanaRpcClient>,
}

impl Node1QuicClient {
    /// Connect and authenticate. Reuse the returned client for all subsequent sends.
    pub async fn connect(server_addr: &str, api_key: &str, rpc_url: String) -> Result<Self> {
        let socket_addr = server_addr
            .to_socket_addrs()
            .context("resolve Node1 QUIC server address")?
            .next()
            .context("no socket address for Node1 QUIC")?;

        let api_key_uuid =
            Uuid::parse_str(api_key).context("Node1 API key must be a valid UUID")?;
        let api_key_bytes: [u8; 16] = *api_key_uuid.as_bytes();

        let server_name = server_addr.split(':').next().unwrap_or(server_addr);

        let client_config = Self::build_client_config()?;
        let mut endpoint =
            Endpoint::client("0.0.0.0:0".parse()?).context("create QUIC endpoint")?;
        endpoint.set_default_client_config(client_config);

        let connecting =
            endpoint.connect(socket_addr, server_name).context("Node1 QUIC connect failed")?;
        let connection = timeout(CONNECT_TIMEOUT, connecting)
            .await
            .context("Node1 QUIC connect timeout")?
            .context("Node1 QUIC handshake failed")?;

        timeout(AUTH_TIMEOUT, Self::authenticate(&connection, &api_key_bytes))
            .await
            .context("Node1 QUIC auth timeout")??;

        Ok(Self {
            endpoint,
            connection: Mutex::new(connection),
            server_addr: server_addr.to_string(),
            server_name: server_name.to_string(),
            api_key_uuid: api_key_bytes,
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url)),
        })
    }

    fn build_client_config() -> Result<ClientConfig> {
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();

        let client_crypto = QuicClientConfig::try_from(crypto).context("build QUIC TLS config")?;
        let mut client_config = ClientConfig::new(Arc::new(client_crypto));

        let mut transport = TransportConfig::default();
        transport.max_idle_timeout(Some(IdleTimeout::try_from(MAX_IDLE_TIMEOUT).unwrap()));
        transport.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
        client_config.transport_config(Arc::new(transport));

        Ok(client_config)
    }

    async fn authenticate(connection: &Connection, api_key_bytes: &[u8; 16]) -> Result<()> {
        let (mut send, mut recv) = connection.open_bi().await.context("open_bi for auth")?;
        send.write_all(api_key_bytes).await.context("write auth bytes")?;
        send.finish().context("finish auth stream")?;

        let mut reply = [0u8; 1];
        recv.read_exact(&mut reply).await.context("read auth reply")?;
        match reply[0] {
            0 => Ok(()),
            code => anyhow::bail!("Node1 QUIC auth rejected, reply={}", code),
        }
    }

    async fn ensure_connected(&self) -> Result<Connection> {
        let guard = self.connection.lock().await;
        if let Some(_reason) = guard.close_reason() {
            drop(guard);
            let socket_addr = self
                .server_addr
                .to_socket_addrs()
                .context("resolve Node1 QUIC server address")?
                .next()
                .context("no socket address")?;
            let connecting = self
                .endpoint
                .connect(socket_addr, &self.server_name)
                .context("Node1 QUIC reconnect failed")?;
            let connection = timeout(CONNECT_TIMEOUT, connecting)
                .await
                .context("Node1 QUIC reconnect timeout")?
                .context("Node1 QUIC re-handshake failed")?;

            timeout(AUTH_TIMEOUT, Self::authenticate(&connection, &self.api_key_uuid))
                .await
                .context("Node1 QUIC re-auth timeout")??;

            let mut g = self.connection.lock().await;
            *g = connection.clone();
            Ok(connection)
        } else {
            Ok(guard.clone())
        }
    }

    async fn read_response(recv: &mut RecvStream) -> Result<(u16, String)> {
        let mut header = [0u8; 6];
        recv.read_exact(&mut header)
            .await
            .map_err(|e| anyhow::anyhow!("read response header: {:?}", e))?;
        let status = u16::from_be_bytes(header[0..2].try_into().unwrap());
        let msg_len = u32::from_be_bytes(header[2..6].try_into().unwrap()) as usize;
        let mut msg = vec![0u8; msg_len];
        if msg_len > 0 {
            recv.read_exact(&mut msg)
                .await
                .map_err(|e| anyhow::anyhow!("read response body: {:?}", e))?;
        }
        Ok((status, String::from_utf8_lossy(&msg).into_owned()))
    }

    /// Send one transaction over QUIC (opens new bi stream, writes bincode tx, reads status+msg).
    pub async fn send_transaction_bytes(&self, tx_bytes: &[u8]) -> Result<(u16, String)> {
        if tx_bytes.len() > MAX_TX_SIZE {
            anyhow::bail!(
                "Node1 QUIC: transaction too large ({} > {})",
                tx_bytes.len(),
                MAX_TX_SIZE
            );
        }

        let conn = self.ensure_connected().await?;
        let (mut send, mut recv) = conn.open_bi().await.context("open_bi for tx")?;
        send.write_all(tx_bytes).await.context("write tx")?;
        send.finish().context("finish tx stream")?;
        Self::read_response(&mut recv).await
    }
}

#[async_trait::async_trait]
impl SwqosClientTrait for Node1QuicClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start = Instant::now();
        let signature = transaction.signatures.first().copied().unwrap_or_default();
        let tx_bytes = bincode::serialize(transaction).context("Node1 QUIC: bincode serialize")?;

        let (status, msg) = timeout(SEND_TIMEOUT, self.send_transaction_bytes(&tx_bytes))
            .await
            .context("Node1 QUIC send timeout")??;

        if status != 200 {
            if crate::common::sdk_log::sdk_log_enabled() {
                eprintln!(
                    " [node1-quic] {} submit failed: status={} msg={}",
                    trade_type, status, msg
                );
            }
            anyhow::bail!("Node1 QUIC submit failed: status={} msg={}", status, msg);
        }

        if crate::common::sdk_log::sdk_log_enabled() {
            println!(" [node1-quic] {} submitted: {:?}", trade_type, start.elapsed());
        }

        let start = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => {
                if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
                    println!(" [node1-quic] {} confirmed: {:?}", trade_type, start.elapsed());
                }
                Ok(())
            }
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    eprintln!(
                        " [node1-quic] {} confirmation failed: {:?}",
                        trade_type,
                        start.elapsed()
                    );
                }
                Err(e)
            }
        }
    }

    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        for tx in transactions {
            self.send_transaction(trade_type, tx, wait_confirmation).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip = *NODE1_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| NODE1_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Node1
    }
}

impl Drop for Node1QuicClient {
    fn drop(&mut self) {
        self.connection.get_mut().close(0u32.into(), b"client closing");
    }
}

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
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
