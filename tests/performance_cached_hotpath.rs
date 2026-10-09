//! Manual production API baseline: cached preparation/signing and real parser APIs; no network.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sol_trade_sdk::{
    common::{GasFeeStrategy, TradeTransactionVersion},
    instruction::raydium_cpmm::RaydiumCpmmInstructionBuilder,
    swqos::TradeType,
    trading::{
        common::build_transaction_with_version,
        core::{
            params::{
                CacheReadContext, CachedAccount, DexParamEnum, PoolTradeHint,
                SubscriptionAccountCache, SwapParams,
            },
            traits::InstructionBuilder,
        },
    },
};
use solana_sdk::{signature::Keypair, signer::Signer, transaction::VersionedTransaction};
use std::{hint::black_box, sync::Arc, time::Instant};
fn measure(mut f: impl FnMut(), name: &str) -> Value {
    for _ in 0..100 {
        f()
    }
    let mut rounds = Vec::new();
    for _ in 0..3 {
        let mut raw = Vec::new();
        for _ in 0..1000 {
            let t = Instant::now();
            f();
            raw.push(t.elapsed().as_nanos() as u64);
        }
        let mut s = raw.clone();
        s.sort_unstable();
        rounds.push(json!({"raw_ns":raw,"p50_ns":s[499],"p95_ns":s[949],"p99_ns":s[989]}));
    }
    json!({"stage":name,"rounds":rounds})
}
#[test]
#[ignore = "bounded manual release baseline"]
fn actual_cached_and_parser_hotpaths() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().build()?;
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cpmm_transfer_fee_bank_20261008.json"))?;
    let payer = Arc::new(Keypair::new());
    let mut measurements = Vec::new();
    for case in fixture["cases"].as_array().unwrap().iter().take(2) {
        let s = &case["snapshot"];
        let mut cache = SubscriptionAccountCache::default();
        for a in s["accounts"].as_array().unwrap() {
            cache.update(
                a["pubkey"].as_str().unwrap().parse()?,
                CachedAccount {
                    owner: a["owner"].as_str().unwrap().parse()?,
                    data: STANDARD.decode(a["data"].as_str().unwrap())?,
                    slot: a["slot"].as_str().unwrap().parse()?,
                    write_version: 0,
                },
            )?;
        }
        let leg = &s["legs"][0];
        let hint = PoolTradeHint {
            pool: leg["pool"].as_str().unwrap().parse()?,
            input_mint: leg["input_mint"].as_str().unwrap().parse()?,
            output_mint: leg["output_mint"].as_str().unwrap().parse()?,
        };
        let ctx = CacheReadContext {
            slot: s["read_slot"].as_str().unwrap().parse()?,
            epoch: s["epoch"].as_str().unwrap().parse()?,
            maximum_slot_age: 0,
        };
        let timestamp = s["unix_timestamp"].as_str().unwrap().parse()?;
        let hash = s["recent_blockhash"].as_str().unwrap().parse()?;
        let side = if s["trade_type"] == "Buy" { TradeType::Buy } else { TradeType::Sell };
        let amount = s["amount"].as_str().unwrap().parse()?;
        let prepare = || -> Result<_> {
            let params = SwapParams {
                rpc: None,
                payer: payer.clone(),
                trade_type: side,
                input_mint: hint.input_mint,
                input_token_program: None,
                output_mint: hint.output_mint,
                output_token_program: None,
                input_amount: Some(amount),
                slippage_basis_points: Some(1000),
                address_lookup_table_accounts: vec![],
                recent_blockhash: Some(hash),
                wait_tx_confirmed: false,
                protocol_params: DexParamEnum::RaydiumCpmm(cache.cpmm_at(hint, ctx, timestamp)?),
                open_seed_optimize: false,
                swqos_clients: Arc::new(vec![]),
                middleware_manager: None,
                durable_nonce: None,
                with_tip: false,
                create_input_mint_ata: false,
                close_input_mint_ata: false,
                create_output_mint_ata: true,
                close_output_mint_ata: false,
                fixed_output_amount: None,
                gas_fee_strategy: GasFeeStrategy::new(),
                simulate: false,
                log_enabled: false,
                wait_for_all_submits: false,
                use_dedicated_sender_threads: false,
                sender_thread_cores: None,
                max_sender_concurrency: 0,
                effective_core_ids: Arc::new(vec![]),
                check_min_tip: false,
                transaction_version: TradeTransactionVersion::V1,
                grpc_recv_us: None,
                use_exact_sol_amount: None,
            };
            rt.block_on(async {
                if side == TradeType::Buy {
                    RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await
                } else {
                    RaydiumCpmmInstructionBuilder.build_sell_instructions(&params).await
                }
            })
        };
        let ix = prepare()?;
        assert!(!ix.is_empty());
        measurements.push(measure(
            || {
                black_box(prepare().unwrap());
            },
            &format!("{}: cache decode quote instructions", case["validation"]["name"]),
        ));
        for version in [TradeTransactionVersion::V0, TradeTransactionVersion::V1] {
            let sign = |ix: &[_]| -> Result<Vec<u8>> {
                let tx = build_transaction_with_version(
                    &payer,
                    300000,
                    0,
                    version,
                    ix,
                    &[],
                    Some(hash),
                    None,
                    "RaydiumCpmm",
                    side == TradeType::Buy,
                    false,
                    &payer.pubkey(),
                    0.,
                    None,
                )?;
                Ok(wincode::serialize(&tx)?)
            };
            let wire = sign(&ix)?;
            let decoded: VersionedTransaction = wincode::deserialize_exact(&wire)?;
            decoded.sanitize()?;
            decoded.verify_and_hash_message()?;
            measurements.push(measure(
                || {
                    black_box(sign(&ix).unwrap());
                },
                &format!("{}: {:?} compile sign serialize", case["validation"]["name"], version),
            ));
            measurements.push(measure(
                || {
                    black_box(sign(&prepare().unwrap()).unwrap());
                },
                &format!(
                    "{}: {:?} cached prepare compile sign serialize",
                    case["validation"]["name"], version
                ),
            ));
        }
    }
    let parser: Value = serde_json::from_str(include_str!(
        "../../sol-parser-sdk/tests/fixtures/signed_cpmm_bank_20261009.json"
    ))?;
    for case in parser["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "fee-input" || c["name"] == "slippage-reject")
    {
        let mut rpc = case["rpc"].clone();
        rpc["transaction"] = json!([case["wire"], "base64"]);
        let rpc: solana_transaction_status_client_types::EncodedConfirmedTransactionWithStatusMeta =
            serde_json::from_value(rpc)?;
        let (meta, transaction) = sol_parser_sdk::convert_rpc_to_grpc(&rpc)?;
        let sig = rpc.transaction.transaction.decode().unwrap().signatures[0];
        let some = Some(transaction.clone());
        let route = sol_parser_sdk::analyze_rpc_transaction_routes(&rpc, &[])?;
        assert_eq!(route.succeeded, case["name"] == "fee-input");
        assert_eq!(route.legs.len(), 1);
        measurements.push(measure(
            || {
                black_box(sol_parser_sdk::analyze_rpc_transaction_routes(&rpc, &[]).unwrap());
            },
            &format!("{}: Parser RPC decode routes", case["name"]),
        ));
        measurements.push(measure(
            || {
                black_box(sol_parser_sdk::analyze_yellowstone_transaction_routes(
                    &transaction,
                    &meta,
                    &[],
                ));
            },
            &format!("{}: Parser prepared Yellowstone routes", case["name"]),
        ));
        measurements.push(measure(
            || {
                black_box(sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                    &meta, &some, sig, rpc.slot, 0, None, 0, None,
                ));
            },
            &format!("{}: Parser enhanced instruction events", case["name"]),
        ));
    }
    std::fs::write(
        std::env::var("PERFORMANCE_OUTPUT")?,
        serde_json::to_vec_pretty(
            &json!({"warmup":100,"samples":1000,"rounds":3,"profile":"default release","measurements":measurements}),
        )?,
    )?;
    Ok(())
}
