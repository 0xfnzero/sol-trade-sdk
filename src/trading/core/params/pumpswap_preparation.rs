//! PumpSwap account validation and complete route preparation from subscription data.
use super::*;
use crate::{
    client::{BuyAmount, SellAmount, SimpleBuyParams, SimpleSellParams, TradeTokenType},
    common::{GasFeeStrategy, GasFeeStrategyValue},
    instruction::{
        pumpswap::PumpSwapInstructionBuilder,
        utils::{pumpswap as ps, pumpswap_types as types},
    },
    trading::{
        common::build_transaction_with_version,
        core::{execution::InstructionProcessor, traits::InstructionBuilder},
        factory::{DexType, TradeFactory},
    },
};
use anyhow::{anyhow, ensure, Result};
use solana_sdk::{hash::Hash, pubkey::Pubkey, transaction::VersionedTransaction};

pub(super) fn canonical(mint: Pubkey) -> Pubkey {
    if mint == crate::constants::SOL_TOKEN_ACCOUNT {
        crate::constants::WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}

fn asset(mint: Pubkey) -> TradeTokenType {
    match mint {
        crate::constants::SOL_TOKEN_ACCOUNT => TradeTokenType::SOL,
        crate::constants::WSOL_TOKEN_ACCOUNT => TradeTokenType::WSOL,
        crate::constants::USDC_TOKEN_ACCOUNT => TradeTokenType::USDC,
        _ => TradeTokenType::Token(mint),
    }
}

/// Immutable exact-input route quoted from a caller-selected cache snapshot.
/// Reprepare after state/epoch changes. This is a quote, not an execution guarantee.
#[derive(Clone)]
pub struct PreparedPumpSwapTrade {
    route: PumpSwapQuoteRoute,
    steps: Vec<CachedRouteStep>,
    preview: StonkFunQuoteRoutePreview,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount_in: u64,
    slippage_basis_points: u16,
    is_buy: bool,
    context: CacheReadContext,
}

impl PreparedPumpSwapTrade {
    pub(crate) fn into_quote_route(self) -> PumpSwapQuoteRoute {
        self.route
    }

    /// Execution-ordered pool identities and exact quoted inputs. SOL is represented
    /// as WSOL here; high-level request constructors preserve native SOL endpoints.
    pub fn steps(&self) -> &[CachedRouteStep] {
        &self.steps
    }

    pub fn preview(&self) -> &StonkFunQuoteRoutePreview {
        &self.preview
    }
    pub fn context(&self) -> CacheReadContext {
        self.context
    }

    /// Check quote age against the caller's latest slot/epoch subscription. Uses
    /// the stricter of the preparation and current age limits. This bounds time
    /// since preparation, not the current age of each source account; reprepare
    /// to revalidate accounts. It cannot establish fork consistency or fillability.
    pub fn validate_freshness(&self, current: CacheReadContext) -> Result<()> {
        ensure!(current.epoch == self.context.epoch, "PumpSwap quote epoch changed; reprepare");
        let age = current
            .slot
            .checked_sub(self.context.slot)
            .ok_or_else(|| anyhow!("PumpSwap quote is from a future slot; reprepare"))?;
        ensure!(
            age <= current.maximum_slot_age.min(self.context.maximum_slot_age),
            "PumpSwap quote is stale; reprepare"
        );
        Ok(())
    }

    /// Build and locally sign one complete transaction without submitting it.
    /// `fee` and `tip_account` describe one chosen send lane, not the entire gas
    /// strategy. Preserves account flags, middleware, version, ALTs and nonce.
    /// The production builder checks final wire size and supported signers after
    /// both middleware stages. No automatic version switch or priority removal.
    /// RPC-dependent simulation/confirmation flags must be disabled. Caller-owned
    /// middleware must also be offline to maintain the no-RPC property.
    pub async fn build_transaction(
        self,
        params: SwapParams,
        current: CacheReadContext,
        fee: GasFeeStrategyValue,
        tip_account: Pubkey,
    ) -> Result<VersionedTransaction> {
        self.validate_freshness(current)?;
        ensure!(
            fee.tip.is_finite() && fee.tip >= 0.0 && fee.tip * 1_000_000_000.0 < u64::MAX as f64,
            "Invalid PumpSwap transaction tip"
        );
        ensure!(
            !params.with_tip || fee.tip == 0.0 || tip_account != Pubkey::default(),
            "PumpSwap transaction tip requires a destination"
        );
        let is_buy = self.is_buy;
        let params = self.apply_to(params)?;
        let protocol_name = TradeFactory::create_executor(DexType::PumpSwap).protocol_name();
        let instructions = if is_buy {
            PumpSwapInstructionBuilder.build_buy_instructions(&params).await?
        } else {
            PumpSwapInstructionBuilder.build_sell_instructions(&params).await?
        };
        InstructionProcessor::preprocess(&instructions)?;
        let instructions = match &params.middleware_manager {
            Some(manager) => manager.apply_middlewares_process_protocol_instructions(
                instructions,
                protocol_name,
                is_buy,
            )?,
            None => instructions,
        };
        build_transaction_with_version(
            &params.payer,
            fee.cu_limit,
            fee.cu_price,
            params.transaction_version,
            &instructions,
            &params.address_lookup_table_accounts,
            params.recent_blockhash,
            params.middleware_manager.as_ref(),
            protocol_name,
            is_buy,
            params.with_tip,
            &tip_account,
            fee.tip,
            params.durable_nonce.as_ref(),
        )
    }

    /// Preserves prepared amount, slippage, endpoints and direction. AccountPolicy
    /// defaults to Auto; configure ALTs, nonce or simulation on the returned request.
    pub fn into_buy_params(self, blockhash: Hash, gas: GasFeeStrategy) -> Result<SimpleBuyParams> {
        ensure!(self.is_buy, "Prepared PumpSwap trade is a sell, not a buy");
        Ok(SimpleBuyParams::new(
            DexType::PumpSwap,
            asset(self.input_mint),
            self.output_mint,
            BuyAmount::ExactInput(self.amount_in),
            DexParamEnum::PumpSwapQuoteRoute(self.route),
            blockhash,
            gas,
        )
        .slippage_basis_points(u64::from(self.slippage_basis_points)))
    }

    pub fn into_sell_params(
        self,
        blockhash: Hash,
        gas: GasFeeStrategy,
    ) -> Result<SimpleSellParams> {
        ensure!(!self.is_buy, "Prepared PumpSwap trade is a buy, not a sell");
        Ok(SimpleSellParams::new(
            DexType::PumpSwap,
            asset(self.output_mint),
            self.input_mint,
            SellAmount::ExactInput(self.amount_in),
            DexParamEnum::PumpSwapQuoteRoute(self.route),
            blockhash,
            gas,
        )
        .slippage_basis_points(u64::from(self.slippage_basis_points)))
    }

    /// Retain the caller's wallet, nonce/blockhash, ALTs and account flags. Quote
    /// execution receives no RPC; use the high-level constructors for Auto flags.
    pub fn apply_to(self, mut params: SwapParams) -> Result<SwapParams> {
        params.trade_type =
            if self.is_buy { crate::swqos::TradeType::Buy } else { crate::swqos::TradeType::Sell };
        params.input_mint = self.input_mint;
        params.output_mint = self.output_mint;
        params.input_token_program = None;
        params.output_token_program = None;
        params.input_amount = Some(self.amount_in);
        params.slippage_basis_points = Some(u64::from(self.slippage_basis_points));
        params.fixed_output_amount = None;
        params.use_exact_sol_amount = Some(true);
        params.protocol_params = DexParamEnum::PumpSwapQuoteRoute(self.route);
        params.without_rpc()
    }
}

impl SubscriptionAccountCache {
    /// Validate current pool, both mints/vaults and both fee configs. No fallback
    /// to historical event accounts, legacy fees or guessed token programs.
    /// Requires zero mint transfer fees; cashback pools need explicit observed
    /// fee parameters and are rejected here because Pool does not contain the rate.
    pub fn pumpswap(&self, hint: PoolTradeHint, ctx: CacheReadContext) -> Result<PumpSwapParams> {
        let data = self.get(hint.pool, ps::accounts::AMM_PROGRAM, ctx)?;
        ensure!(
            data.get(..8) == Some(types::POOL_DISCRIMINATOR.as_slice()),
            "Invalid cached PumpSwap pool discriminator"
        );
        let pool = types::pool_decode(&data[8..])
            .ok_or_else(|| anyhow!("Invalid cached PumpSwap pool"))?;
        hint.matches(pool.base_mint, pool.quote_mint)?;
        ensure!(
            !pool.is_cashback_coin,
            "Cached PumpSwap cashback pool requires explicit observed cashback fee parameters"
        );
        let global = ps::decode_global_config(self.get(
            ps::accounts::GLOBAL_ACCOUNT,
            ps::accounts::AMM_PROGRAM,
            ctx,
        )?)
        .ok_or_else(|| anyhow!("Invalid cached PumpSwap global config"))?;
        let fees = ps::decode_fee_config(self.get(
            ps::accounts::FEE_CONFIG,
            ps::accounts::FEE_PROGRAM,
            ctx,
        )?)
        .ok_or_else(|| anyhow!("Invalid cached PumpSwap fee config"))?;
        let (base_program, base_fee) = self.mint(pool.base_mint, None, ctx)?;
        let (quote_program, quote_fee) = self.mint(pool.quote_mint, None, ctx)?;
        ensure!(
            base_fee.basis_points == 0 && quote_fee.basis_points == 0,
            "Cached PumpSwap routes require zero transfer fees on both mints"
        );
        let reserve = |vault: Pubkey, mint: Pubkey, program: Pubkey| -> Result<u64> {
            let bytes = self.get(vault, program, ctx)?;
            if program == crate::constants::TOKEN_PROGRAM {
                ensure!(bytes.len() == 165, "Invalid classic PumpSwap vault length");
            }
            use spl_token_2022_interface::{
                extension::StateWithExtensions,
                state::{Account, AccountState},
            };
            let account = StateWithExtensions::<Account>::unpack(bytes)
                .map_err(|error| anyhow!("Invalid cached PumpSwap vault: {error}"))?;
            ensure!(
                account.base.mint.to_bytes() == mint.to_bytes()
                    && account.base.owner.to_bytes() == hint.pool.to_bytes()
                    && account.base.state == AccountState::Initialized,
                "PumpSwap vault mint, authority or state does not match the pool"
            );
            Ok(account.base.amount)
        };
        let base_mint = self.get(pool.base_mint, base_program, ctx)?;
        let snapshot = ps::PoolRpcSnapshot {
            base_reserve: reserve(pool.pool_base_token_account, pool.base_mint, base_program)?,
            quote_reserve: reserve(pool.pool_quote_token_account, pool.quote_mint, quote_program)?,
            base_token_program: base_program,
            quote_token_program: quote_program,
            base_mint_supply: u64::from_le_bytes(base_mint[36..44].try_into()?),
        };
        let params = PumpSwapParams::from_cached_pool_snapshot_strict(
            &hint.pool, &pool, &snapshot, &fees, &global,
        )?;
        params.validate_accounts()?;
        Ok(params)
    }

    /// Prepare the entire explicit route, including PumpSwap, from cached accounts.
    /// A direct trade is one step. Buy and sell require separate direction-specific
    /// steps. SOL aliases are normalized for quoting but preserved for wrap/unwrap.
    pub fn prepare_pumpswap_trade(
        &self,
        steps: &[CachedRouteStep],
        input_mint: Pubkey,
        output_mint: Pubkey,
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
    ) -> Result<PreparedPumpSwapTrade> {
        self.prepare_pumpswap_trade_with_bridge_limits(
            steps,
            input_mint,
            output_mint,
            request,
            ctx,
            is_buy,
            None,
        )
    }

    pub(super) fn prepare_pumpswap_trade_with_bridge_limits(
        &self,
        steps: &[CachedRouteStep],
        input_mint: Pubkey,
        output_mint: Pubkey,
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
        bridge_limits: Option<super::cached_quote::BridgeQuoteLimits>,
    ) -> Result<PreparedPumpSwapTrade> {
        ensure!(
            steps.iter().any(|step| self
                .accounts
                .get(&step.pool.pool)
                .is_some_and(|a| a.owner == ps::accounts::AMM_PROGRAM)),
            "PumpSwap route must include a PumpSwap pool"
        );
        let steps: Vec<_> = steps
            .iter()
            .map(|step| CachedRouteStep {
                pool: PoolTradeHint {
                    input_mint: canonical(step.pool.input_mint),
                    output_mint: canonical(step.pool.output_mint),
                    ..step.pool
                },
                input_amount: step.input_amount,
            })
            .collect();
        let route = self.quote_route_exact_in_with_bridge_limits(
            &steps,
            canonical(input_mint),
            canonical(output_mint),
            request,
            ctx,
            is_buy,
            bridge_limits,
        )?;
        let preview = route.preview(
            input_mint,
            output_mint,
            request.amount_in,
            u64::from(request.slippage_basis_points),
            is_buy,
        )?;
        Ok(PreparedPumpSwapTrade {
            route,
            steps: steps
                .into_iter()
                .zip(&preview.hops)
                .map(|(mut step, hop)| {
                    step.input_amount = Some(hop.input_amount);
                    step
                })
                .collect(),
            preview,
            input_mint,
            output_mint,
            amount_in: request.amount_in,
            slippage_basis_points: request.slippage_basis_points,
            is_buy,
            context: ctx,
        })
    }
}

#[cfg(test)]
#[path = "pumpswap_transaction_tests.rs"]
mod transaction_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::mainnet_sim, instruction::pumpswap::PumpSwapInstructionBuilder, swqos::TradeType,
        trading::core::traits::InstructionBuilder,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};

    fn fixture() -> (SubscriptionAccountCache, PoolTradeHint, CacheReadContext) {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/pumpswap_ttwo_accounts.json"
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
        let hint = PoolTradeHint {
            pool: f["pool"].as_str().unwrap().parse().unwrap(),
            input_mint: solana_sdk::pubkey!("TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo"),
            output_mint: solana_sdk::pubkey!("nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump"),
        };
        (
            cache,
            hint,
            CacheReadContext {
                slot: f["slot"].as_u64().unwrap(),
                epoch: f["epoch"].as_u64().unwrap(),
                maximum_slot_age: f["maximum_slot_age"].as_u64().unwrap(),
            },
        )
    }
    fn request(amount_in: u64) -> CachedQuoteRequest {
        CachedQuoteRequest {
            amount_in,
            slippage_basis_points: 100,
            unix_timestamp: 1792302000,
            maximum_arrays: 6,
        }
    }
    fn step(pool: PoolTradeHint) -> CachedRouteStep {
        CachedRouteStep { pool, input_amount: None }
    }
    fn reversed(h: PoolTradeHint) -> PoolTradeHint {
        PoolTradeHint { input_mint: h.output_mint, output_mint: h.input_mint, ..h }
    }

    #[cfg(feature = "parser-adapter")]
    #[test]
    fn pumpswap_parser_hint_requotes_without_copying_observed_amounts() {
        use sol_parser_sdk::transaction_route::{InstructionPosition, RouteSwapLeg, SwapProtocol};
        let (cache, hint, ctx) = fixture();
        let mut leg = RouteSwapLeg {
            position: InstructionPosition {
                outer_index: 1,
                inner_index: Some(0),
                stack_height: Some(2),
            },
            program: ps::accounts::AMM_PROGRAM,
            protocol: SwapProtocol::PumpSwap,
            pool: hint.pool,
            trader: Pubkey::new_unique(),
            input_account: Pubkey::new_unique(),
            output_account: Pubkey::new_unique(),
            input_mint: Some(hint.input_mint),
            output_mint: Some(hint.output_mint),
            amount_specified_is_input: false,
            specified_amount: u64::MAX,
            other_amount_threshold: u64::MAX,
            actual_input_amount: None,
            actual_output_amount: None,
            stonkfun_mode: None,
            stonkfun_graduated: false,
        };
        let inferred = PoolTradeHint::from_route_leg(&leg).unwrap();
        let prepared = cache
            .prepare_pumpswap_trade(
                &[step(inferred)],
                hint.input_mint,
                hint.output_mint,
                request(10_000),
                ctx,
                true,
            )
            .unwrap();
        assert_eq!(prepared.preview().hops[0].input_amount, 10_000);
        assert!(prepared.preview().minimum_output_credit > 0);
        leg.input_mint = None;
        assert!(PoolTradeHint::from_route_leg(&leg).is_err());
    }

    #[tokio::test]
    async fn pumpswap_cached_mainnet_accounts_prepare_both_directions_without_rpc() {
        let (cache, hint, ctx) = fixture();
        for (hint, is_buy, amount) in [(hint, true, 10_000), (reversed(hint), false, 1_000_000_000)]
        {
            let prepared = cache
                .prepare_pumpswap_trade(
                    &[step(hint)],
                    hint.input_mint,
                    hint.output_mint,
                    request(amount),
                    ctx,
                    is_buy,
                )
                .unwrap();
            assert!(prepared.preview().minimum_output_credit > 0);
            assert_eq!(prepared.context().slot, ctx.slot);
            let mut old = mainnet_sim::swap_params(
                mainnet_sim::create_wallet(),
                TradeType::Buy,
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                999,
                9000,
                DexParamEnum::PumpSwapQuoteRoute(Default::default()),
            );
            old.simulate = false;
            old.fixed_output_amount = Some(999);
            old.use_exact_sol_amount = Some(false);
            old.input_token_program = Some(crate::constants::TOKEN_PROGRAM);
            let preview = prepared.preview().clone();
            let params = prepared.apply_to(old).unwrap();
            assert_eq!(params.input_amount, Some(amount));
            assert_eq!(params.input_mint, hint.input_mint);
            assert_eq!(params.output_mint, hint.output_mint);
            assert_eq!(params.fixed_output_amount, None);
            assert_eq!(params.use_exact_sol_amount, Some(true));
            assert!(params.rpc.is_none());
            let ixs = if is_buy {
                PumpSwapInstructionBuilder.build_buy_instructions(&params).await
            } else {
                PumpSwapInstructionBuilder.build_sell_instructions(&params).await
            }
            .unwrap();
            let swap = ixs.iter().find(|ix| ix.program_id == ps::accounts::AMM_PROGRAM).unwrap();
            assert_eq!(
                swap.accounts[17].pubkey,
                solana_sdk::pubkey!("n63vaKSu4PdJs87W2DLSsR5SiSwhHoksJsGPTjjrv13")
            );
            assert_eq!(u64::from_le_bytes(swap.data[8..16].try_into().unwrap()), amount);
            assert_eq!(
                u64::from_le_bytes(swap.data[16..24].try_into().unwrap()),
                preview.minimum_output_credit
            );
        }
    }

    #[test]
    fn pumpswap_cached_accounts_reject_stale_missing_wrong_owners_and_corrupt_vaults() {
        let (cache, hint, ctx) = fixture();
        for key in cache.accounts.keys() {
            for invalid in 0..4 {
                let mut bad = cache.clone();
                match invalid {
                    0 => {
                        bad.accounts.remove(key);
                    }
                    1 => bad.accounts.get_mut(key).unwrap().owner = Pubkey::default(),
                    2 => {
                        bad.accounts.get_mut(key).unwrap().slot =
                            ctx.slot - ctx.maximum_slot_age - 1
                    }
                    _ => bad.accounts.get_mut(key).unwrap().slot = ctx.slot + 1,
                }
                assert!(bad.pumpswap(hint, ctx).is_err(), "key {key}, invalid {invalid}");
            }
        }
        let pool = cache.pumpswap(hint, ctx).unwrap();
        for key in [pool.pool_base_token_account, pool.pool_quote_token_account] {
            for offset in [0, 32, 108] {
                let mut bad = cache.clone();
                bad.accounts.get_mut(&key).unwrap().data[offset] ^= 1;
                assert!(bad.pumpswap(hint, ctx).is_err());
            }
        }
        for key in [hint.pool, ps::accounts::GLOBAL_ACCOUNT, ps::accounts::FEE_CONFIG] {
            let mut bad = cache.clone();
            bad.accounts.get_mut(&key).unwrap().data[0] ^= 1;
            assert!(bad.pumpswap(hint, ctx).is_err());
        }
        let mut cashback = cache.clone();
        // Pool Borsh layout: discriminator + bump/index + six keys + supply + creator + mayhem.
        cashback.accounts.get_mut(&hint.pool).unwrap().data[244] = 1;
        assert!(cashback.pumpswap(hint, ctx).err().unwrap().to_string().contains("cashback"));
    }

    #[test]
    fn pumpswap_cached_mint_fee_is_evaluated_at_the_requested_epoch() {
        let (mut cache, hint, ctx) = fixture();
        let mut mint = vec![0; 278];
        mint[45] = 1;
        mint[165] = 1;
        mint[166..168].copy_from_slice(&1u16.to_le_bytes());
        mint[168..170].copy_from_slice(&108u16.to_le_bytes());
        mint[260..268].copy_from_slice(&(ctx.epoch + 1).to_le_bytes());
        mint[268..276].copy_from_slice(&1_000u64.to_le_bytes());
        mint[276..278].copy_from_slice(&100u16.to_le_bytes());
        cache.accounts.get_mut(&hint.input_mint).unwrap().data = mint;
        assert!(cache.pumpswap(hint, ctx).is_ok());
        let next_epoch = CacheReadContext { epoch: ctx.epoch + 1, ..ctx };
        assert!(cache
            .pumpswap(hint, next_epoch)
            .err()
            .unwrap()
            .to_string()
            .contains("zero transfer fees"));
    }

    #[tokio::test]
    async fn pumpswap_cached_conversion_prepares_sol_buy_and_independent_sell() {
        let (mut cache, pump, ctx) = fixture();
        let (mut conversion, mut market) = super::super::cached_quote::tests::wp_fixture();
        market.input_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        market.output_mint = pump.input_mint;
        let pool = &mut conversion.accounts.get_mut(&market.pool).unwrap().data;
        pool[101..133].copy_from_slice(market.input_mint.as_ref());
        pool[181..213].copy_from_slice(market.output_mint.as_ref());
        for a in conversion.accounts.values_mut() {
            a.slot = ctx.slot;
        }
        cache.accounts.extend(conversion.accounts);
        let mut mint = vec![0; 82];
        mint[45] = 1;
        cache
            .update(
                market.input_mint,
                CachedAccount {
                    owner: crate::constants::TOKEN_PROGRAM,
                    data: mint,
                    slot: ctx.slot,
                    write_version: 0,
                },
            )
            .unwrap();
        let sol = crate::constants::SOL_TOKEN_ACCOUNT;
        let buy_steps = [step(PoolTradeHint { input_mint: sol, ..market }), step(pump)];
        let buy = cache
            .prepare_pumpswap_trade(&buy_steps, sol, pump.output_mint, request(10_000), ctx, true)
            .unwrap();
        assert_eq!(buy.preview().hops[1].input_amount, buy.preview().hops[0].minimum_amount_out);
        assert!(buy.clone().into_sell_params(Hash::default(), GasFeeStrategy::new()).is_err());
        let buy_params = buy.into_buy_params(Hash::default(), GasFeeStrategy::new()).unwrap();
        assert_eq!(buy_params.pay_with, TradeTokenType::SOL);
        assert_eq!(buy_params.slippage_basis_points, Some(100));
        assert_eq!(buy_params.amount, BuyAmount::ExactInput(10_000));
        let sell_steps =
            [step(reversed(pump)), step(PoolTradeHint { output_mint: sol, ..reversed(market) })];
        let sell = cache
            .prepare_pumpswap_trade(
                &sell_steps,
                pump.output_mint,
                sol,
                request(1_000_000_000),
                ctx,
                false,
            )
            .unwrap();
        assert_eq!(sell.preview().hops[1].input_amount, sell.preview().hops[0].minimum_amount_out);
        assert!(sell.clone().into_buy_params(Hash::default(), GasFeeStrategy::new()).is_err());
        let sell_params = sell.into_sell_params(Hash::default(), GasFeeStrategy::new()).unwrap();
        assert_eq!(sell_params.receive_as, TradeTokenType::SOL);
        assert_eq!(sell_params.amount, SellAmount::ExactInput(1_000_000_000));
        assert!(cache
            .prepare_pumpswap_trade(&buy_steps, pump.output_mint, sol, request(1000), ctx, false)
            .is_err());
        assert!(cache
            .prepare_pumpswap_trade(&[step(market)], sol, pump.input_mint, request(1000), ctx, true)
            .is_err());
        let reused = [step(market), step(pump), step(reversed(pump))];
        assert!(cache
            .prepare_pumpswap_trade(&reused, sol, pump.input_mint, request(1000), ctx, true)
            .is_err());
    }
    // Synthetic liquidity curves with real config/mint snapshots. Independent
    // reserves make small/large-order route selection observable in tests.
    fn conversion_pool(
        cache: &mut SubscriptionAccountCache,
        base: Pubkey,
        quote: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
        ctx: CacheReadContext,
    ) -> PoolTradeHint {
        let address = Pubkey::new_unique();
        let base_vault = Pubkey::new_unique();
        let quote_vault = Pubkey::new_unique();
        let mut data = vec![0; 8 + types::POOL_SIZE];
        data[..8].copy_from_slice(&types::POOL_DISCRIMINATOR);
        for (offset, key) in [
            (11, Pubkey::new_unique()),
            (43, base),
            (75, quote),
            (107, Pubkey::new_unique()),
            (139, base_vault),
            (171, quote_vault),
        ] {
            data[offset..offset + 32].copy_from_slice(key.as_ref());
        }
        cache.accounts.insert(
            address,
            CachedAccount {
                owner: ps::accounts::AMM_PROGRAM,
                data,
                slot: ctx.slot,
                write_version: 0,
            },
        );
        for (mint, vault, reserve) in
            [(base, base_vault, base_reserve), (quote, quote_vault, quote_reserve)]
        {
            cache.accounts.entry(mint).or_insert_with(|| {
                let mut data = vec![0; 82];
                data[36..44].copy_from_slice(&1_000_000_000_000u64.to_le_bytes());
                data[45] = 1;
                CachedAccount {
                    owner: crate::constants::TOKEN_PROGRAM,
                    data,
                    slot: ctx.slot,
                    write_version: 0,
                }
            });
            let mut data = vec![0; 165];
            data[..32].copy_from_slice(mint.as_ref());
            data[32..64].copy_from_slice(address.as_ref());
            data[64..72].copy_from_slice(&reserve.to_le_bytes());
            data[108] = 1;
            cache.accounts.insert(
                vault,
                CachedAccount {
                    owner: cache.accounts[&mint].owner,
                    data,
                    slot: ctx.slot,
                    write_version: 0,
                },
            );
        }
        PoolTradeHint { pool: address, input_mint: quote, output_mint: base }
    }

    #[test]
    fn pumpswap_search_compares_protected_outputs_for_amount_and_direction() {
        let (mut cache, pump, ctx) = fixture();
        let sol = crate::constants::SOL_TOKEN_ACCOUNT;
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let thin = conversion_pool(&mut cache, pump.input_mint, wsol, 10_000_000, 5_000_000, ctx);
        let deep =
            conversion_pool(&mut cache, pump.input_mint, wsol, 1_000_000_000, 1_000_000_000, ctx);
        let mut search =
            PumpSwapRouteSearch::new(pump.pool, pump.output_mint, sol, vec![reversed(deep), thin]);
        search.maximum_bridge_price_impact_basis_points = 9_999;
        for (amount, expected) in [(10_000, thin.pool), (100_000_000, deep.pool)] {
            let selected =
                cache.prepare_best_pumpswap_trade(&search, request(amount), ctx, true).unwrap();
            assert_eq!(selected.routes_evaluated, 2);
            assert!(selected.rejected_routes.is_empty());
            assert_eq!(selected.trade.steps()[0].pool.pool, expected);
            let independent: Vec<_> = [thin, deep]
                .into_iter()
                .map(|h| {
                    cache
                        .prepare_pumpswap_trade(
                            &[step(h), step(pump)],
                            sol,
                            pump.output_mint,
                            request(amount),
                            ctx,
                            true,
                        )
                        .unwrap()
                        .preview()
                        .minimum_output_credit
                })
                .collect();
            assert_eq!(
                selected.trade.preview().minimum_output_credit,
                *independent.iter().max().unwrap()
            );
            assert_eq!(
                selected.trade.steps()[1].input_amount,
                Some(selected.trade.preview().hops[0].minimum_amount_out)
            );
        }
        let sell =
            cache.prepare_best_pumpswap_trade(&search, request(1_000_000_000), ctx, false).unwrap();
        assert_eq!(sell.trade.steps()[0].pool.pool, pump.pool);
        assert_eq!(sell.trade.steps()[1].pool.pool, deep.pool);
        assert_eq!(
            sell.trade.into_sell_params(Hash::default(), GasFeeStrategy::new()).unwrap().receive_as,
            TradeTokenType::SOL
        );
    }

    #[test]
    fn bridge_limits_apply_to_conversion_pumpswap_legs_in_both_directions() {
        let (mut cache, pump, ctx) = fixture();
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let bridge =
            conversion_pool(&mut cache, pump.input_mint, wsol, 1_000_000_000, 1_000_000_000, ctx);
        let mut search = PumpSwapRouteSearch::new(pump.pool, pump.output_mint, wsol, vec![bridge]);
        assert_eq!(search.bridge_slippage_basis_points, 100);
        assert_eq!(search.maximum_bridge_price_impact_basis_points, 100);
        let error = cache
            .prepare_best_pumpswap_trade(&search, request(100_000_000), ctx, true)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("Bridge price impact"), "{error}");
        search.maximum_bridge_price_impact_basis_points = 2_000;
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(100_000_000), ctx, true)
            .is_ok());
        for is_buy in [true, false] {
            search.bridge_slippage_basis_points = 50;
            let mut req = request(if is_buy { 10_000 } else { 1_000_000_000 });
            req.slippage_basis_points = 500; // Final PumpSwap tolerance is independent.
            let selected = cache.prepare_best_pumpswap_trade(&search, req, ctx, is_buy).unwrap();
            let idx = if is_buy { 0 } else { 1 };
            let step = selected.trade.steps()[idx];
            let raw = cache
                .quote_exact_in(
                    step.pool,
                    CachedQuoteRequest {
                        amount_in: step.input_amount.unwrap(),
                        slippage_basis_points: 0,
                        ..req
                    },
                    ctx,
                )
                .unwrap();
            assert_eq!(
                selected.trade.preview().hops[idx].minimum_amount_out,
                (u128::from(raw.estimated_net_amount_out) * 9_950 / 10_000) as u64
            );
        }
        for bad in [10_000, u16::MAX] {
            search.bridge_slippage_basis_points = bad;
            assert!(cache
                .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
                .is_err());
        }
    }

    #[test]
    fn pumpswap_search_skips_stale_paths_and_reports_their_cause() {
        let (mut cache, pump, ctx) = fixture();
        let sol = crate::constants::SOL_TOKEN_ACCOUNT;
        let thin = conversion_pool(
            &mut cache,
            pump.input_mint,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            10_000_000,
            5_000_000,
            ctx,
        );
        let deep = conversion_pool(
            &mut cache,
            pump.input_mint,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            1_000_000_000,
            1_000_000_000,
            ctx,
        );
        cache.accounts.get_mut(&thin.pool).unwrap().slot = ctx.slot - ctx.maximum_slot_age - 1;
        let mut search =
            PumpSwapRouteSearch::new(pump.pool, pump.output_mint, sol, vec![thin, deep]);
        let result =
            cache.prepare_best_pumpswap_trade(&search, request(10_000), ctx, true).unwrap();
        assert_eq!(result.trade.steps()[0].pool.pool, deep.pool);
        assert_eq!(result.rejected_routes.len(), 1);
        assert!(result.rejected_routes[0].reason.contains("stale"));
        assert_eq!(result.rejected_routes[0].pools, [thin.pool, pump.pool]);
        search.conversion_pools = vec![thin];
        let error = cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("No executable") && error.contains("stale"));
    }

    #[test]
    fn pumpswap_search_orders_two_hops_and_requotes_the_reverse_path() {
        let (mut cache, pump, ctx) = fixture();
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let usdc = crate::constants::USDC_TOKEN_ACCOUNT;
        let second =
            conversion_pool(&mut cache, pump.input_mint, wsol, 1_000_000_000, 1_000_000_000, ctx);
        let first = conversion_pool(&mut cache, wsol, usdc, 1_000_000_000, 1_000_000_000, ctx);
        let mut search = PumpSwapRouteSearch::new(
            pump.pool,
            pump.output_mint,
            usdc,
            vec![second, reversed(first)],
        );
        for buy in [true, false] {
            let result = cache
                .prepare_best_pumpswap_trade(
                    &search,
                    request(if buy { 10_000 } else { 1_000_000_000 }),
                    ctx,
                    buy,
                )
                .unwrap();
            let pools: Vec<_> = result.trade.steps().iter().map(|s| s.pool.pool).collect();
            assert_eq!(
                pools,
                if buy {
                    vec![first.pool, second.pool, pump.pool]
                } else {
                    vec![pump.pool, second.pool, first.pool]
                }
            );
        }
        search.maximum_conversion_hops = 1;
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string()
            .contains("No conversion path"));
    }

    #[test]
    fn pumpswap_search_is_deterministic_and_never_returns_a_partial_search() {
        let (mut cache, pump, ctx) = fixture();
        let sol = crate::constants::SOL_TOKEN_ACCOUNT;
        let a = conversion_pool(
            &mut cache,
            pump.input_mint,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            1_000_000_000,
            1_000_000_000,
            ctx,
        );
        let b = conversion_pool(
            &mut cache,
            pump.input_mint,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            1_000_000_000,
            1_000_000_000,
            ctx,
        );
        let mut search =
            PumpSwapRouteSearch::new(pump.pool, pump.output_mint, sol, vec![b, a, reversed(a)]);
        let selected =
            cache.prepare_best_pumpswap_trade(&search, request(10_000), ctx, true).unwrap();
        assert_eq!(selected.routes_evaluated, 2);
        assert_eq!(selected.trade.steps()[0].pool.pool, a.pool.min(b.pool));
        search.conversion_pools.reverse();
        assert_eq!(
            cache
                .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
                .unwrap()
                .trade
                .steps()[0]
                .pool
                .pool,
            a.pool.min(b.pool)
        );
        search.maximum_search_nodes = 2; // root and one path; the other must not be silently omitted
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string()
            .contains("budget exhausted"));
        search.maximum_search_nodes = 128;
        search.conversion_pools.push(PoolTradeHint { output_mint: Pubkey::new_unique(), ..a });
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string()
            .contains("Conflicting"));
    }

    #[test]
    fn pumpswap_search_direct_trade_and_invalid_requests() {
        let (cache, pump, ctx) = fixture();
        let mut search =
            PumpSwapRouteSearch::new(pump.pool, pump.output_mint, pump.input_mint, vec![]);
        search.maximum_conversion_hops = 0;
        search.maximum_search_nodes = 1;
        for buy in [true, false] {
            let result = cache
                .prepare_best_pumpswap_trade(
                    &search,
                    request(if buy { 10_000 } else { 1_000_000_000 }),
                    ctx,
                    buy,
                )
                .unwrap();
            assert_eq!(result.routes_evaluated, 1);
            assert_eq!(result.trade.steps().len(), 1);
        }
        assert!(cache.prepare_best_pumpswap_trade(&search, request(0), ctx, true).is_err());
        search.conversion_pools.push(pump);
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string()
            .contains("must not reuse"));
        search.conversion_pools.clear();
        search.asset_mint = search.token_mint;
        assert!(cache.prepare_best_pumpswap_trade(&search, request(10_000), ctx, true).is_err());
    }

    #[test]
    fn pumpswap_search_excludes_cycles_and_target_token_detours() {
        let (mut cache, pump, ctx) = fixture();
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let direct =
            conversion_pool(&mut cache, pump.input_mint, wsol, 1_000_000_000, 1_000_000_000, ctx);
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let hint = |input_mint, output_mint| PoolTradeHint {
            pool: Pubkey::new_unique(),
            input_mint,
            output_mint,
        };
        let mut search = PumpSwapRouteSearch::new(
            pump.pool,
            pump.output_mint,
            crate::constants::SOL_TOKEN_ACCOUNT,
            vec![
                direct,
                hint(wsol, a),
                hint(a, b),
                hint(b, wsol),
                hint(wsol, pump.output_mint),
                hint(pump.output_mint, pump.input_mint),
            ],
        );
        search.maximum_conversion_hops = 3;
        let result =
            cache.prepare_best_pumpswap_trade(&search, request(10_000), ctx, true).unwrap();
        assert_eq!(result.routes_evaluated, 1);
        assert!(result.rejected_routes.is_empty());
        assert_eq!(result.trade.steps()[0].pool.pool, direct.pool);
        for (hops, nodes) in [(4, 128), (2, 0), (2, 4097)] {
            search.maximum_conversion_hops = hops;
            search.maximum_search_nodes = nodes;
            assert!(cache
                .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
                .err()
                .unwrap()
                .to_string()
                .contains("search limits"));
        }
        search.maximum_conversion_hops = 2;
        search.maximum_search_nodes = 128;
        search.conversion_pools = vec![direct; 129];
        assert!(cache
            .prepare_best_pumpswap_trade(&search, request(10_000), ctx, true)
            .err()
            .unwrap()
            .to_string()
            .contains("at most 128"));
    }

    #[test]
    fn pumpswap_search_can_trade_the_quote_side_of_the_selected_pool() {
        let (cache, pump, ctx) = fixture();
        let search = PumpSwapRouteSearch::new(pump.pool, pump.input_mint, pump.output_mint, vec![]);
        let result =
            cache.prepare_best_pumpswap_trade(&search, request(1_000_000_000), ctx, true).unwrap();
        assert_eq!(result.trade.steps()[0].pool.input_mint, pump.output_mint);
        assert_eq!(result.trade.steps()[0].pool.output_mint, pump.input_mint);
        let explicit = cache
            .prepare_pumpswap_trade(
                &[step(reversed(pump))],
                pump.output_mint,
                pump.input_mint,
                request(1_000_000_000),
                ctx,
                true,
            )
            .unwrap();
        assert_eq!(
            result.trade.preview().minimum_output_credit,
            explicit.preview().minimum_output_credit
        );
    }
}
