//! Real signer/wire regression; local capturing transports never broadcast.
use anyhow::Result;
use sol_trade_sdk::{
    common::{GasFeeStrategy, GasFeeStrategyType, TradeTransactionVersion},
    swqos::{SwqosClient, SwqosClientTrait, SwqosType, TradeType},
    trading::core::{
        async_executor::execute_parallel_with_version, params::SenderConcurrencyConfig,
    },
};
use solana_hash::Hash;
use solana_sdk::{
    pubkey::Pubkey, signature::Keypair, signer::Signer, transaction::VersionedTransaction,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
struct Capture {
    minimum: Option<u64>,
    tip: Pubkey,
    lookups: AtomicUsize,
    wires: Mutex<Vec<Vec<u8>>>,
}

#[tokio::test]
async fn fully_filtered_batch_does_not_poison_next_equal_tip_signed_batch() {
    for dedicated in [false, true] {
        let make = |minimum| {
            Arc::new(Capture {
                minimum: Some(minimum),
                tip: Pubkey::new_unique(),
                lookups: AtomicUsize::new(0),
                wires: Mutex::new(vec![]),
            })
        };
        let (boundary, blocked) = (make(100_000), make(100_001));
        let clients: Vec<Arc<SwqosClient>> = vec![boundary.clone(), blocked.clone()];
        let payer = Arc::new(Keypair::new());
        let strategy = GasFeeStrategy::new();
        strategy.set_normal_fee_strategy(SwqosType::Jito, 200_000, 0, 0.000099999, 0.000099999);
        let execute = || {
            execute_parallel_with_version(
                &clients,
                payer.clone(),
                vec![],
                vec![],
                Some(Hash::new_unique()),
                None,
                None,
                "all-filtered-recovery",
                true,
                false,
                true,
                true,
                strategy.clone(),
                dedicated,
                SenderConcurrencyConfig {
                    sender_thread_cores: None,
                    effective_core_ids: Arc::new(vec![]),
                    max_sender_concurrency: 2,
                },
                false,
                TradeTransactionVersion::V1,
            )
        };
        assert!(execute().await.unwrap_err().to_string().contains("No available gas fee strategy"));
        for capture in [&boundary, &blocked] {
            assert_eq!(capture.lookups.load(Ordering::SeqCst), 0);
            assert!(capture.wires.lock().unwrap().is_empty());
        }
        strategy.set_normal_fee_strategy(SwqosType::Jito, 200_000, 0, 0.0001, 0.0001);
        assert!(execute().await.unwrap().0);
        assert_eq!(boundary.lookups.load(Ordering::SeqCst), 1);
        assert_eq!(blocked.lookups.load(Ordering::SeqCst), 0);
        assert!(blocked.wires.lock().unwrap().is_empty());
        let wires = boundary.wires.lock().unwrap();
        assert_eq!(wires.len(), 1);
        let tx: VersionedTransaction = wincode::deserialize_exact(&wires[0]).unwrap();
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap();
        assert_eq!(tx.message.static_account_keys()[0], payer.pubkey());
        let tip = tx
            .message
            .instructions()
            .iter()
            .find(|ix| ix.data.len() == 12 && ix.data[..4] == 2u32.to_le_bytes())
            .unwrap();
        assert_eq!(u64::from_le_bytes(tip.data[4..12].try_into().unwrap()), 100_000);
    }
}
#[async_trait::async_trait]
impl SwqosClientTrait for Capture {
    fn configured_min_tip_lamports(&self) -> Option<u64> {
        self.minimum
    }
    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Jito
    }
    fn get_tip_account(&self) -> Result<String> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(self.tip.to_string())
    }
    async fn send_transaction(
        &self,
        _: TradeType,
        tx: &VersionedTransaction,
        wait: bool,
    ) -> Result<()> {
        assert!(!wait, "confirmation RPC must be disabled on the hot path");
        tx.sanitize()?;
        tx.verify_and_hash_message()?;
        self.wires.lock().unwrap().push(wincode::serialize(tx)?);
        Ok(())
    }
    async fn send_transactions(
        &self,
        _: TradeType,
        _: &Vec<VersionedTransaction>,
        _: bool,
    ) -> Result<()> {
        unreachable!()
    }
}
#[tokio::test]
async fn decimal_routes_filter_before_lookup_build_sign_submit_for_buy_sell_and_both_pools() {
    for version in [TradeTransactionVersion::V0, TradeTransactionVersion::V1] {
        for buy in [false, true] {
            for dedicated in [false, true] {
                let payer = Arc::new(Keypair::new());
                let make = |minimum| {
                    Arc::new(Capture {
                        minimum,
                        tip: Pubkey::new_unique(),
                        lookups: AtomicUsize::new(0),
                        wires: Mutex::new(vec![]),
                    })
                };
                let blocked = make(Some(100_000_001));
                let boundary = make(Some(100_000_000));
                let plain = make(None);
                let clients: Vec<Arc<SwqosClient>> =
                    vec![blocked.clone(), boundary.clone(), plain.clone()];
                let strategy = GasFeeStrategy::new();
                let side = if buy { TradeType::Buy } else { TradeType::Sell };
                for (trade_type, tip) in
                    [(side, 0.1), (if buy { TradeType::Sell } else { TradeType::Buy }, 1.0)]
                {
                    strategy.set(
                        SwqosType::Jito,
                        trade_type,
                        GasFeeStrategyType::HighTipLowCuPrice,
                        200_000,
                        0,
                        tip,
                    );
                }
                strategy.set(
                    SwqosType::Jito,
                    side,
                    GasFeeStrategyType::LowTipHighCuPrice,
                    200_000,
                    0,
                    0.099999999,
                );
                let result = execute_parallel_with_version(
                    &clients,
                    payer.clone(),
                    vec![],
                    vec![],
                    Some(Hash::new_unique()),
                    None,
                    None,
                    "decimal-offline",
                    buy,
                    false,
                    true,
                    true,
                    strategy,
                    dedicated,
                    SenderConcurrencyConfig {
                        sender_thread_cores: None,
                        effective_core_ids: Arc::new(vec![]),
                        max_sender_concurrency: 2,
                    },
                    false,
                    version,
                )
                .await
                .unwrap();
                assert!(result.0);
                assert_eq!(blocked.lookups.load(Ordering::SeqCst), 0);
                assert!(blocked.wires.lock().unwrap().is_empty());
                for (route, expected) in
                    [(&boundary, vec![100_000_000]), (&plain, vec![99_999_999, 100_000_000])]
                {
                    assert_eq!(
                        route.lookups.load(Ordering::SeqCst),
                        1,
                        "tip lookup is cached per endpoint"
                    );
                    let mut amounts = vec![];
                    for wire in route.wires.lock().unwrap().iter() {
                        let parsed: VersionedTransaction = wincode::deserialize(wire).unwrap();
                        parsed.sanitize().unwrap();
                        parsed.verify_and_hash_message().unwrap();
                        assert_eq!(parsed.message.static_account_keys()[0], payer.pubkey());
                        let transfer = parsed
                            .message
                            .instructions()
                            .iter()
                            .find(|ix| {
                                parsed.message.static_account_keys()[ix.program_id_index as usize]
                                    == Pubkey::default()
                                    && ix.data.len() == 12
                                    && ix.data[..4] == 2u32.to_le_bytes()
                            })
                            .unwrap();
                        assert_eq!(&transfer.data[..4], &2u32.to_le_bytes());
                        amounts.push(u64::from_le_bytes(transfer.data[4..12].try_into().unwrap()));
                        assert!(parsed.message.static_account_keys().contains(&route.tip));
                    }
                    amounts.sort_unstable();
                    assert_eq!(amounts, expected);
                }
            }
        }
    }
}
#[test]
fn export_actual_sdk_transfer_for_independent_bank() {
    let Ok(output) = std::env::var("REVIEW_BANK_WIRE_OUTPUT") else {
        return;
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    let hash: Hash = std::env::var("REVIEW_BANK_BLOCKHASH").unwrap().parse().unwrap();
    let payer = Arc::new(Keypair::new());
    let recipient = Pubkey::new_unique();
    let ix = solana_system_interface::instruction::transfer(&payer.pubkey(), &recipient, 1_000_000);
    let tx = sol_trade_sdk::trading::common::build_transaction_with_version(
        &payer,
        200_000,
        0,
        TradeTransactionVersion::V0,
        &[ix],
        &[],
        Some(hash),
        None,
        "independent-bank",
        true,
        false,
        &Pubkey::default(),
        0.0,
        None,
    )
    .unwrap();
    tx.sanitize().unwrap();
    tx.verify_and_hash_message().unwrap();
    let artifact = serde_json::json!({"sdk":"sol-trade-sdk", "wire":STANDARD.encode(wincode::serialize(&tx).unwrap()),
        "payer":payer.pubkey().to_string(), "recipient":recipient.to_string(), "lamports":1_000_000, "blockhash":hash.to_string()});
    std::fs::write(output, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
}

#[test]
#[ignore = "explicit offline benchmark, no fixed latency assertion"]
fn benchmark_decimal_route_selection_and_local_signed_submission() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    sol_trade_sdk::trading::core::async_executor::warm_swqos_sender_pool(2).unwrap();
    let mut cases = vec![];
    for (name, minimum) in [("None", None), ("eligible", Some(100_000)), ("below", Some(100_001))] {
        let payer = Arc::new(Keypair::new());
        let capture = Arc::new(Capture {
            minimum,
            tip: Pubkey::new_unique(),
            lookups: AtomicUsize::new(0),
            wires: Mutex::new(vec![]),
        });
        let clients: Vec<Arc<SwqosClient>> = vec![capture.clone()];
        let strategy = GasFeeStrategy::new();
        strategy.set_normal_fee_strategy(SwqosType::Jito, 200_000, 0, 0.0001, 0.0001);
        let submit = || {
            execute_parallel_with_version(
                &clients,
                payer.clone(),
                vec![],
                vec![],
                Some(Hash::new_unique()),
                None,
                None,
                "offline-benchmark",
                true,
                false,
                true,
                true,
                strategy.clone(),
                false,
                SenderConcurrencyConfig {
                    sender_thread_cores: None,
                    effective_core_ids: Arc::new(vec![]),
                    max_sender_concurrency: 2,
                },
                false,
                TradeTransactionVersion::V0,
            )
        };
        for _ in 0..50 {
            let _ = runtime.block_on(submit());
        }
        let mut samples = Vec::with_capacity(500);
        for _ in 0..500 {
            let start = std::time::Instant::now();
            let result = runtime.block_on(submit());
            samples.push(start.elapsed().as_nanos() as u64);
            assert_eq!(result.is_ok(), name != "below");
            capture.wires.lock().unwrap().clear();
        }
        samples.sort_unstable();
        cases.push(serde_json::json!({"case":name,"iterations":500,"warmup":50,"median_ns":samples[250],"p95_ns":samples[475]}));
    }
    let artifact = serde_json::json!({"sdk":"sol-trade-sdk","profile":"cargo test dev optimized opt-level=1","arch":std::env::consts::ARCH,
        "scope":"executor fee selection, local real signing, wire verification and capturing transport; no network; below skips all building", "cases":cases});
    std::fs::write(
        std::env::var("REVIEW_BENCH_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}
