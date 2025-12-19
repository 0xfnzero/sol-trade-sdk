use anyhow::Context as _;
use anyhow::Result;
use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use rand::seq::IndexedRandom as _;
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::{signature::Keypair, transaction::VersionedTransaction};
use solana_tls_utils::{new_dummy_x509_certificate, SkipServerVerification};
use std::time::Instant;
use std::{
    net::{SocketAddr, ToSocketAddrs as _},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

use crate::common::SolanaRpcClient;
use crate::swqos::common::poll_transaction_confirmation;
use crate::swqos::SwqosClientTrait;
use crate::{
    constants::swqos::SOYAS_TIP_ACCOUNTS,
    swqos::{SwqosType, TradeType},
};

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
const SOYAS_SERVER: &str = "soyas-landing";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(25);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub struct SoyasClient {
    pub rpc_client: Arc<SolanaRpcClient>,
    endpoint: Endpoint,
    client_config: ClientConfig,
    addr: SocketAddr,
    connection: ArcSwap<Connection>,
    reconnect: Mutex<()>,
}

impl SoyasClient {
    pub async fn new(rpc_url: String, endpoint_string: String, api_key: String) -> Result<Self> {
        let rpc_client = SolanaRpcClient::new(rpc_url);
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
        let connection = endpoint.connect(addr, SOYAS_SERVER)?.await?;

        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            endpoint,
            client_config,
            addr,
            connection: ArcSwap::from_pointee(connection),
            reconnect: Mutex::new(()),
        })
    }

    async fn reconnect(&self) -> anyhow::Result<()> {
        let _guard = self.reconnect.try_lock()?;
        let connection = self
            .endpoint
            .connect_with(self.client_config.clone(), self.addr, SOYAS_SERVER)?
            .await?;
        self.connection.store(Arc::new(connection));
        Ok(())
    }

    async fn try_send_bytes(connection: &Connection, payload: &[u8]) -> anyhow::Result<()> {
        let mut stream = connection.open_uni().await?;
        stream.write_all(payload).await?;
        stream.finish()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl SwqosClientTrait for SoyasClient {
    async fn send_transaction(
        &self,
        trade_type: TradeType,
        transaction: &VersionedTransaction,
    ) -> Result<()> {
        let signature = transaction.get_signature();
        let serialized_tx = bincode::serialize(transaction)?;
        let connection = self.connection.load_full();
        if Self::try_send_bytes(&connection, &serialized_tx).await.is_ok() {
            return Ok(());
        }
        eprintln!(" [soyas] failed to send transaction; reconnecting");
        self.reconnect().await?;
        let connection = self.connection.load_full();
        Self::try_send_bytes(&connection, &serialized_tx).await?;

        let start_time: Instant = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, *signature).await {
            Ok(_) => (),
            Err(e) => {
                println!(" signature: {:?}", signature);
                println!(" [soyas] {} confirmation failed: {:?}", trade_type, start_time.elapsed());
                return Err(e);
            }
        }
        println!(" signature: {:?}", signature);
        println!(" [soyas] {} confirmed: {:?}", trade_type, start_time.elapsed());
        Ok(())
    }

    async fn send_transactions(
        &self,
        trade_type: TradeType,
        transactions: &Vec<VersionedTransaction>,
    ) -> Result<()> {
        for transaction in transactions {
            self.send_transaction(trade_type, transaction).await?;
        }
        Ok(())
    }

    fn get_tip_account(&self) -> Result<String> {
        let tip_account = *SOYAS_TIP_ACCOUNTS
            .choose(&mut rand::rng())
            .or_else(|| SOYAS_TIP_ACCOUNTS.first())
            .unwrap();
        Ok(tip_account.to_string())
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Soyas
    }
}
