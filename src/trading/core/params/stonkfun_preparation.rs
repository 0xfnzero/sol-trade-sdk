//! Complete cached preparation for one StonkFun buy or one StonkFun sell.
use super::*;
use anyhow::{anyhow, ensure, Result};
use solana_sdk::pubkey::Pubkey;

#[derive(Clone)]
pub struct PreparedStonkFunTrade {
    pub via: StonkFunViaQuoteParams,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
    pub slippage_basis_points: u16,
    pub is_buy: bool,
}
impl PreparedStonkFunTrade {
    /// Uses the caller's wallet, blockhash/nonce, ALTs and account lifecycle
    /// policy. Applies the exact prepared amount, direction and asset selection.
    pub fn apply_to(self, mut params: SwapParams) -> Result<SwapParams> {
        params.trade_type =
            if self.is_buy { crate::swqos::TradeType::Buy } else { crate::swqos::TradeType::Sell };
        params.input_mint = self.input_mint;
        params.output_mint = self.output_mint;
        params.input_token_program = None;
        params.output_token_program = None;
        params.input_amount = Some(self.amount_in);
        params.slippage_basis_points = Some(u64::from(self.slippage_basis_points));
        // Meme builders use fixed_output_amount for exact-output swaps.
        // Keep exact-input mode; they enforce the same prepared net minimum
        // from these immutable params and slippage during instruction build.
        params.fixed_output_amount = None;
        params.protocol_params = self.via.into_extension();
        params.without_rpc()
    }
}
fn canonical(mint: Pubkey) -> Pubkey {
    if mint == crate::constants::SOL_TOKEN_ACCOUNT {
        crate::constants::WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}
impl SubscriptionAccountCache {
    /// `meme_hint` comes from parser identity clues; `asset` is the user's choice
    /// of SOL, WSOL, USDC, or the pool quote. Buy/sell paths are independent.
    pub fn prepare_stonkfun_trade(
        &self,
        meme_hint: PoolTradeHint,
        meme_mint: Pubkey,
        asset: Pubkey,
        conversion: &[CachedRouteStep],
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
    ) -> Result<PreparedStonkFunTrade> {
        ensure!(
            request.amount_in > 0 && request.slippage_basis_points < 10_000,
            "Invalid StonkFun preparation request"
        );
        let account = self
            .accounts
            .get(&meme_hint.pool)
            .ok_or_else(|| anyhow!("Missing cached meme pool"))?;
        let (meme, quote) = if account.owner == crate::instruction::utils::bonk::accounts::BONK {
            let data = self.get(meme_hint.pool, account.owner, ctx)?;
            let p = crate::instruction::utils::bonk_types::pool_state_decode(
                data.get(8..).ok_or_else(|| anyhow!("Truncated LaunchLab pool"))?,
            )
            .ok_or_else(|| anyhow!("Invalid LaunchLab pool"))?;
            ensure!(p.base_mint == meme_mint, "Meme mint does not match LaunchLab base mint");
            (StonkFunMemeLeg::Curve(self.launchlab(meme_hint, ctx)?), p.quote_mint)
        } else {
            ensure!(
                account.owner == crate::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM,
                "Meme pool is neither LaunchLab nor CPMM"
            );
            let p = self.cpmm_at(meme_hint, ctx, request.unix_timestamp)?;
            let quote = if p.base_mint == meme_mint {
                p.quote_mint
            } else {
                ensure!(p.quote_mint == meme_mint, "Meme mint is absent from CPMM");
                p.base_mint
            };
            (StonkFunMemeLeg::Graduated(p), quote)
        };
        let endpoint = canonical(asset);
        ensure!(
            endpoint == quote
                || endpoint == crate::constants::WSOL_TOKEN_ACCOUNT
                || endpoint == crate::constants::USDC_TOKEN_ACCOUNT,
            "Payment/receipt asset must be SOL, WSOL, USDC or the meme pool quote"
        );
        ensure!(
            !conversion.iter().any(|s| s.pool.pool == meme_hint.pool),
            "Quote conversion cannot reuse the meme pool"
        );
        let meme_min = |amount: u64, buy: bool| -> Result<u64> {
            match &meme {
                StonkFunMemeLeg::Curve(p) => {
                    if buy {
                        Ok(crate::utils::calc::bonk::get_buy_quote(
                            amount,
                            p,
                            0,
                            u128::from(request.slippage_basis_points),
                        )?
                        .minimum_amount_out)
                    } else {
                        crate::utils::calc::bonk::get_sell_min_amount_out(
                            amount,
                            p,
                            0,
                            u128::from(request.slippage_basis_points),
                        )
                    }
                }
                StonkFunMemeLeg::Graduated(p) => {
                    Ok(crate::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
                        p,
                        if buy { quote == p.base_mint } else { meme_mint == p.base_mint },
                        amount,
                        u64::from(request.slippage_basis_points),
                    )?
                    .min_amount_out)
                }
                // Not prepared from this cache: it reads LaunchLab and CPMM pools.
                StonkFunMemeLeg::MeteoraDbc(_) | StonkFunMemeLeg::MeteoraDammV2(_) => {
                    Err(anyhow!("Meteora meme legs are not prepared from the subscription cache"))
                }
            }
        };
        let (route, minimum) = if endpoint == quote {
            ensure!(conversion.is_empty(), "Direct quote trade must not include conversion hops");
            (StonkFunQuoteRoute::default(), meme_min(request.amount_in, is_buy)?)
        } else if is_buy {
            let route =
                self.quote_route_exact_in(conversion, endpoint, quote, request, ctx, true)?;
            let credit = route
                .preview(
                    endpoint,
                    quote,
                    request.amount_in,
                    u64::from(request.slippage_basis_points),
                    true,
                )?
                .minimum_output_credit;
            let minimum = meme_min(credit, true)?;
            (route, minimum)
        } else {
            let credit = meme_min(request.amount_in, false)?;
            let route = self.quote_route_exact_in(
                conversion,
                quote,
                endpoint,
                CachedQuoteRequest { amount_in: credit, ..request },
                ctx,
                false,
            )?;
            let minimum = route
                .preview(quote, endpoint, credit, u64::from(request.slippage_basis_points), false)?
                .minimum_output_credit;
            (route, minimum)
        };
        ensure!(minimum > 0, "StonkFun quote produces zero protected output");
        Ok(PreparedStonkFunTrade {
            via: StonkFunViaQuoteParams {
                meme_leg: meme,
                sol_hop: StonkFunSolHop::Route(route),
                quote_hop: None,
                hop_slippage_basis_points: None,
            },
            input_mint: if is_buy { asset } else { meme_mint },
            output_mint: if is_buy { meme_mint } else { asset },
            amount_in: request.amount_in,
            minimum_amount_out: minimum,
            slippage_basis_points: request.slippage_basis_points,
            is_buy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::stonkfun::StonkFunInstructionBuilder;
    use crate::trading::core::traits::InstructionBuilder;
    fn fixture(
        asset: Pubkey,
        inner: bool,
    ) -> (SubscriptionAccountCache, PoolTradeHint, PoolTradeHint, Pubkey) {
        let (mut cache, meme, _) = super::super::subscription_cache::tests::fixture();
        let (mut quote, mut hop) = super::super::cached_quote::tests::wp_fixture();
        let endpoint = canonical(asset);
        hop.input_mint = endpoint;
        hop.output_mint = meme.input_mint;
        let data = &mut quote.accounts.get_mut(&hop.pool).unwrap().data;
        data[101..133].copy_from_slice(endpoint.as_ref());
        data[181..213].copy_from_slice(meme.input_mint.as_ref());
        let mut mint = vec![0; 82];
        mint[45] = 1;
        cache
            .update(
                endpoint,
                CachedAccount {
                    owner: crate::constants::TOKEN_PROGRAM,
                    data: mint,
                    slot: 100,
                    write_version: 1,
                },
            )
            .unwrap();
        cache.accounts.extend(quote.accounts);
        if inner {
            let global = Pubkey::new_unique();
            let platform = Pubkey::new_unique();
            let mut data = vec![0; 429];
            data[..8].copy_from_slice(&[247, 237, 227, 245, 215, 195, 222, 70]);
            for (offset, key) in [
                (141, global),
                (173, platform),
                (205, meme.output_mint),
                (237, meme.input_mint),
                (269, Pubkey::new_unique()),
                (301, Pubkey::new_unique()),
                (333, Pubkey::new_unique()),
            ] {
                data[offset..offset + 32].copy_from_slice(key.as_ref());
            }
            for (offset, value) in [
                (29, 1_000_000_000u64),
                (37, 1_000_000_000),
                (45, 1_000_000_000),
                (61, 100_000_000),
            ] {
                data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            }
            let mut g = vec![0; 35];
            g[..8].copy_from_slice(&[149, 8, 156, 202, 160, 252, 176, 217]);
            g[27..35].copy_from_slice(&3000u64.to_le_bytes());
            let mut p = vec![0; 728];
            p[..8].copy_from_slice(&[160, 78, 128, 0, 248, 83, 230, 160]);
            for (key, data) in [(meme.pool, data), (global, g), (platform, p)] {
                cache.accounts.insert(
                    key,
                    CachedAccount {
                        owner: crate::instruction::utils::bonk::accounts::BONK,
                        data,
                        slot: 100,
                        write_version: 1,
                    },
                );
            }
        }
        (cache, meme, hop, meme.output_mint)
    }
    #[tokio::test]
    async fn independent_inner_and_graduated_asset_matrix_builds_without_rpc() {
        let ctx = CacheReadContext { slot: 100, epoch: 3, maximum_slot_age: 0 };
        for inner in [true, false] {
            for asset in [
                crate::constants::SOL_TOKEN_ACCOUNT,
                crate::constants::WSOL_TOKEN_ACCOUNT,
                crate::constants::USDC_TOKEN_ACCOUNT,
            ] {
                let (cache, meme, hop, mint) = fixture(asset, inner);
                for buy in [true, false] {
                    let request = CachedQuoteRequest {
                        amount_in: if buy { 10_000 } else { 100 },
                        slippage_basis_points: 100,
                        unix_timestamp: 1000,
                        maximum_arrays: 6,
                    };
                    let conversion = if buy {
                        hop
                    } else {
                        PoolTradeHint {
                            input_mint: hop.output_mint,
                            output_mint: hop.input_mint,
                            ..hop
                        }
                    };
                    let prepared = cache
                        .prepare_stonkfun_trade(
                            meme,
                            mint,
                            asset,
                            &[CachedRouteStep { pool: conversion, input_amount: None }],
                            request,
                            ctx,
                            buy,
                        )
                        .unwrap();
                    assert!(prepared.minimum_amount_out > 0);
                    let mut template = crate::instruction::stonkfun::tests::swap_params(
                        crate::swqos::TradeType::Buy,
                        asset,
                        mint,
                        prepared.via.clone().into_extension(),
                    );
                    template.simulate = false;
                    template.close_input_mint_ata = false;
                    template.close_output_mint_ata =
                        asset == crate::constants::SOL_TOKEN_ACCOUNT && !buy;
                    let params = prepared.apply_to(template).unwrap();
                    assert!(params.rpc.is_none());
                    assert!(params.fixed_output_amount.is_none());
                    let instructions = if buy {
                        StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap()
                    } else {
                        StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap()
                    };
                    let economic=instructions.iter().filter(|ix|ix.program_id==crate::instruction::utils::whirlpool::PROGRAM_ID || ix.program_id==crate::instruction::utils::bonk::accounts::BONK || ix.program_id==crate::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM).count();
                    assert_eq!(economic, 2);
                }
                for buy in [true, false] {
                    let request = CachedQuoteRequest {
                        amount_in: 100,
                        slippage_basis_points: 100,
                        unix_timestamp: 1000,
                        maximum_arrays: 6,
                    };
                    let prepared = cache
                        .prepare_stonkfun_trade(meme, mint, meme.input_mint, &[], request, ctx, buy)
                        .unwrap();
                    let mut template = crate::instruction::stonkfun::tests::swap_params(
                        crate::swqos::TradeType::Buy,
                        meme.input_mint,
                        mint,
                        prepared.via.clone().into_extension(),
                    );
                    template.simulate = false;
                    let params = prepared.apply_to(template).unwrap();
                    let instructions = if buy {
                        StonkFunInstructionBuilder.build_buy_instructions(&params).await.unwrap()
                    } else {
                        StonkFunInstructionBuilder.build_sell_instructions(&params).await.unwrap()
                    };
                    assert!(!instructions.iter().any(
                        |ix| ix.program_id == crate::instruction::utils::whirlpool::PROGRAM_ID
                    ));
                }
            }
        }
    }
}
