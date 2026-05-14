use anyhow::Context as _;
use anyhow::Result;
use arc_swap::ArcSwap;
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use rand::seq::IndexedRandom as _;
use rcgen::{CertificateParams, KeyPair as RcgenKeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use solana_client::rpc_client::SerializableTransaction;
use solana_sdk::signer::Signer;
use solana_sdk::{signature::Keypair, transaction::VersionedTransaction};
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

// Skip server verification implementation
#[derive(Debug)]
struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// TLS 客户端证书：ECDSA P-256 + CN=钱包公钥（与 Speedlanding / Astralane QUIC 策略一致）。
fn generate_client_tls_credentials(
    keypair: &Keypair,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let tls_key = RcgenKeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let mut cert_params = CertificateParams::new(vec![])?;
    cert_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, rcgen::DnValue::Utf8String(keypair.pubkey().to_string()));
    let cert = cert_params.self_signed(&tls_key)?;
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(tls_key.serialize_der()));
    Ok((cert_der, key_der))
}

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
        let keypair_bytes = bs58::decode(api_key.trim()).into_vec().map_err(|e| {
            anyhow::anyhow!("Soyas api_token base58 解码失败（QUIC mTLS 用）: {}", e)
        })?;
        let keypair = Keypair::try_from(keypair_bytes.as_slice()).map_err(|e| {
            anyhow::anyhow!("Soyas api_token 无法解析为 Solana keypair（QUIC mTLS 用）: {}", e)
        })?;
        let (cert, key) = generate_client_tls_credentials(&keypair)?;
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
        wait_confirmation: bool,
    ) -> Result<()> {
        let start_time = Instant::now();
        let signature = transaction.get_signature();
        let serialized_tx = bincode::serialize(transaction)?;
        let connection = self.connection.load_full();
        if Self::try_send_bytes(&connection, &serialized_tx).await.is_err() {
            if crate::common::sdk_log::sdk_log_enabled() {
                crate::common::sdk_log::log_swqos_submission_failed(
                    "Soyas",
                    trade_type,
                    start_time.elapsed(),
                    "reconnecting",
                );
            }
            self.reconnect().await?;
            let connection = self.connection.load_full();
            if let Err(e) = Self::try_send_bytes(&connection, &serialized_tx).await {
                if crate::common::sdk_log::sdk_log_enabled() {
                    crate::common::sdk_log::log_swqos_submission_failed(
                        "Soyas",
                        trade_type,
                        start_time.elapsed(),
                        &e,
                    );
                }
                return Err(e.into());
            }
        }
        if crate::common::sdk_log::sdk_log_enabled() {
            crate::common::sdk_log::log_swqos_submitted("Soyas", trade_type, start_time.elapsed());
        }
        let start_time = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, *signature, wait_confirmation).await {
            Ok(_) => (),
            Err(e) => {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(" signature: {:?}", signature);
                    println!(
                        " [{:width$}] {} confirmation failed: {:?}",
                        "Soyas",
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
                "Soyas",
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
