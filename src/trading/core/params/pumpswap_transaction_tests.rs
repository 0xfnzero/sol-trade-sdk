use super::*;
use crate::{
    common::{mainnet_sim, nonce_cache::DurableNonceInfo, TradeTransactionVersion},
    constants::{SOL_TOKEN_ACCOUNT, TOKEN_PROGRAM, TOKEN_PROGRAM_2022},
    swqos::TradeType,
    trading::middleware::{InstructionMiddleware, MiddlewareManager},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use solana_message::{AddressLookupTableAccount, VersionedMessage};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};
use std::sync::Arc;

fn fixture(is_buy: bool) -> (PreparedPumpSwapTrade, SwapParams, CacheReadContext) {
    let f: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/pumpswap_ttwo_route_accounts.json"
    ))
    .unwrap();
    let mut cache = SubscriptionAccountCache::default();
    for a in f["accounts"].as_array().unwrap() {
        cache
            .update(
                a["address"].as_str().unwrap().parse().unwrap(),
                CachedAccount {
                    owner: a["owner"].as_str().unwrap().parse().unwrap(),
                    data: STANDARD.decode(a["data_base64"].as_str().unwrap()).unwrap(),
                    slot: a["slot"].as_u64().unwrap(),
                    write_version: 0,
                },
            )
            .unwrap();
    }
    let ctx = CacheReadContext {
        slot: f["slot"].as_u64().unwrap(),
        epoch: f["epoch"].as_u64().unwrap(),
        maximum_slot_age: f["maximum_slot_age"].as_u64().unwrap(),
    };
    let key = |v: &serde_json::Value| v.as_str().unwrap().parse::<Pubkey>().unwrap();
    let s = &f["route_search"];
    let search = PumpSwapRouteSearch::new(
        key(&s["pool"]),
        key(&s["token_mint"]),
        key(&s["asset_mint"]),
        s["conversion_pools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| PoolTradeHint {
                pool: key(&p["pool"]),
                input_mint: key(&p["input_mint"]),
                output_mint: key(&p["output_mint"]),
            })
            .collect(),
    );
    let request = CachedQuoteRequest {
        amount_in: if is_buy { 1_000_000 } else { 1_000_000_000 },
        slippage_basis_points: 100,
        unix_timestamp: f["unix_timestamp"].as_u64().unwrap(),
        maximum_arrays: 6,
    };
    let trade = cache.prepare_best_pumpswap_trade(&search, request, ctx, is_buy).unwrap().trade;
    let mut params = mainnet_sim::swap_params(
        mainnet_sim::create_wallet(),
        TradeType::Buy,
        Pubkey::default(),
        Pubkey::default(),
        0,
        0,
        DexParamEnum::PumpSwapQuoteRoute(Default::default()),
    );
    params.simulate = false;
    params.open_seed_optimize = true;
    params.close_output_mint_ata = !is_buy;
    params.recent_blockhash = Some(Hash::new_unique());
    (trade, params, ctx)
}

fn fee() -> GasFeeStrategyValue {
    GasFeeStrategyValue { cu_limit: 450_000, cu_price: 10_000, tip: 0.00001 }
}

async fn instructions(trade: &PreparedPumpSwapTrade, params: SwapParams) -> Vec<Instruction> {
    let is_buy = trade.is_buy;
    let params = trade.clone().apply_to(params).unwrap();
    if is_buy {
        PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap()
    } else {
        PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap()
    }
}

// Only a compilation fixture: this table does NOT exist on chain.
fn synthetic_alt(ixs: &[Instruction]) -> AddressLookupTableAccount {
    let mut addresses: Vec<_> = ixs
        .iter()
        .flat_map(|ix| ix.accounts.iter())
        .filter(|a| !a.is_signer)
        .map(|a| a.pubkey)
        .collect();
    addresses.sort();
    addresses.dedup();
    AddressLookupTableAccount { key: Pubkey::new_unique(), addresses }
}

#[tokio::test]
async fn pumpswap_route_seed_default_uses_repeatable_atas_in_every_leg() {
    for is_buy in [true, false] {
        let (trade, params, _) = fixture(is_buy);
        let payer = params.payer.pubkey();
        let ixs = instructions(&trade, params.clone()).await;
        let creates: Vec<_> = ixs
            .iter()
            .filter(|ix| ix.program_id == crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID)
            .collect();
        assert_eq!(creates.len(), 4);
        for ix in &creates {
            assert_eq!(ix.data, [1], "ATA creation must be idempotent");
            let mint = ix.accounts[3].pubkey;
            let program = ix.accounts[5].pubkey;
            assert!(program == TOKEN_PROGRAM || program == TOKEN_PROGRAM_2022);
            let ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &payer, &mint, &program,
            );
            assert_eq!(ix.accounts[1].pubkey, ata);
            for step in &trade.steps {
                let hop = step.pool;
                if hop.input_mint == mint || hop.output_mint == mint {
                    // All venue swaps sharing this mint must reference the setup ATA.
                    let matching: Vec<_> = ixs
                        .iter()
                        .filter(|ix| ix.accounts.iter().any(|a| a.pubkey == hop.pool))
                        .collect();
                    assert!(!matching.is_empty());
                    assert!(matching.iter().all(|ix| ix.accounts.iter().any(|a| a.pubkey == ata)));
                }
            }
        }
        let system: Vec<_> = ixs.iter().filter(|ix| ix.program_id == Pubkey::default()).collect();
        assert_eq!(system.len(), usize::from(is_buy));
        if is_buy {
            assert_eq!(system[0].data[..4], 2u32.to_le_bytes());
            assert_eq!(system[0].data[4..], trade.amount_in.to_le_bytes());
        }
        let closes: Vec<_> =
            ixs.iter().filter(|ix| ix.program_id == TOKEN_PROGRAM && ix.data == [9]).collect();
        assert_eq!(closes.len(), usize::from(!is_buy));
        if !is_buy {
            let wsol = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &payer,
                &canonical(SOL_TOKEN_ACCOUNT),
                &TOKEN_PROGRAM,
            );
            assert_eq!(closes[0].accounts[0].pubkey, wsol);
        }
        let mut unseeded = params.clone();
        unseeded.open_seed_optimize = false;
        let unseeded_ixs = instructions(&trade, unseeded).await;
        let unseeded_creates: Vec<_> = unseeded_ixs
            .iter()
            .filter(|ix| ix.program_id == crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID)
            .collect();
        assert_eq!(creates, unseeded_creates);
        // A subsequent construction retains idempotent setup for existing ATAs.
        let again = instructions(&trade, params).await;
        let again_creates: Vec<_> = again
            .iter()
            .filter(|ix| ix.program_id == crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID)
            .collect();
        assert_eq!(creates, again_creates);
    }
}

#[tokio::test]
async fn pumpswap_offline_build_checks_wire_size_version_and_signatures() {
    for is_buy in [true, false] {
        let (trade, mut params, ctx) = fixture(is_buy);
        let error = trade
            .clone()
            .build_transaction(params.clone(), ctx, fee(), Pubkey::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("transaction too large"));
        params.transaction_version = TradeTransactionVersion::V1;
        let tx = trade
            .clone()
            .build_transaction(params.clone(), ctx, fee(), Pubkey::default())
            .await
            .unwrap();
        assert!(matches!(tx.message, VersionedMessage::V1(_)));
        assert!(tx.signatures[0].verify(params.payer.pubkey().as_ref(), &tx.message.serialize()));
        assert_eq!(*tx.message.recent_blockhash(), params.recent_blockhash.unwrap());
        assert!(
            wincode::serialized_size(&tx).unwrap() as usize
                <= solana_message::v1::MAX_TRANSACTION_SIZE
        );
        params.address_lookup_table_accounts =
            vec![synthetic_alt(&instructions(&trade, params.clone()).await)];
        let error = trade
            .clone()
            .build_transaction(params.clone(), ctx, fee(), Pubkey::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("V1 transactions do not support"));
        params.transaction_version = TradeTransactionVersion::V0;
        let tx =
            trade.build_transaction(params.clone(), ctx, fee(), Pubkey::default()).await.unwrap();
        assert!(matches!(tx.message, VersionedMessage::V0(_)));
        assert!(wincode::serialized_size(&tx).unwrap() <= 1232);
        assert!(tx.signatures[0].verify(params.payer.pubkey().as_ref(), &tx.message.serialize()));
    }
}

#[tokio::test]
async fn pumpswap_offline_build_rejects_stale_epoch_future_and_missing_hash() {
    let (trade, mut params, ctx) = fixture(true);
    trade
        .validate_freshness(CacheReadContext { slot: ctx.slot + ctx.maximum_slot_age, ..ctx })
        .unwrap();
    for (current, expected) in [
        (
            CacheReadContext {
                slot: ctx.slot + ctx.maximum_slot_age + 1,
                maximum_slot_age: u64::MAX,
                ..ctx
            },
            "stale",
        ),
        (CacheReadContext { slot: ctx.slot + 1, maximum_slot_age: 0, ..ctx }, "stale"),
        (CacheReadContext { slot: ctx.slot - 1, ..ctx }, "future slot"),
        (CacheReadContext { epoch: ctx.epoch + 1, ..ctx }, "epoch changed"),
    ] {
        let error = trade
            .clone()
            .build_transaction(params.clone(), current, fee(), Pubkey::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    params.recent_blockhash = None;
    let error = trade.build_transaction(params, ctx, fee(), Pubkey::default()).await.unwrap_err();
    assert!(error.to_string().contains("Must provide either"));
}

#[tokio::test]
async fn pumpswap_offline_build_retains_nonce_tip_and_compute_budget() {
    let (trade, mut params, ctx) = fixture(true);
    let nonce = Pubkey::new_unique();
    let hash = Hash::new_unique();
    let tip = Pubkey::new_unique();
    params.with_tip = true;
    params.durable_nonce =
        Some(DurableNonceInfo { nonce_account: Some(nonce), current_nonce: Some(hash) });
    params.address_lookup_table_accounts =
        vec![synthetic_alt(&instructions(&trade, params.clone()).await)];
    let tx = trade.clone().build_transaction(params.clone(), ctx, fee(), tip).await.unwrap();
    assert_eq!(*tx.message.recent_blockhash(), hash);
    let ixs = tx.message.instructions();
    assert_eq!(ixs[0].data, 4u32.to_le_bytes()); // advance_nonce_account must be first
    let keys = tx.message.static_account_keys();
    assert_eq!(keys[ixs[0].accounts[0] as usize], nonce);
    assert_eq!(ixs[1].data[..4], 2u32.to_le_bytes());
    assert_eq!(ixs[1].data[4..], 10_000u64.to_le_bytes());
    assert_eq!(keys[ixs[1].accounts[1] as usize], tip);
    assert_eq!(ixs[2].data, [vec![3], fee().cu_price.to_le_bytes().to_vec()].concat());
    assert_eq!(ixs[3].data, [vec![2], fee().cu_limit.to_le_bytes().to_vec()].concat());
    params.durable_nonce.as_mut().unwrap().current_nonce = None;
    assert!(trade
        .build_transaction(params, ctx, fee(), tip)
        .await
        .unwrap_err()
        .to_string()
        .contains("current_nonce is None"));
}

#[derive(Clone)]
struct ExtraInstruction {
    protocol: Instruction,
    full: Instruction,
}
impl InstructionMiddleware for ExtraInstruction {
    fn name(&self) -> &'static str {
        "offline-build-test"
    }
    fn process_protocol_instructions(
        &self,
        mut ixs: Vec<Instruction>,
        name: &str,
        buy: bool,
    ) -> Result<Vec<Instruction>> {
        assert_eq!(name, "PumpSwap");
        assert!(buy);
        ixs.push(self.protocol.clone());
        Ok(ixs)
    }
    fn process_full_instructions(
        &self,
        mut ixs: Vec<Instruction>,
        name: &str,
        buy: bool,
    ) -> Result<Vec<Instruction>> {
        assert_eq!(name, "PumpSwap");
        assert!(buy);
        assert!(ixs.contains(&self.protocol));
        ixs.push(self.full.clone());
        Ok(ixs)
    }
    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
    }
}

#[tokio::test]
async fn pumpswap_offline_build_checks_final_middleware_size_and_signers() {
    let (trade, mut params, ctx) = fixture(true);
    params.transaction_version = TradeTransactionVersion::V1;
    let small = Instruction { program_id: Pubkey::new_unique(), accounts: vec![], data: vec![42] };
    for extra in [
        Instruction { data: vec![0; solana_message::v1::MAX_TRANSACTION_SIZE], ..small.clone() },
        Instruction {
            accounts: vec![AccountMeta::new_readonly(Pubkey::new_unique(), true)],
            ..small.clone()
        },
    ] {
        let expected =
            if extra.accounts.is_empty() { "transaction too large" } else { "additional signers" };
        params.middleware_manager =
            Some(Arc::new(MiddlewareManager::new().add_middleware(Box::new(ExtraInstruction {
                protocol: small.clone(),
                full: extra,
            }))));
        let error = trade
            .clone()
            .build_transaction(params.clone(), ctx, fee(), Pubkey::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[tokio::test]
async fn pumpswap_offline_build_rejects_invalid_tips_and_rpc_flags() {
    let (trade, mut params, ctx) = fixture(true);
    params.transaction_version = TradeTransactionVersion::V1;
    for tip in [f64::NAN, f64::INFINITY, f64::MAX, -0.001] {
        let error = trade
            .clone()
            .build_transaction(
                params.clone(),
                ctx,
                GasFeeStrategyValue { tip, ..fee() },
                Pubkey::new_unique(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Invalid PumpSwap transaction tip"));
    }
    params.with_tip = true;
    assert!(trade
        .clone()
        .build_transaction(params.clone(), ctx, fee(), Pubkey::default())
        .await
        .unwrap_err()
        .to_string()
        .contains("requires a destination"));
    for simulate in [true, false] {
        params.simulate = simulate;
        params.wait_tx_confirmed = !simulate;
        assert!(trade
            .clone()
            .build_transaction(params.clone(), ctx, fee(), Pubkey::new_unique())
            .await
            .unwrap_err()
            .to_string()
            .contains("RPC-free"));
    }
}

/// Each venue in the shared route builder must retain its own quote policy.
/// Sell starts with PumpSwap and then visits DLMM
/// and AMMv4, exercising policy overrides followed by unrelated venues.
#[tokio::test]
async fn mixed_venue_routes_match_independent_leg_builders_for_each_quote_policy() {
    use crate::instruction::{
        meteora_dlmm::MeteoraDlmmInstructionBuilder,
        raydium_amm_v4::RaydiumAmmV4InstructionBuilder,
        stonkfun_quote_route::{build_quote_route, plan_route},
    };
    for is_buy in [true, false] {
        let (trade, settings, _) = fixture(is_buy);
        let mut applied = trade.apply_to(settings).unwrap();
        let route = match &applied.protocol_params {
            DexParamEnum::PumpSwapQuoteRoute(route) => route.clone(),
            _ => unreachable!(),
        };
        // Test the generic conversion-route contract as well as PumpSwap's
        // forced exact-input leg, independently of its dedicated route guard.
        applied.protocol_params = DexParamEnum::StonkFunQuoteRoute(route.clone());
        applied.open_seed_optimize = false;
        for policy in [None, Some(false), Some(true)] {
            applied.use_exact_sol_amount = policy;
            let plan = plan_route(
                &route,
                applied.input_mint,
                applied.output_mint,
                applied.input_amount.unwrap(),
                applied.slippage_basis_points.unwrap(),
                is_buy,
            )
            .unwrap();
            let mut expected = Vec::new();
            for hop in &plan.hops {
                let mut leg = applied.clone();
                leg.input_mint = hop.input;
                leg.output_mint = hop.output;
                leg.input_amount = Some(hop.amount);
                leg.input_token_program = Some(hop.input_program);
                leg.output_token_program = Some(hop.output_program);
                leg.create_input_mint_ata = false;
                leg.create_output_mint_ata = false;
                leg.close_input_mint_ata = false;
                leg.close_output_mint_ata = false;
                let instructions = match &hop.hop.venue {
                    StonkFunQuoteVenue::PumpSwap { pool, .. } => {
                        leg.protocol_params = DexParamEnum::PumpSwap(pool.clone());
                        leg.fixed_output_amount = None;
                        leg.use_exact_sol_amount = Some(true);
                        PumpSwapInstructionBuilder.build_buy_instructions(&leg).await
                    }
                    StonkFunQuoteVenue::RaydiumAmmV4(pool) => {
                        leg.protocol_params = DexParamEnum::RaydiumAmmV4(pool.clone());
                        leg.fixed_output_amount = None;
                        RaydiumAmmV4InstructionBuilder.build_buy_instructions(&leg).await
                    }
                    StonkFunQuoteVenue::MeteoraDlmm(pool) => {
                        leg.protocol_params = DexParamEnum::MeteoraDlmm(pool.clone());
                        leg.fixed_output_amount = Some(hop.hop.minimum_amount_out);
                        MeteoraDlmmInstructionBuilder.build_buy_instructions(&leg).await
                    }
                    _ => panic!("Unexpected venue in frozen three-hop fixture"),
                }
                .unwrap();
                expected.extend(instructions);
            }
            let actual = build_quote_route(&applied, &route, is_buy).await.unwrap();
            let swaps: Vec<_> = actual
                .into_iter()
                .filter(|ix| expected.iter().any(|want| ix.program_id == want.program_id))
                .collect();
            assert_eq!(swaps, expected, "direction {is_buy}, caller policy {policy:?}");
            assert_eq!(applied.use_exact_sol_amount, policy);
        }
    }
}
