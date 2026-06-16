use anyhow::Context as _;
use anyhow::Result;
use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use rand::seq::IndexedRandom as _;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::signer::Signer;
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
use crate::swqos::SwqosClientTrait;
use crate::{
    constants::swqos::SOLAMI_TIP_ACCOUNTS,
    swqos::{SwqosType, TradeType},
};

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
const SOLAMI_SERVER: &str = "solami-beam";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SolamiClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    endpoint: Endpoint,
    client_config: ClientConfig,
    addr: SocketAddr,
    connection: ArcSwap<Connection>,
    reconnect: Mutex<()>,
}

impl SolamiClient {
    pub async fn new(rpc_url: String, endpoint_string: String, api_key: String) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let keypair_bytes = bs58::decode(api_key.trim())
            .into_vec()
            .map_err(|e| anyhow::anyhow!("Solami api_token base58 decode failed: {}", e))?;
        let keypair = Keypair::try_from(keypair_bytes.as_slice()).map_err(|e| {
            anyhow::anyhow!("Solami api_token is not a valid Solana keypair: {}", e)
        })?;
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
        let connecting = endpoint.connect(addr, SOLAMI_SERVER)?;
        let connection = timeout(CONNECT_TIMEOUT, connecting)
            .await
            .context("Solami QUIC connect timeout")?
            .with_context(|| {
                format!(
                    "Solami QUIC handshake failed (verify wallet pubkey {} is registered and UDP {} is reachable)",
                    keypair.pubkey(),
                    endpoint_string
                )
            })?;

        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            endpoint,
            client_config,
            addr,
            connection: ArcSwap::from_pointee(connection),
            reconnect: Mutex::new(()),
        })
    }

    async fn ensure_connected(&self) -> Result<Arc<Connection>> {
        let current = self.connection.load_full();
        if current.close_reason().is_none() {
            return Ok(current);
        }
        let _guard = self.reconnect.lock().await;
        let current = self.connection.load_full();
        if current.close_reason().is_some() {
            let connecting =
                self.endpoint.connect_with(self.client_config.clone(), self.addr, SOLAMI_SERVER)?;
            let connection = timeout(CONNECT_TIMEOUT, connecting)
                .await
                .context("Solami QUIC reconnect timeout")?
                .with_context(|| {
                    format!(
                        "Solami QUIC re-handshake failed (peer {} SNI {})",
                        self.addr, SOLAMI_SERVER
                    )
                })?;
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
impl SwqosClientTrait for SolamiClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let signature = transaction.get_signature();
        let serialized_tx = bincode::serialize(transaction)?;
        let connection = self.ensure_connected().await?;
        let mut send_result =
            timeout(SEND_TIMEOUT, Self::try_send_bytes(&connection, &serialized_tx)).await;
        let need_retry = matches!(&send_result, Ok(Err(_)) | Err(_));
        if need_retry {
            if crate::common::sdk_log::sdk_log_enabled() {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "Solami",
                    trade_type,
                    start_time.elapsed(),
                    "reconnecting",
                );
            }
            let connection = self.ensure_connected().await?;
            send_result =
                timeout(SEND_TIMEOUT, Self::try_send_bytes(&connection, &serialized_tx)).await;
        }
        match send_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    crate::common::sdk_log::log_swqos_submission_failed(
                        "Solami",
                        trade_type,
                        start_time.elapsed(),
                        &e,
                    );
                }
                return Err(e);
            }
            Err(_) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    crate::common::sdk_log::log_swqos_submission_failed(
                        "Solami",
                        trade_type,
                        start_time.elapsed(),
                        "timeout",
                    );
                }
                anyhow::bail!("Solami QUIC send timeout");
            }
        }
        if crate::common::sdk_log::sdk_log_enabled() {
            crate::common::sdk_log::log_swqos_submitted("Solami", trade_type, start_time.elapsed());
        }
        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, *signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [{:width$}] {} confirmation failed: {:?}",
                        "Solami",
                        trade_type,
                        start_time.elapsed(),
                        width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
                    );
                }
                return Err(e);
            }
        }
        if wait_confirmation && crate::common::sdk_log::sdk_log_enabled() {
            println!(" signature: {:?}", signature);
            println!(
                " [{:width$}] {} confirmed: {:?}",
                "Solami",
                trade_type,
                start_time.elapsed(),
                width = crate::common::sdk_log::SWQOS_LABEL_WIDTH
            );
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
        let tip_account = *SOLAMI_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| SOLAMI_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Solami
    }
}
