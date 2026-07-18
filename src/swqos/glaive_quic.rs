//! Glaive's persistent QUIC submission transport.
//!
//! Protocol reference: <https://glaive.trade/docs#send-quic>

use anyhow::{Context as _, Result};
use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use solana_sdk::signature::Keypair;
use solana_tls_utils::{new_dummy_x509_certificate, SkipServerVerification};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs as _},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::Mutex, time::timeout};
use uuid::Uuid;

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
const GLAIVE_QUIC_SNI: &str = "glaive-intake";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MEV_PROTECT_BIT: u8 = 1 << 0;
pub const MAX_TRANSACTION_SIZE: usize = 1232;

pub struct GlaiveQuicClient {
    endpoint: Endpoint,
    client_config: ClientConfig,
    server_addr: SocketAddr,
    auth_frame: [u8; 17],
    connection: ArcSwap<Connection>,
    reconnect: Mutex<()>,
}

impl GlaiveQuicClient {
    pub async fn connect(
        endpoint_addr: &str,
        api_key_uuid: &str,
        mev_protection: bool,
    ) -> Result<Self> {
        let auth_frame = build_auth_frame(api_key_uuid, mev_protection)?;
        let client_config = build_client_config()?;
        let server_addr = resolve_endpoint(endpoint_addr)?;

        let mut endpoint = Endpoint::client(local_bind_addr(server_addr))
            .context("create Glaive QUIC endpoint")?;
        endpoint.set_default_client_config(client_config.clone());

        let connection = connect_once(&endpoint, &client_config, server_addr).await?;
        authenticate(&connection, &auth_frame).await?;

        Ok(Self {
            endpoint,
            client_config,
            server_addr,
            auth_frame,
            connection: ArcSwap::from_pointee(connection),
            reconnect: Mutex::new(()),
        })
    }

    pub async fn send_transaction(&self, transaction_bytes: &[u8]) -> Result<()> {
        validate_transaction_size(transaction_bytes)?;

        let stale = self.connection.load_full();
        match send_once(&stale, transaction_bytes).await {
            Ok(()) => Ok(()),
            Err(first_error) => {
                let connection = self.reconnect_if_stale(&stale).await.with_context(|| {
                    format!("Glaive QUIC reconnect after send failure: {first_error}")
                })?;
                send_once(&connection, transaction_bytes)
                    .await
                    .context("Glaive QUIC send failed after reconnect")
            }
        }
    }

    async fn reconnect_if_stale(&self, stale: &Arc<Connection>) -> Result<Arc<Connection>> {
        let _guard = self.reconnect.lock().await;
        let current = self.connection.load_full();
        if !Arc::ptr_eq(&current, stale) && current.close_reason().is_none() {
            return Ok(current);
        }

        let connection =
            connect_once(&self.endpoint, &self.client_config, self.server_addr).await?;
        authenticate(&connection, &self.auth_frame).await?;
        self.connection.store(Arc::new(connection));
        Ok(self.connection.load_full())
    }
}

impl Drop for GlaiveQuicClient {
    fn drop(&mut self) {
        self.connection.load_full().close(0u32.into(), b"client closing");
    }
}

async fn connect_once(
    endpoint: &Endpoint,
    client_config: &ClientConfig,
    server_addr: SocketAddr,
) -> Result<Connection> {
    let connecting = endpoint
        .connect_with(client_config.clone(), server_addr, GLAIVE_QUIC_SNI)
        .context("start Glaive QUIC handshake")?;
    timeout(CONNECT_TIMEOUT, connecting)
        .await
        .context("Glaive QUIC connect timeout")?
        .context("Glaive QUIC handshake failed")
}

async fn authenticate(connection: &Connection, auth_frame: &[u8; 17]) -> Result<()> {
    timeout(AUTH_TIMEOUT, async {
        let mut stream = connection.open_uni().await.context("open Glaive auth stream")?;
        stream.write_all(auth_frame).await.context("write Glaive auth frame")?;
        stream.finish().context("finish Glaive auth stream")?;
        Result::<()>::Ok(())
    })
    .await
    .context("Glaive QUIC auth timeout")??;
    Ok(())
}

async fn send_once(connection: &Connection, transaction_bytes: &[u8]) -> Result<()> {
    timeout(SEND_TIMEOUT, async {
        let mut stream = connection.open_uni().await.context("open Glaive transaction stream")?;
        stream.write_all(transaction_bytes).await.context("write Glaive transaction")?;
        stream.finish().context("finish Glaive transaction stream")?;
        Result::<()>::Ok(())
    })
    .await
    .context("Glaive QUIC send timeout")??;
    Ok(())
}

fn build_auth_frame(api_key_uuid: &str, mev_protection: bool) -> Result<[u8; 17]> {
    let api_key =
        Uuid::parse_str(api_key_uuid.trim()).context("Glaive API key must be a valid UUID v4")?;
    if api_key.get_version_num() != 4 {
        anyhow::bail!("Glaive API key must be a UUID v4");
    }

    let mut frame = [0u8; 17];
    frame[..16].copy_from_slice(&api_key.as_u128().to_be_bytes());
    frame[16] = if mev_protection { MEV_PROTECT_BIT } else { 0 };
    Ok(frame)
}

fn validate_transaction_size(transaction_bytes: &[u8]) -> Result<()> {
    if transaction_bytes.len() > MAX_TRANSACTION_SIZE {
        anyhow::bail!(
            "Glaive QUIC transaction too large: {} bytes (max {})",
            transaction_bytes.len(),
            MAX_TRANSACTION_SIZE
        );
    }
    Ok(())
}

fn build_client_config() -> Result<ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let keypair = Keypair::new();
    let (certificate, private_key) = new_dummy_x509_certificate(&keypair);

    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_client_auth_cert(vec![certificate], private_key)
        .context("configure Glaive QUIC client certificate")?;
    crypto.alpn_protocols = vec![ALPN_TPU_PROTOCOL_ID.to_vec()];

    let quic_crypto = QuicClientConfig::try_from(crypto)
        .context("convert Glaive rustls config to QUIC config")?;
    let mut client_config = ClientConfig::new(Arc::new(quic_crypto));
    let mut transport = TransportConfig::default();
    transport.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
    transport.max_idle_timeout(Some(IdleTimeout::try_from(MAX_IDLE_TIMEOUT)?));
    client_config.transport_config(Arc::new(transport));
    Ok(client_config)
}

fn resolve_endpoint(endpoint_addr: &str) -> Result<SocketAddr> {
    let mut candidates: Vec<_> = endpoint_addr
        .to_socket_addrs()
        .with_context(|| format!("resolve Glaive QUIC endpoint {endpoint_addr}"))?
        .collect();
    candidates.sort_by_key(|addr| if addr.is_ipv4() { 0 } else { 1 });
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("Glaive QUIC endpoint resolved to no addresses: {endpoint_addr}"))
}

fn local_bind_addr(remote: SocketAddr) -> SocketAddr {
    match remote.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_UUID: &str = "00112233-4455-4677-8899-aabbccddeeff";

    #[test]
    fn auth_frame_is_big_endian_uuid_plus_flags() {
        let frame = build_auth_frame(TEST_UUID, true).unwrap();
        assert_eq!(
            &frame[..16],
            &[
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x46, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
        assert_eq!(frame[16], MEV_PROTECT_BIT);

        let unprotected = build_auth_frame(TEST_UUID, false).unwrap();
        assert_eq!(unprotected[16], 0);
    }

    #[test]
    fn auth_frame_rejects_non_v4_or_malformed_keys() {
        assert!(build_auth_frame("not-a-uuid", false).is_err());
        assert!(build_auth_frame("00112233-4455-1677-8899-aabbccddeeff", false).is_err());
    }

    #[test]
    fn transaction_size_matches_solana_packet_limit() {
        assert!(validate_transaction_size(&vec![0; MAX_TRANSACTION_SIZE]).is_ok());
        assert!(validate_transaction_size(&vec![0; MAX_TRANSACTION_SIZE + 1]).is_err());
    }
}
