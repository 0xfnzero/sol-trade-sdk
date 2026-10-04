//! Offline exact-input quote preparation from subscription snapshots.
use super::{
    CacheReadContext, MeteoraDlmmParams, PoolTradeHint, RaydiumClmmParams, StonkFunQuoteHop,
    StonkFunQuoteVenue, SubscriptionAccountCache, TokenTransferFee, WhirlpoolParams,
};
use crate::instruction::utils::{meteora_dlmm as dl, raydium_clmm as cl, whirlpool as wp};
use anyhow::{anyhow, bail, ensure, Result};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Copy, Debug)]
pub struct CachedQuoteRequest {
    pub amount_in: u64,
    pub slippage_basis_points: u16,
    /// Chain Clock time from a subscription, not a timestamp from the copied trade.
    pub unix_timestamp: u64,
    /// Bounded work and transaction size. Fail rather than silently truncate a route.
    pub maximum_arrays: usize,
}
#[derive(Clone)]
pub struct CachedSwapQuote {
    pub hop: StonkFunQuoteHop,
    pub estimated_net_amount_out: u64,
    pub minimum_net_amount_out: u64,
    pub state_slot: u64,
    pub epoch: u64,
}
fn read<const N: usize>(d: &[u8], o: usize) -> Result<[u8; N]> {
    d.get(o..o + N)
        .ok_or_else(|| anyhow!("Truncated liquidity account at {o}"))?
        .try_into()
        .map_err(Into::into)
}
fn pk(d: &[u8], o: usize) -> Result<Pubkey> {
    Ok(Pubkey::new_from_array(read(d, o)?))
}
fn u16le(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(read(d, o)?))
}
fn u32le(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(read(d, o)?))
}
fn i32le(d: &[u8], o: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(read(d, o)?))
}
fn u64le(d: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(read(d, o)?))
}
fn u128le(d: &[u8], o: usize) -> Result<u128> {
    Ok(u128::from_le_bytes(read(d, o)?))
}
fn i128le(d: &[u8], o: usize) -> Result<i128> {
    Ok(i128::from_le_bytes(read(d, o)?))
}
fn finish(
    h: PoolTradeHint,
    venue: StonkFunQuoteVenue,
    q: CachedQuoteRequest,
    ctx: CacheReadContext,
    net: u64,
    fee: TokenTransferFee,
) -> Result<CachedSwapQuote> {
    let minimum =
        ((net as u128) * (10_000 - u64::from(q.slippage_basis_points)) as u128 / 10_000) as u64;
    ensure!(minimum > 0, "Quote has zero protected output");
    Ok(CachedSwapQuote {
        hop: StonkFunQuoteHop::exact_in(venue, h.input_mint, h.output_mint, minimum)
            .with_input_amount(q.amount_in)
            .with_output_transfer_fee(fee),
        estimated_net_amount_out: net,
        minimum_net_amount_out: minimum,
        state_slot: ctx.slot,
        epoch: ctx.epoch,
    })
}
impl SubscriptionAccountCache {
    /// Quotes and produces fresh execution accounts for a quote-conversion pool.
    /// Does not discover pools, fetch state, copy wallets, or sign/send transactions.
    pub fn quote_exact_in(
        &self,
        hint: PoolTradeHint,
        q: CachedQuoteRequest,
        ctx: CacheReadContext,
    ) -> Result<CachedSwapQuote> {
        ensure!(
            q.amount_in > 0
                && q.slippage_basis_points < 10_000
                && (1..=32).contains(&q.maximum_arrays),
            "Invalid cached quote request"
        );
        let owner = self
            .accounts
            .get(&hint.pool)
            .ok_or_else(|| anyhow!("Missing cached pool: {}", hint.pool))?
            .owner;
        if owner == wp::PROGRAM_ID {
            return self.quote_whirlpool(hint, q, ctx);
        }
        if owner == cl::PROGRAM_ID {
            return self.quote_clmm(hint, q, ctx);
        }
        if owner == dl::PROGRAM_ID {
            return self.quote_dlmm(hint, q, ctx);
        }
        if owner == crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4 {
            let p = self.amm_v4(hint, ctx)?;
            let data = self.get(hint.pool, owner, ctx)?;
            let state = crate::instruction::utils::raydium_amm_v4_types::amm_info_decode(data)
                .ok_or_else(|| anyhow!("Invalid AMM state"))?;
            ensure!(
                state.status != 7 || q.unix_timestamp >= state.out_put.pool_open_time,
                "AMM v4 pool has not opened"
            );
            let quote = crate::utils::calc::raydium_amm_v4::compute_swap_amount_for_pool(
                &p,
                hint.input_mint == p.coin_mint,
                q.amount_in,
                u64::from(q.slippage_basis_points),
            )?;
            return finish(
                hint,
                StonkFunQuoteVenue::RaydiumAmmV4(p),
                q,
                ctx,
                quote.amount_out,
                TokenTransferFee::default(),
            );
        }
        if owner == crate::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM {
            let p = self.cpmm_at(hint, ctx, q.unix_timestamp)?;
            let base_in = hint.input_mint == p.base_mint;
            let quote = crate::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
                &p,
                base_in,
                q.amount_in,
                u64::from(q.slippage_basis_points),
            )?;
            let fee = if base_in { p.quote_transfer_fee } else { p.base_transfer_fee };
            return finish(hint, StonkFunQuoteVenue::RaydiumCpmm(p), q, ctx, quote.amount_out, fee);
        }
        bail!("Unsupported cached quote pool owner: {owner}")
    }
    /// Current SwapBaseInV2 accounts/reserves; no OpenBook state is used by v2.
    pub fn amm_v4(
        &self,
        h: PoolTradeHint,
        ctx: CacheReadContext,
    ) -> Result<super::RaydiumAmmV4Params> {
        use crate::instruction::utils::{raydium_amm_v4 as amm, raydium_amm_v4_types as types};
        let d = self.get(h.pool, amm::accounts::RAYDIUM_AMM_V4, ctx)?;
        let p = types::amm_info_decode(d).ok_or_else(|| anyhow!("Invalid AMM v4 account"))?;
        h.matches(p.coin_mint, p.pc_mint)?;
        ensure!(matches!(p.status, 1 | 6 | 7), "AMM v4 swap disabled");
        self.mint(p.coin_mint, Some(crate::constants::TOKEN_PROGRAM), ctx)?;
        self.mint(p.pc_mint, Some(crate::constants::TOKEN_PROGRAM), ctx)?;
        let balance = |v: Pubkey, m: Pubkey, pnl: u64| -> Result<u64> {
            let d = self.get(v, crate::constants::TOKEN_PROGRAM, ctx)?;
            ensure!(d.len() >= 165 && pk(d, 0)? == m && d[108] == 1, "Invalid AMM v4 vault");
            u64le(d, 64)?.checked_sub(pnl).ok_or_else(|| anyhow!("AMM v4 PnL exceeds vault"))
        };
        let mut params = super::RaydiumAmmV4Params::new(
            h.pool,
            p.coin_mint,
            p.pc_mint,
            p.token_coin,
            p.token_pc,
            balance(p.token_coin, p.coin_mint, p.out_put.need_take_pnl_coin)?,
            balance(p.token_pc, p.pc_mint, p.out_put.need_take_pnl_pc)?,
        );
        params.swap_fee_numerator = p.fees.swap_fee_numerator;
        params.swap_fee_denominator = p.fees.swap_fee_denominator;
        ensure!(
            params.swap_fee_denominator > 0
                && params.swap_fee_numerator < params.swap_fee_denominator,
            "Invalid cached AMM swap fees"
        );
        Ok(params)
    }
    fn quote_whirlpool(
        &self,
        h: PoolTradeHint,
        q: CachedQuoteRequest,
        ctx: CacheReadContext,
    ) -> Result<CachedSwapQuote> {
        use orca_whirlpools_core as core;
        let d = self.get(h.pool, wp::PROGRAM_ID, ctx)?;
        ensure!(d.len() >= 653, "Truncated Whirlpool state");
        let state = wp::decode_whirlpool(d)?;
        h.matches(state.token_mint_a, state.token_mint_b)?;
        ensure!(state.tick_spacing > 0, "Zero Whirlpool spacing");
        let direction = h.input_mint == state.token_mint_a;
        let (program_a, fee_a) = self.mint(state.token_mint_a, None, ctx)?;
        let (program_b, fee_b) = self.mint(state.token_mint_b, None, ctx)?;
        let pool = core::WhirlpoolFacade {
            fee_tier_index_seed: read(d, 43)?,
            tick_spacing: state.tick_spacing,
            fee_rate: u16le(d, 45)?,
            protocol_fee_rate: u16le(d, 47)?,
            liquidity: u128le(d, 49)?,
            sqrt_price: u128le(d, 65)?,
            tick_current_index: state.tick_current_index,
            ..Default::default()
        };
        ensure!(
            (core::MIN_SQRT_PRICE..=core::MAX_SQRT_PRICE).contains(&pool.sqrt_price)
                && (core::MIN_TICK_INDEX..=core::MAX_TICK_INDEX).contains(&pool.tick_current_index),
            "Invalid Whirlpool price/tick"
        );
        let oracle = if pool.is_initialized_with_adaptive_fee() {
            let od = self.get(wp::oracle(&h.pool), wp::PROGRAM_ID, ctx)?;
            ensure!(
                od.get(..8) == Some(&[139, 194, 131, 179, 140, 179, 229, 244])
                    && pk(od, 8)? == h.pool,
                "Invalid Whirlpool oracle"
            );
            ensure!(
                u16le(od, 62)? > 0 && u16le(od, 52)? <= 10_000,
                "Invalid Whirlpool adaptive fee constants"
            );
            let enabled = u64le(od, 40)?;
            ensure!(q.unix_timestamp >= enabled, "Whirlpool trade not enabled yet");
            Some(core::OracleFacade {
                trade_enable_timestamp: enabled,
                adaptive_fee_constants: core::AdaptiveFeeConstantsFacade {
                    filter_period: u16le(od, 48)?,
                    decay_period: u16le(od, 50)?,
                    reduction_factor: u16le(od, 52)?,
                    adaptive_fee_control_factor: u32le(od, 54)?,
                    max_volatility_accumulator: u32le(od, 58)?,
                    tick_group_size: u16le(od, 62)?,
                    major_swap_threshold_ticks: u16le(od, 64)?,
                },
                adaptive_fee_variables: core::AdaptiveFeeVariablesFacade {
                    last_reference_update_timestamp: u64le(od, 82)?,
                    last_major_swap_timestamp: u64le(od, 90)?,
                    volatility_reference: u32le(od, 98)?,
                    tick_group_index_reference: i32le(od, 102)?,
                    volatility_accumulator: u32le(od, 106)?,
                },
            })
        } else {
            None
        };
        let shifted = state
            .tick_current_index
            .checked_add(if direction { 0 } else { i32::from(state.tick_spacing) })
            .ok_or_else(|| anyhow!("Whirlpool tick overflow"))?;
        let mut arrays = Vec::new();
        let mut addresses = Vec::new();
        let input_fee = if direction { fee_a } else { fee_b };
        let amount = q
            .amount_in
            .checked_sub(input_fee.calculate(q.amount_in))
            .ok_or_else(|| anyhow!("Input transfer fee exceeds amount"))?;
        for index in 0..q.maximum_arrays.min(6) {
            let start = wp::get_start_tick_index(
                shifted,
                state.tick_spacing,
                if direction { -(index as i32) } else { index as i32 },
            );
            let address = wp::tick_array_pda(&h.pool, start);
            let data = self.get(address, wp::PROGRAM_ID, ctx)?;
            let arr = decode_orca_ticks(data, h.pool, start)?;
            arrays.push(Some(arr));
            addresses.push(address);
            let mut sequence = [None; 6];
            sequence[..arrays.len()].copy_from_slice(&arrays);
            let seq = core::TickArraySequence::new(sequence, state.tick_spacing)
                .map_err(|e| anyhow!("Whirlpool arrays: {e:?}"))?;
            match core::compute_swap(
                amount,
                0,
                pool,
                seq,
                direction,
                true,
                q.unix_timestamp,
                oracle.map(Into::into),
            ) {
                Ok(result) => {
                    let (consumed, gross) = if direction {
                        (result.token_a, result.token_b)
                    } else {
                        (result.token_b, result.token_a)
                    };
                    if consumed != amount {
                        continue;
                    }
                    let output_fee = if direction { fee_b } else { fee_a };
                    while addresses.len() < 3 {
                        addresses.push(*addresses.last().unwrap());
                    }
                    let p = WhirlpoolParams::new(
                        h.pool,
                        state.token_mint_a,
                        state.token_mint_b,
                        state.token_vault_a,
                        state.token_vault_b,
                        program_a,
                        program_b,
                        addresses,
                    );
                    return finish(
                        h,
                        StonkFunQuoteVenue::OrcaWhirlpool(p),
                        q,
                        ctx,
                        gross - output_fee.calculate(gross),
                        output_fee,
                    );
                }
                Err(_) if index + 1 < q.maximum_arrays.min(6) => continue,
                Err(e) => bail!("Whirlpool quote requires more arrays or invalid state: {e:?}"),
            }
        }
        bail!("Whirlpool quote exceeds loaded array budget")
    }
    fn quote_clmm(
        &self,
        h: PoolTradeHint,
        q: CachedQuoteRequest,
        ctx: CacheReadContext,
    ) -> Result<CachedSwapQuote> {
        use solana_clmm_raydium as math;
        let d = self.get(h.pool, cl::PROGRAM_ID, ctx)?;
        ensure!(d.len() >= 1544, "Truncated CLMM state");
        let p = cl::decode_pool_state(d)?;
        h.matches(p.token_mint_0, p.token_mint_1)?;
        ensure!(p.tick_spacing > 0 && d[389] & 16 == 0, "CLMM swap disabled or zero spacing");
        ensure!(q.unix_timestamp > u64le(d, 1080)?, "CLMM pool has not opened");
        let config = self.get(p.amm_config, cl::PROGRAM_ID, ctx)?;
        ensure!(
            config.get(..8) == Some(&[218, 244, 33, 104, 203, 203, 43, 111])
                && u16le(config, 51)? == p.tick_spacing,
            "Invalid CLMM config"
        );
        let fee_rate = u32le(config, 47)?;
        ensure!(fee_rate < 1_000_000, "Invalid CLMM fee rate");
        let (program_0, fee_0) = self.mint(p.token_mint_0, None, ctx)?;
        let (program_1, fee_1) = self.mint(p.token_mint_1, None, ctx)?;
        let direction = h.input_mint == p.token_mint_0;
        let input_fee = if direction { fee_0 } else { fee_1 };
        let output_fee = if direction { fee_1 } else { fee_0 };
        let amount = q.amount_in - input_fee.calculate(q.amount_in);
        ensure!(amount > 0, "Zero CLMM input after transfer fee");
        ensure!(
            (math::MIN_SQRT_PRICE_X64..=math::MAX_SQRT_PRICE_X64).contains(&u128le(d, 253)?)
                && (math::MIN_TICK..=math::MAX_TICK).contains(&p.tick_current),
            "Invalid CLMM price/tick"
        );
        let pool = math::SwapPool {
            sqrt_price_x64: u128le(d, 253)?,
            liquidity: u128le(d, 237)?,
            tick_current: p.tick_current,
            tick_spacing: p.tick_spacing,
            fee_rate_pips: fee_rate,
        };
        let start = cl::get_array_start_index(p.tick_current, p.tick_spacing);
        let step = cl::tick_count(p.tick_spacing);
        let mut ticks = Vec::new();
        let mut arrays = Vec::new();
        let mut bitmap_extension = None;
        // The base bitmap covers [-512,511] arrays. The extension is required
        // only when traversing outside it; never assume a missing bit is empty.
        for offset in 0..2048 {
            let next = start
                .checked_add(if direction { -(offset * step) } else { offset * step })
                .ok_or_else(|| anyhow!("CLMM array overflow"))?;
            if next > math::MAX_TICK || next + step <= math::MIN_TICK {
                break;
            }
            let index = next / step;
            let initialized = if (-512..512).contains(&index) {
                bit(d, 904, index + 512)?
            } else {
                let k = cl::tick_array_bitmap_extension(&h.pool);
                let ed = self.get(k, cl::PROGRAM_ID, ctx)?;
                ensure!(
                    ed.get(..8) == Some(&[60, 150, 36, 219, 97, 128, 139, 153])
                        && ed.len() >= 1832
                        && pk(ed, 8)? == h.pool,
                    "CLMM bitmap pool mismatch"
                );
                bitmap_extension = Some(k);
                // Extension layout: pool_id, positive[14][8], negative[14][8].
                let pos = if index >= 512 {
                    index - 512
                } else {
                    let distance = -index - 513;
                    (distance / 512) * 512 + 511 - distance % 512
                };
                ensure!((0..7168).contains(&pos), "CLMM extension bitmap index out of range");
                bit(ed, if index >= 512 { 40 } else { 936 }, pos)?
            };
            if !initialized {
                continue;
            }
            let address = cl::tick_array_pda(&h.pool, next);
            let td = self.get(address, cl::PROGRAM_ID, ctx)?;
            ensure!(
                td.len() >= 10240
                    && td.get(..8) == Some(&[192, 155, 85, 205, 49, 249, 129, 42])
                    && pk(td, 8)? == h.pool
                    && i32le(td, 40)? == next,
                "Invalid CLMM tick array"
            );
            for i in 0..60 {
                let o = 44 + i * 168;
                let tick = i32le(td, o)?;
                if u128le(td, o + 20)? > 0 || u64le(td, o + 124)? > 0 || u64le(td, o + 132)? > 0 {
                    ensure!(
                        tick == next + (i as i32) * i32::from(p.tick_spacing),
                        "CLMM tick index mismatch"
                    );
                    // Order-aware/dynamic traversal is handled by the local orchestrator.
                    ticks.push(ClmmTick {
                        tick,
                        liquidity_net: i128le(td, o + 4)?,
                        liquidity_gross: u128le(td, o + 20)?,
                        orders: u64le(td, o + 124)?,
                        partial_orders: u64le(td, o + 132)?,
                    });
                }
            }
            ticks.sort_by_key(|t| t.tick);
            arrays.push(address);
            let boundary = if direction {
                next.max(math::MIN_TICK)
            } else {
                (next + step - 1).min(math::MAX_TICK)
            };
            let limit =
                math::get_sqrt_price_at_tick(boundary).map_err(|e| anyhow!("CLMM limit: {e:?}"))?;
            if (direction && limit >= pool.sqrt_price_x64)
                || (!direction && limit <= pool.sqrt_price_x64)
            {
                continue;
            }
            let result = clmm_swap(
                &pool,
                &ticks,
                amount,
                limit,
                d[390],
                &d[1096..1176],
                q.unix_timestamp,
                direction,
            )?;
            if result.0 == amount {
                let mut params = RaydiumClmmParams::new(
                    p.amm_config,
                    h.pool,
                    p.observation_key,
                    p.token_mint_0,
                    p.token_mint_1,
                    p.token_vault_0,
                    p.token_vault_1,
                    program_0,
                    program_1,
                    arrays,
                );
                params.tick_array_bitmap_extension = bitmap_extension;
                return finish(
                    h,
                    StonkFunQuoteVenue::RaydiumClmm(params),
                    q,
                    ctx,
                    result.1 - output_fee.calculate(result.1),
                    output_fee,
                );
            }
            ensure!(arrays.len() < q.maximum_arrays, "CLMM quote exceeds array budget");
        }
        bail!("Insufficient CLMM liquidity in supplied snapshot")
    }
    fn quote_dlmm(
        &self,
        h: PoolTradeHint,
        q: CachedQuoteRequest,
        ctx: CacheReadContext,
    ) -> Result<CachedSwapQuote> {
        let data = self.get(h.pool, dl::PROGRAM_ID, ctx)?;
        ensure!(data.len() >= 904 && data[82] == 0, "DLMM disabled or truncated state");
        let p = dl::decode_lb_pair(data)?;
        h.matches(p.token_x_mint, p.token_y_mint)?;
        ensure!(
            p.bin_step > 0 && data[34] <= 18 && u16le(data, 14)? <= 10_000 && data[36] <= 1,
            "Invalid DLMM fee configuration"
        );
        ensure!(data[86] <= 1, "Unknown DLMM activation type");
        let activation = u64le(data, 816)?;
        ensure!(
            if data[86] == 0 { ctx.slot >= activation } else { q.unix_timestamp >= activation },
            "DLMM pool not activated yet"
        );
        let (program_x, fee_x) = self.mint(p.token_x_mint, None, ctx)?;
        let (program_y, fee_y) = self.mint(p.token_y_mint, None, ctx)?;
        let direction = h.input_mint == p.token_x_mint;
        let input_fee = if direction { fee_x } else { fee_y };
        let output_fee = if direction { fee_y } else { fee_x };
        let amount = q.amount_in - input_fee.calculate(q.amount_in);
        ensure!(amount > 0, "Zero DLMM input after fee");
        let (_, variable, _, _, _, _) =
            meteora_dlmm::decode_lb_pair(data).map_err(|e| anyhow!("DLMM fee state: {e:?}"))?;
        ensure!(
            variable.last_update_timestamp >= 0
                && u64::try_from(variable.last_update_timestamp)? <= q.unix_timestamp
                && (-443636..=443636).contains(&p.active_id),
            "Invalid DLMM timestamp/active bin"
        );
        let base = dl::bin_id_to_bin_array_index(p.active_id);
        let mut arrays = Vec::new();
        let mut bytes = Vec::new();
        let mut bitmap_extension = None;
        for offset in 0..2048i64 {
            let index = base + if direction { -offset } else { offset };
            if index * 70 > 443636 || (index + 1) * 70 <= -443636 {
                break;
            }
            let initialized = if (-512..512).contains(&index) {
                bit(data, 584, (index + 512) as i32)?
            } else {
                let k = dl::bitmap_extension_pda(&h.pool);
                let d = self.get(k, dl::PROGRAM_ID, ctx)?;
                ensure!(
                    d.get(..8) == Some(&[80, 111, 124, 113, 55, 237, 18, 5])
                        && d.len() >= 1576
                        && pk(d, 8)? == h.pool,
                    "DLMM bitmap pool mismatch"
                );
                bitmap_extension = Some(k);
                let pos = if index >= 512 { index - 512 } else { -index - 513 };
                ensure!((0..6144).contains(&pos), "DLMM extension bitmap index out of range");
                bit(d, if index >= 512 { 40 } else { 808 }, pos as i32)?
            };
            let address = dl::bin_array_pda(&h.pool, index);
            if initialized {
                let bd = self.get(address, dl::PROGRAM_ID, ctx)?;
                ensure!(
                    bd.len() >= 10136
                        && bd.get(..8) == Some(&[92, 142, 92, 220, 5, 148, 70, 181])
                        && i64::from_le_bytes(read(bd, 8)?) == index
                        && pk(bd, 24)? == h.pool,
                    "Invalid DLMM bin array"
                );
                bytes.push(bd.to_vec());
                arrays.push(address);
            } else {
                // Known-empty bitmap interval is represented explicitly for the
                // pure engine, never as missing state and never as an ix account.
                let mut empty = vec![0; 10136];
                empty[8..16].copy_from_slice(&index.to_le_bytes());
                empty[24..56].copy_from_slice(h.pool.as_ref());
                bytes.push(empty);
            }
            let pool = meteora_dlmm::PoolState::from_accounts(
                data,
                &bytes,
                0,
                0,
                Some(&h.pool.to_bytes()),
                false,
            )
            .map_err(|e| anyhow!("DLMM state: {e:?}"))?;
            let support_orders = match data[35] {
                0 => pk(data, 264)? == Pubkey::default() && pk(data, 408)? == Pubkey::default(),
                1 => false,
                2 => true,
                _ => bail!("Unknown DLMM function type"),
            };
            let result = meteora_dlmm::quote(
                &pool,
                u128::from(amount),
                direction,
                i64::try_from(q.unix_timestamp)?,
                support_orders,
                true,
                None,
                None,
            );
            if let Ok(result) = result {
                if result.complete && result.remaining_in == 0 && !arrays.is_empty() {
                    let gross = u64::try_from(result.gross_amount_out)?;
                    let mut params = MeteoraDlmmParams::new(
                        h.pool,
                        p.reserve_x,
                        p.reserve_y,
                        p.token_x_mint,
                        p.token_y_mint,
                        p.oracle,
                        program_x,
                        program_y,
                        arrays,
                    );
                    params.bitmap_extension = bitmap_extension;
                    return finish(
                        h,
                        StonkFunQuoteVenue::MeteoraDlmm(params),
                        q,
                        ctx,
                        gross - output_fee.calculate(gross),
                        output_fee,
                    );
                }
            }
            ensure!(arrays.len() < q.maximum_arrays, "DLMM quote exceeds array budget");
        }
        bail!("Insufficient DLMM liquidity in supplied snapshot")
    }
}
fn bit(d: &[u8], offset: usize, index: i32) -> Result<bool> {
    ensure!(index >= 0, "Negative bitmap index");
    let index = index as usize;
    Ok(*d.get(offset + index / 8).ok_or_else(|| anyhow!("Truncated liquidity bitmap"))?
        & (1 << (index % 8))
        != 0)
}
fn decode_orca_ticks(
    d: &[u8],
    pool: Pubkey,
    start: i32,
) -> Result<orca_whirlpools_core::TickArrayFacade> {
    use orca_whirlpools_core::{TickArrayFacade, TickFacade};
    ensure!(i32le(d, 8)? == start, "Whirlpool array start mismatch");
    let fixed = d.get(..8) == Some(&[69, 97, 189, 190, 110, 7, 66, 187]);
    let dynamic = d.get(..8) == Some(&[17, 216, 246, 142, 225, 199, 218, 56]);
    ensure!(fixed || dynamic, "Unknown Whirlpool array discriminator");
    ensure!(pk(d, if fixed { 9956 } else { 12 })? == pool, "Whirlpool array pool mismatch");
    let mut ticks = [TickFacade::default(); 88];
    let mut o = if fixed { 12 } else { 60 };
    for (i, tick) in ticks.iter_mut().enumerate() {
        let initialized = *d.get(o).ok_or_else(|| anyhow!("Truncated Whirlpool tick"))?;
        ensure!(initialized <= 1, "Invalid Whirlpool tick tag");
        o += 1;
        if dynamic {
            ensure!(bit(d, 44, i as i32)? == (initialized == 1), "Whirlpool tick bitmap mismatch");
        }
        if fixed || initialized == 1 {
            *tick = TickFacade {
                initialized: initialized == 1,
                liquidity_net: i128le(d, o)?,
                liquidity_gross: u128le(d, o + 16)?,
                ..Default::default()
            };
            o += 112;
        }
    }
    Ok(TickArrayFacade { start_tick_index: start, ticks })
}
#[derive(Clone, Copy)]
struct ClmmTick {
    tick: i32,
    liquidity_net: i128,
    liquidity_gross: u128,
    orders: u64,
    partial_orders: u64,
}

/// Exact-in traversal using upstream integer swap steps, with current CLMM
/// fee-side, dynamic spacing groups and boundary limit-order semantics.
fn clmm_swap(
    pool: &solana_clmm_raydium::SwapPool,
    ticks: &[ClmmTick],
    amount: u64,
    limit: u128,
    fee_on: u8,
    dynamic: &[u8],
    timestamp: u64,
    down: bool,
) -> Result<(u64, u64)> {
    use math::big_num::U256;
    use solana_clmm_raydium as math;
    ensure!(fee_on <= 2, "Unknown CLMM fee side");
    let input_fee = fee_on == 0 || (fee_on == 1 && down) || (fee_on == 2 && !down);
    let enabled = dynamic.iter().any(|b| *b != 0);
    let mut group = pool.tick_current.div_euclid(i32::from(pool.tick_spacing));
    let mut reference = i32le(dynamic, 14)?;
    let mut volatility_reference = u32le(dynamic, 18)?;
    let mut volatility = u32le(dynamic, 22)?;
    let maximum = u32le(dynamic, 10)?;
    let control = u32le(dynamic, 6)?;
    if enabled {
        ensure!(
            u16le(dynamic, 0)? > 0
                && u16le(dynamic, 2)? > u16le(dynamic, 0)?
                && (1..10_000).contains(&u16le(dynamic, 4)?)
                && (1..100_000).contains(&control)
                && u64::from(maximum) * u64::from(pool.tick_spacing) <= u64::from(u32::MAX),
            "Invalid CLMM dynamic fee params"
        );
        let elapsed = timestamp.saturating_sub(u64le(dynamic, 26)?);
        if elapsed >= u64::from(u16le(dynamic, 0)?) {
            reference = group;
            volatility_reference = if elapsed < u64::from(u16le(dynamic, 2)?) {
                (u64::from(volatility) * u64::from(u16le(dynamic, 4)?) / 10_000) as u32
            } else {
                0
            };
        }
    }
    let mut current = pool.sqrt_price_x64;
    let mut tick = pool.tick_current;
    let mut liquidity = pool.liquidity;
    let mut remaining = amount;
    let mut output = 0u64;
    // Local order balances: every quote begins with the subscribed snapshot.
    let mut orders: Vec<u64> = ticks
        .iter()
        .map(|t| {
            t.orders.checked_add(t.partial_orders).ok_or_else(|| anyhow!("CLMM order overflow"))
        })
        .collect::<Result<_>>()?;
    for _ in 0..8192 {
        if remaining == 0 || current == limit {
            return Ok((amount - remaining, output));
        }
        let next = ticks
            .iter()
            .enumerate()
            .filter(|(i, t)| t.liquidity_gross > 0 || orders[*i] > 0)
            .filter(|(_, t)| if down { t.tick <= tick } else { t.tick > tick })
            .min_by_key(|(_, t)| if down { -t.tick } else { t.tick });
        let next_tick =
            next.map(|(_, t)| t.tick).unwrap_or(if down { math::MIN_TICK } else { math::MAX_TICK });
        let next_price = math::get_sqrt_price_at_tick(next_tick)
            .map_err(|e| anyhow!("CLMM tick price: {e:?}"))?;
        let target = if down { next_price.max(limit) } else { next_price.min(limit) };
        let mut fee = pool.fee_rate_pips;
        let mut skipped = true;
        let mut bound = target;
        if enabled {
            volatility = (u64::from(volatility_reference)
                + u64::from(reference.abs_diff(group)) * 10_000)
                .min(u64::from(maximum)) as u32;
            let crossed = u128::from(volatility) * u128::from(pool.tick_spacing);
            let extra = (u128::from(control) * crossed * crossed).div_ceil(10_000_000_000_000);
            fee = (u128::from(fee) + extra).min(100_000) as u32;
            skipped = liquidity == 0 || volatility == maximum;
            if !skipped {
                let boundary = (if down { group } else { group + 1 })
                    .saturating_mul(i32::from(pool.tick_spacing))
                    .clamp(math::MIN_TICK, math::MAX_TICK);
                let price = math::get_sqrt_price_at_tick(boundary)
                    .map_err(|e| anyhow!("CLMM dynamic boundary: {e:?}"))?;
                bound = if down { target.max(price) } else { target.min(price) };
            }
        }
        let old_price = current;
        if current != bound {
            let step = math::compute_swap_step(
                current,
                bound,
                liquidity,
                remaining,
                if input_fee { fee } else { 0 },
                true,
                down,
            )
            .map_err(|e| anyhow!("CLMM swap step: {e:?}"))?;
            let consumed = step
                .amount_in
                .checked_add(if input_fee { step.fee_amount } else { 0 })
                .ok_or_else(|| anyhow!("CLMM input overflow"))?;
            remaining = remaining
                .checked_sub(consumed)
                .ok_or_else(|| anyhow!("CLMM input exceeds remaining"))?;
            let net = if input_fee {
                step.amount_out
            } else {
                step.amount_out
                    - ((u128::from(step.amount_out) * u128::from(fee)).div_ceil(1_000_000) as u64)
            };
            output = output.checked_add(net).ok_or_else(|| anyhow!("CLMM output overflow"))?;
            current = step.sqrt_price_next_x64;
        }
        if current == next_price {
            if let Some((index, t)) = next {
                if orders[index] > 0 && remaining > 0 {
                    // Price Q64.64 is rounded down for token0 input, up for token1 input.
                    let square = U256::from(current) * U256::from(current);
                    let mask = (U256::one() << 64) - U256::one();
                    let price = (square >> 64)
                        + if !down && square & mask != U256::zero() {
                            U256::one()
                        } else {
                            U256::zero()
                        };
                    ensure!(price > U256::zero(), "Zero CLMM order price");
                    let fee_amount = if input_fee {
                        (u128::from(remaining) * u128::from(fee)).div_ceil(1_000_000) as u64
                    } else {
                        0
                    };
                    let available = remaining - fee_amount;
                    let matched = if down {
                        U256::from(available) * price / (U256::one() << 64)
                    } else {
                        (U256::from(available) << 64) / price
                    };
                    let gross = matched.min(U256::from(orders[index])).as_u64();
                    let (consumed, fee_amount) = if matched > U256::from(orders[index]) {
                        let (num, den) = if down {
                            (U256::from(gross) << 64, price)
                        } else {
                            (U256::from(gross) * price, U256::one() << 64)
                        };
                        let input = (num + den - U256::one()) / den;
                        ensure!(input <= U256::from(u64::MAX), "CLMM order input overflow");
                        let input = input.as_u64();
                        let fee = if input_fee {
                            (u128::from(input) * u128::from(fee))
                                .div_ceil(u128::from(1_000_000 - fee))
                                as u64
                        } else {
                            0
                        };
                        (input, fee)
                    } else {
                        (available, fee_amount)
                    };
                    remaining = remaining
                        .checked_sub(
                            consumed
                                .checked_add(fee_amount)
                                .ok_or_else(|| anyhow!("CLMM order fee overflow"))?,
                        )
                        .ok_or_else(|| anyhow!("CLMM order overconsumes input"))?;
                    orders[index] -= gross;
                    let net = if input_fee {
                        gross
                    } else {
                        gross - (u128::from(gross) * u128::from(fee)).div_ceil(1_000_000) as u64
                    };
                    output = output
                        .checked_add(net)
                        .ok_or_else(|| anyhow!("CLMM order output overflow"))?;
                }
                if t.liquidity_gross > 0 && orders[index] == 0 {
                    liquidity = math::cross(liquidity, t.liquidity_net, down)
                        .map_err(|e| anyhow!("CLMM liquidity crossing: {e:?}"))?;
                }
                tick = if (down && orders[index] == 0) || (!down && orders[index] > 0) {
                    next_tick - 1
                } else {
                    next_tick
                };
            } else {
                break;
            }
        } else if current != old_price {
            tick = math::get_tick_at_sqrt_price(current)
                .map_err(|e| anyhow!("CLMM current tick: {e:?}"))?;
        }
        if enabled {
            if skipped {
                let boundary_tick = if current == next_price { next_tick } else { tick };
                group = boundary_tick.div_euclid(i32::from(pool.tick_spacing));
                if !down && boundary_tick % i32::from(pool.tick_spacing) == 0 {
                    group -= 1;
                }
            }
            group += if down { -1 } else { 1 };
        }
    }
    ensure!(remaining == 0 || current == limit, "CLMM quote iteration budget exhausted");
    Ok((amount - remaining, output))
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::CachedAccount;
    use super::*;
    fn put(c: &mut SubscriptionAccountCache, k: Pubkey, owner: Pubkey, data: Vec<u8>) {
        c.update(k, CachedAccount { owner, data, slot: 100, write_version: 1 }).unwrap();
    }
    fn num(d: &mut [u8], o: usize, n: u64) {
        d[o..o + 8].copy_from_slice(&n.to_le_bytes());
    }
    fn wide(d: &mut [u8], o: usize, n: u128) {
        d[o..o + 16].copy_from_slice(&n.to_le_bytes());
    }
    fn key(d: &mut [u8], o: usize, k: Pubkey) {
        d[o..o + 32].copy_from_slice(k.as_ref());
    }
    fn context() -> CacheReadContext {
        CacheReadContext { slot: 100, epoch: 1, maximum_slot_age: 0 }
    }
    fn request(amount: u64) -> CachedQuoteRequest {
        CachedQuoteRequest {
            amount_in: amount,
            slippage_basis_points: 100,
            unix_timestamp: 1000,
            maximum_arrays: 6,
        }
    }
    fn mints(c: &mut SubscriptionAccountCache, a: Pubkey, b: Pubkey) {
        for k in [a, b] {
            let mut d = vec![0; 82];
            d[45] = 1;
            put(c, k, crate::constants::TOKEN_PROGRAM, d);
        }
    }
    pub(crate) fn wp_fixture() -> (SubscriptionAccountCache, PoolTradeHint) {
        let mut c = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let mut d = vec![0; 653];
        d[..8].copy_from_slice(&[63, 149, 209, 12, 225, 128, 99, 9]);
        d[41..43].copy_from_slice(&1u16.to_le_bytes());
        d[43..45].copy_from_slice(&1u16.to_le_bytes());
        d[45..47].copy_from_slice(&3000u16.to_le_bytes());
        wide(&mut d, 49, 1_000_000_000);
        wide(&mut d, 65, 1 << 64);
        key(&mut d, 101, a);
        key(&mut d, 133, Pubkey::new_unique());
        key(&mut d, 181, b);
        key(&mut d, 213, Pubkey::new_unique());
        put(&mut c, pool, wp::PROGRAM_ID, d);
        mints(&mut c, a, b);
        for start in [-528i32, -440, -352, -264, -176, -88, 0, 88, 176, 264, 352, 440, 528] {
            let mut d = vec![0; 9988];
            d[..8].copy_from_slice(&[69, 97, 189, 190, 110, 7, 66, 187]);
            d[8..12].copy_from_slice(&start.to_le_bytes());
            key(&mut d, 9956, pool);
            put(&mut c, wp::tick_array_pda(&pool, start), wp::PROGRAM_ID, d);
        }
        (c, PoolTradeHint { pool, input_mint: a, output_mint: b })
    }
    #[test]
    fn whirlpool_both_directions_and_cross_array_quote_offline() {
        let (c, h) = wp_fixture();
        for pair in [h, PoolTradeHint { input_mint: h.output_mint, output_mint: h.input_mint, ..h }]
        {
            let small = c.quote_exact_in(pair, request(1_000_000), context()).unwrap();
            assert_eq!(small.estimated_net_amount_out, 996006);
            assert!(small.minimum_net_amount_out < small.estimated_net_amount_out);
            let large = c.quote_exact_in(pair, request(5_000_000), context()).unwrap();
            if let StonkFunQuoteVenue::OrcaWhirlpool(p) = large.hop.venue {
                assert!(p.tick_arrays.len() >= 3);
            } else {
                panic!("venue")
            }
        }
    }
    #[test]
    fn missing_cross_array_state_is_an_error_not_empty_liquidity() {
        let (mut c, h) = wp_fixture();
        c.accounts.remove(&wp::tick_array_pda(&h.pool, -88));
        let err = c.quote_exact_in(h, request(1_000_000), context()).err().unwrap();
        assert!(err.to_string().contains("Missing cached account"));
    }
    #[test]
    fn clmm_cache_selects_directional_bitmap_arrays_and_rejects_missing_state() {
        let mut cache = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let config = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let mut data = vec![0; 1544];
        data[..8].copy_from_slice(&[247, 237, 227, 245, 215, 195, 222, 70]);
        for (offset, key_value) in [
            (9, config),
            (73, a),
            (105, b),
            (137, Pubkey::new_unique()),
            (169, Pubkey::new_unique()),
            (201, Pubkey::new_unique()),
        ] {
            key(&mut data, offset, key_value);
        }
        data[235..237].copy_from_slice(&1u16.to_le_bytes());
        wide(&mut data, 237, 1_000_000_000);
        wide(&mut data, 253, 1 << 64);
        data[904 + 63] = 128; // array -1
        data[904 + 64] = 3; // arrays 0 and 1
        put(&mut cache, pool, cl::PROGRAM_ID, data);
        let mut config_data = vec![0; 117];
        config_data[..8].copy_from_slice(&[218, 244, 33, 104, 203, 203, 43, 111]);
        config_data[47..51].copy_from_slice(&3000u32.to_le_bytes());
        config_data[51..53].copy_from_slice(&1u16.to_le_bytes());
        put(&mut cache, config, cl::PROGRAM_ID, config_data);
        mints(&mut cache, a, b);
        for start in [-60i32, 0, 60] {
            let mut data = vec![0; 10240];
            data[..8].copy_from_slice(&[192, 155, 85, 205, 49, 249, 129, 42]);
            key(&mut data, 8, pool);
            data[40..44].copy_from_slice(&start.to_le_bytes());
            put(&mut cache, cl::tick_array_pda(&pool, start), cl::PROGRAM_ID, data);
        }
        for (input_mint, output_mint, first, last) in [(a, b, 0, -60), (b, a, 0, 0)] {
            let q = cache
                .quote_exact_in(
                    PoolTradeHint { pool, input_mint, output_mint },
                    request(1_000_000),
                    context(),
                )
                .unwrap();
            assert_eq!(q.estimated_net_amount_out, 996006);
            let StonkFunQuoteVenue::RaydiumClmm(p) = q.hop.venue else { panic!("venue") };
            assert_eq!(p.tick_arrays.first(), Some(&cl::tick_array_pda(&pool, first)));
            assert_eq!(p.tick_arrays.last(), Some(&cl::tick_array_pda(&pool, last)));
        }
        cache.accounts.remove(&cl::tick_array_pda(&pool, -60));
        assert!(cache
            .quote_exact_in(
                PoolTradeHint { pool, input_mint: a, output_mint: b },
                request(1_000_000),
                context()
            )
            .is_err());
    }
    #[test]
    fn clmm_static_traversal_matches_extracted_upstream_math() {
        use solana_clmm_raydium as math;
        let pool = math::SwapPool {
            sqrt_price_x64: 1 << 64,
            liquidity: 1_000_000_000,
            tick_current: 0,
            tick_spacing: 1,
            fee_rate_pips: 3000,
        };
        for down in [true, false] {
            for amount in [1, 10, 1_000_000, 50_000_000] {
                let tick = if down { -100 } else { 100 };
                let limit = math::get_sqrt_price_at_tick(if down { -1000 } else { 1000 }).unwrap();
                let ticks = [ClmmTick {
                    tick,
                    liquidity_net: 50_000_000,
                    liquidity_gross: 50_000_000,
                    orders: 0,
                    partial_orders: 0,
                }];
                let upstream = math::compute_swap_full(
                    &pool,
                    &[math::InitializedTick { tick, liquidity_net: 50_000_000 }],
                    amount,
                    limit,
                    true,
                    down,
                )
                .unwrap();
                assert_eq!(
                    clmm_swap(&pool, &ticks, amount, limit, 0, &[0; 80], 1000, down).unwrap(),
                    (upstream.amount_in, upstream.amount_out)
                );
            }
        }
    }
    #[test]
    fn clmm_fee_side_and_boundary_orders_work() {
        use solana_clmm_raydium as math;
        let pool = math::SwapPool {
            sqrt_price_x64: 1 << 64,
            liquidity: 0,
            tick_current: 0,
            tick_spacing: 1,
            fee_rate_pips: 3000,
        };
        let ticks = [ClmmTick {
            tick: 0,
            liquidity_net: 0,
            liquidity_gross: 0,
            orders: 2000,
            partial_orders: 0,
        }];
        let limit = math::get_sqrt_price_at_tick(-100).unwrap();
        for fee_on in [0, 1, 2] {
            assert_eq!(
                clmm_swap(&pool, &ticks, 1000, limit, fee_on, &[0; 80], 1000, true).unwrap(),
                (1000, 997)
            );
        }
    }
    #[test]
    fn dlmm_quote_crosses_bins_and_missing_bitmap_account_fails() {
        let mut c = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let mut d = vec![0; 904];
        d[..8].copy_from_slice(&[33, 11, 49, 98, 181, 101, 177, 13]);
        d[80..82].copy_from_slice(&1u16.to_le_bytes());
        key(&mut d, 88, a);
        key(&mut d, 120, b);
        key(&mut d, 152, Pubkey::new_unique());
        key(&mut d, 184, Pubkey::new_unique());
        // Current bin zero, bitmap arrays 0 and -1.
        d[584 + 512 / 8] |= 1;
        d[584 + 511 / 8] |= 128;
        put(&mut c, pool, dl::PROGRAM_ID, d);
        mints(&mut c, a, b);
        for index in [0i64, -1] {
            let mut d = vec![0; 10136];
            d[..8].copy_from_slice(&[92, 142, 92, 220, 5, 148, 70, 181]);
            d[8..16].copy_from_slice(&index.to_le_bytes());
            key(&mut d, 24, pool);
            let bin = if index == 0 { 0 } else { 69 };
            let o = 56 + bin * 144;
            num(&mut d, o, 1000);
            num(&mut d, o + 8, 1000);
            wide(&mut d, o + 16, 1 << 64);
            put(&mut c, dl::bin_array_pda(&pool, index), dl::PROGRAM_ID, d);
        }
        let h = PoolTradeHint { pool, input_mint: a, output_mint: b };
        let result = c.quote_exact_in(h, request(1500), context()).unwrap();
        assert_eq!(result.estimated_net_amount_out, 1500);
        if let StonkFunQuoteVenue::MeteoraDlmm(p) = result.hop.venue {
            assert_eq!(p.bin_arrays.len(), 2);
        } else {
            panic!("venue")
        }
        c.accounts.remove(&dl::bin_array_pda(&pool, -1));
        assert!(c.quote_exact_in(h, request(1500), context()).is_err());
    }
    #[tokio::test]
    async fn amount_appropriate_whirlpool_supplemental_arrays_reach_instruction() {
        use crate::trading::core::traits::InstructionBuilder;
        let (c, h) = wp_fixture();
        let result = c.quote_exact_in(h, request(20_000_000), context()).unwrap();
        let StonkFunQuoteVenue::OrcaWhirlpool(p) = result.hop.venue else { panic!("venue") };
        assert_eq!(p.tick_arrays.len(), 6);
        let mut params = crate::instruction::stonkfun::tests::swap_params(
            crate::swqos::TradeType::Buy,
            h.input_mint,
            h.output_mint,
            super::super::DexParamEnum::OrcaWhirlpool(p.clone()),
        );
        params.input_amount = Some(20_000_000);
        params.fixed_output_amount = Some(result.minimum_net_amount_out);
        let instructions = crate::instruction::whirlpool::WhirlpoolInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .unwrap();
        let ix = instructions.iter().find(|ix| ix.program_id == wp::PROGRAM_ID).unwrap();
        assert_eq!(&ix.data[42..], &[1, 1, 0, 0, 0, 6, 3]);
        assert_eq!(ix.accounts.len(), 18);
        assert_eq!(ix.accounts[17].pubkey, p.tick_arrays[5]);
    }
    #[test]
    fn dynamic_whirlpool_arrays_match_fixed_array_quotes() {
        let (mut c, h) = wp_fixture();
        let before = c.quote_exact_in(h, request(1_000_000), context()).unwrap();
        let ids: Vec<_> = c.accounts.keys().copied().collect();
        for id in ids {
            if c.accounts[&id].data.get(..8) == Some(&[69, 97, 189, 190, 110, 7, 66, 187]) {
                let start = i32le(&c.accounts[&id].data, 8).unwrap();
                let mut data = vec![0; 148];
                data[..8].copy_from_slice(&[17, 216, 246, 142, 225, 199, 218, 56]);
                data[8..12].copy_from_slice(&start.to_le_bytes());
                key(&mut data, 12, h.pool);
                c.accounts.get_mut(&id).unwrap().data = data;
            }
        }
        assert_eq!(
            c.quote_exact_in(h, request(1_000_000), context()).unwrap().estimated_net_amount_out,
            before.estimated_net_amount_out
        );
    }
    #[test]
    fn clmm_dynamic_saturated_fee_matches_constant_fee_reference() {
        use solana_clmm_raydium as math;
        let mut pool = math::SwapPool {
            sqrt_price_x64: 1 << 64,
            liquidity: 1_000_000_000,
            tick_current: 0,
            tick_spacing: 1,
            fee_rate_pips: 3000,
        };
        let mut dynamic = [0; 80];
        dynamic[..2].copy_from_slice(&10u16.to_le_bytes());
        dynamic[2..4].copy_from_slice(&20u16.to_le_bytes());
        dynamic[4..6].copy_from_slice(&5000u16.to_le_bytes());
        dynamic[6..10].copy_from_slice(&1000u32.to_le_bytes());
        for offset in [10, 18, 22] {
            dynamic[offset..offset + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
        }
        dynamic[26..34].copy_from_slice(&1000u64.to_le_bytes());
        for down in [true, false] {
            let limit = math::get_sqrt_price_at_tick(if down { -1000 } else { 1000 }).unwrap();
            let actual = clmm_swap(&pool, &[], 1_000_000, limit, 0, &dynamic, 1001, down).unwrap();
            pool.fee_rate_pips = 3100;
            let reference =
                math::compute_swap_full(&pool, &[], 1_000_000, limit, true, down).unwrap();
            assert_eq!(actual, (reference.amount_in, reference.amount_out));
            pool.fee_rate_pips = 3000;
        }
    }
    #[tokio::test]
    async fn amm_v4_cache_subtracts_pnl_and_uses_current_swap_fee() {
        use crate::trading::core::traits::InstructionBuilder;
        let mut c = SubscriptionAccountCache::default();
        let pool = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let mut d = vec![0; 752];
        num(&mut d, 0, 6);
        num(&mut d, 176, 50);
        num(&mut d, 184, 10000);
        num(&mut d, 192, 100);
        num(&mut d, 200, 200);
        for (o, k) in [(336, va), (368, vb), (400, a), (432, b)] {
            key(&mut d, o, k);
        }
        let program = crate::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4;
        put(&mut c, pool, program, d);
        mints(&mut c, a, b);
        for (v, m, amount) in [(va, a, 1_000_000), (vb, b, 2_000_000)] {
            let mut d = vec![0; 165];
            key(&mut d, 0, m);
            num(&mut d, 64, amount);
            d[108] = 1;
            put(&mut c, v, crate::constants::TOKEN_PROGRAM, d);
        }
        let h = PoolTradeHint { pool, input_mint: a, output_mint: b };
        let result = c.quote_exact_in(h, request(100_000), context()).unwrap();
        assert_eq!(result.estimated_net_amount_out, 180989);
        assert_eq!(result.minimum_net_amount_out, 179179);
        let StonkFunQuoteVenue::RaydiumAmmV4(p) = result.hop.venue else { panic!("venue") };
        assert_eq!((p.coin_reserve, p.pc_reserve), (999900, 1999800));
        let mut params = crate::instruction::stonkfun::tests::swap_params(
            crate::swqos::TradeType::Buy,
            a,
            b,
            super::super::DexParamEnum::RaydiumAmmV4(p),
        );
        params.input_amount = Some(100_000);
        let instructions = crate::instruction::raydium_amm_v4::RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&params)
            .await
            .unwrap();
        let ix = instructions.iter().find(|ix| ix.program_id == program).unwrap();
        assert_eq!(u64::from_le_bytes(ix.data[9..17].try_into().unwrap()), 179179);
    }
    #[test]
    fn cached_split_merge_routes_spend_only_guaranteed_credit() {
        let (mut cache, first) = wp_fixture();
        let (mut branch, mut second) = wp_fixture();
        second.input_mint = first.input_mint;
        second.output_mint = first.output_mint;
        let state = &mut branch.accounts.get_mut(&second.pool).unwrap().data;
        key(state, 101, second.input_mint);
        key(state, 181, second.output_mint);
        cache.accounts.extend(branch.accounts);
        let (mut tail, mut third) = wp_fixture();
        third.input_mint = first.output_mint;
        key(&mut tail.accounts.get_mut(&third.pool).unwrap().data, 101, third.input_mint);
        cache.accounts.extend(tail.accounts);
        let steps = [
            CachedRouteStep { pool: first, input_amount: Some(400_000) },
            CachedRouteStep { pool: second, input_amount: None },
            CachedRouteStep { pool: third, input_amount: None },
        ];
        for is_buy in [true, false] {
            let route = cache
                .quote_route_exact_in(
                    &steps,
                    first.input_mint,
                    third.output_mint,
                    request(1_000_000),
                    context(),
                    is_buy,
                )
                .unwrap();
            let preview =
                route.preview(first.input_mint, third.output_mint, 1_000_000, 100, is_buy).unwrap();
            assert!(preview.minimum_output_credit > 900_000);
            let mut invalid = steps;
            invalid[1].input_amount = Some(600_001);
            assert!(cache
                .quote_route_exact_in(
                    &invalid,
                    first.input_mint,
                    third.output_mint,
                    request(1_000_000),
                    context(),
                    is_buy
                )
                .is_err());
            invalid = steps;
            invalid[1].pool = first;
            let error = cache
                .quote_route_exact_in(
                    &invalid,
                    first.input_mint,
                    third.output_mint,
                    request(1_000_000),
                    context(),
                    is_buy,
                )
                .err()
                .unwrap();
            assert!(error.to_string().contains("reuse a pool"));
        }
    }
    #[test]
    fn truncated_pool_decoders_never_panic() {
        for n in 0..584 {
            assert!(dl::decode_lb_pair(&vec![0; n]).is_err());
        }
        for n in 0..273 {
            assert!(cl::decode_pool_state(&vec![0; n]).is_err());
        }
    }
}

/// Explicit execution-ordered branches; None spends the guaranteed balance.
#[derive(Clone, Copy, Debug)]
pub struct CachedRouteStep {
    pub pool: PoolTradeHint,
    pub input_amount: Option<u64>,
}
impl SubscriptionAccountCache {
    /// Quotes serial or split/merge quote conversions for one independent buy
    /// or sell. Historical transactions are clues, never implicit route plans.
    pub fn quote_route_exact_in(
        &self,
        steps: &[CachedRouteStep],
        input_mint: Pubkey,
        output_mint: Pubkey,
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
    ) -> Result<super::StonkFunQuoteRoute> {
        use std::collections::{HashMap, HashSet};
        let mut balances = HashMap::new();
        balances.insert(input_mint, request.amount_in);
        let mut pools = HashSet::new();
        let mut hops = Vec::new();
        ensure!(!steps.is_empty(), "Empty cached quote route");
        for step in steps {
            ensure!(pools.insert(step.pool.pool),"A cached quote route cannot reuse a pool without updating its state after the earlier swap");
            let available = balances.get(&step.pool.input_mint).copied().unwrap_or(0u64);
            let amount = step.input_amount.unwrap_or(available);
            ensure!(
                amount > 0 && amount <= available,
                "Cached quote route overspends guaranteed balance"
            );
            let quote = self.quote_exact_in(
                step.pool,
                CachedQuoteRequest { amount_in: amount, ..request },
                ctx,
            )?;
            balances.insert(step.pool.input_mint, available - amount);
            let credit = balances.entry(step.pool.output_mint).or_default();
            *credit = credit
                .checked_add(quote.minimum_net_amount_out)
                .ok_or_else(|| anyhow!("Cached route credit overflow"))?;
            hops.push(quote.hop);
        }
        let route = if is_buy {
            super::StonkFunQuoteRoute::buy(hops)
        } else {
            super::StonkFunQuoteRoute::sell(hops)
        };
        route.preview(
            input_mint,
            output_mint,
            request.amount_in,
            u64::from(request.slippage_basis_points),
            is_buy,
        )?;
        Ok(route)
    }
}
