use anyhow::Context as _;
use anyhow::Result;
use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use rand::seq::IndexedRandom as _;
use solana_sdk::{signature::Keypair, transaction::VersionedTransaction};
use solana_tls_utils::{new_dummy_x509_certificate, SkipServerVerification};
use std::time::Instant;
use std::{
    net::{SocketAddr, ToSocketAddrs as _},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::common::SolanaRpcClient;
use crate::swqos::common::poll_transaction_confirmation;
use crate::swqos::serialization::serialize_transaction_bincode_sync;
use crate::swqos::SwqosClientTrait;
use crate::{
    constants::swqos::SPEEDLANDING_TIP_ACCOUNTS,
    swqos::{SwqosType, TradeType},
};

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
/// Fallback SNI when endpoint is IP or cannot extract host (keeps legacy behavior).
const SPEED_SERVER_FALLBACK: &str = "speed-landing";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SpeedlandingClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    endpoint: Endpoint,
    client_config: ClientConfig,
    addr: SocketAddr,
    /// TLS SNI: host from endpoint URL so server presents the right cert (e.g. nyc.speedlanding.trade).
    server_name: String,
    connection: ArcSwap<Connection>,
    reconnect: Mutex<()>,
}

impl SpeedlandingClient {
    /// Extract TLS SNI (host) from endpoint URL. Uses fallback "speed-landing" for IP or when host cannot be determined.
    fn server_name_from_endpoint(endpoint: &str) -> String {
        let without_scheme = endpoint
            .strip_prefix("https://")
            .or_else(|| endpoint.strip_prefix("http://"))
            .unwrap_or(endpoint);
        let host = without_scheme.split(':').next().unwrap_or("").trim();
        if host.is_empty() {
            return SPEED_SERVER_FALLBACK.to_string();
        }
        if !host.chars().any(|c| c.is_ascii_alphabetic()) {
            return SPEED_SERVER_FALLBACK.to_string();
        }
        host.to_string()
    }

    pub async fn new(rpc_url: String, endpoint_string: String, api_key: String) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let server_name = Self::server_name_from_endpoint(&endpoint_string);
        let keypair = Keypair::from_base58_string(&api_key);
        let (cert, key) = new_dummy_x509_certificate(&keypair);
        let mut crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_client_auth_cert(vec![cert], key)
            .context("failed to configure client certificate")?;

        crypto.alpn_protocols = vec![ALPN_TPU_PROTOCOL_ID.to_vec()];

        let client_crypto = QuicClientConfig::try_from(crypto)
            .context("failed to convert rustls config into quinn crypto config")?;
        let mut client_config = ClientConfig::new(Arc::new(client_crypto));
        let mut transport = TransportConfig::default();
        transport.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
        transport.max_idle_timeout(Some(IdleTimeout::try_from(MAX_IDLE_TIMEOUT)?));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
        endpoint.set_default_client_config(client_config.clone());
        let addr = endpoint_string
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("Address not resolved"))?;
        let connecting = endpoint.connect(addr, &server_name)?;
        let connection = timeout(CONNECT_TIMEOUT, connecting)
            .await
            .context("Speedlanding QUIC connect timeout")?
            .context("Speedlanding QUIC handshake failed")?;

        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            endpoint,
            client_config,
            addr,
            server_name,
            connection: ArcSwap::from_pointee(connection),
            reconnect: Mutex::new(()),
        })
    }

    /// Ensure we have a live connection: if current one is closed, reconnect under lock so
    /// concurrent senders wait and then all use the new connection. Uses blocking lock so
    /// waiters get the updated connection.
    async fn ensure_connected(&self) -> Result<Arc<Connection>> {
        let guard = self.reconnect.lock().await;
        let current = self.connection.load_full();
        if current.close_reason().is_none() {
            return Ok(current);
        }
        drop(guard);
        let _guard = self.reconnect.lock().await;
        let current = self.connection.load_full();
        if current.close_reason().is_some() {
            let connecting = self
                .endpoint
                .connect_with(self.client_config.clone(), self.addr, self.server_name.as_str())?;
            let connection = timeout(CONNECT_TIMEOUT, connecting)
                .await
                .context("Speedlanding QUIC reconnect timeout")?
                .context("Speedlanding QUIC re-handshake failed")?;
            self.connection.store(Arc::new(connection));
            return Ok(self.connection.load_full());
        }
        Ok(current)
    }

    async fn try_send_bytes(connection: &Connection, payload: &[u8]) -> Result<()> {
        let mut stream = connection.open_uni().await?;
        stream.write_all(payload).await?;
        stream.finish()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl SwqosClientTrait for SpeedlandingClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let (buf_guard, signature) = serialize_transaction_bincode_sync(transaction)?;
        let connection = self.ensure_connected().await?;
        let mut send_result = timeout(SEND_TIMEOUT, Self::try_send_bytes(&connection, &*buf_guard)).await;
        let need_retry = match &send_result {
            Ok(Ok(())) => false,
            Ok(Err(_)) | Err(_) => true,
        };
        if need_retry {
            if crate::common::sdk_log::sdk_log_enabled() {
                eprintln!(" [speedlanding] {} send failed or timeout, reconnecting", trade_type);
            }
            let connection = self.ensure_connected().await?;
            send_result = timeout(SEND_TIMEOUT, Self::try_send_bytes(&connection, &*buf_guard)).await;
        }
        send_result
            .context("Speedlanding QUIC send timeout")??;
        match poll_transaction_confirmation(&self.rpc_client, signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(" [speedlanding] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                }
                return Err(e);
            }
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(" [speedlanding] {} confirmed: {:?}", trade_type, start_time.elapsed());
        }
        Ok(())
    }

    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
        wait_confirmation: bool,
    ) -> Result<()> {
        for transaction in transactions {
            self.send_transaction(trade_type, transaction, wait_confirmation).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *SPEEDLANDING_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| SPEEDLANDING_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Speedlanding
    }
}
