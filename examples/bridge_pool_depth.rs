//! Replay a coherent mainnet account snapshot, without RPC or transaction submission.
//! cargo run --example bridge_pool_depth -- snapshot.json
//! Snapshot format: {slot, hints: [{pool,input_mint,output_mint}], accounts: {pubkey: RPC account}}.
//! Hints must be oriented USDC/WSOL -> bridge token. This is a catalog audit,
//! not a live price oracle or a replacement for refreshing subscription state.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sol_trade_sdk::{
    constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    trading::core::params::{CachedQuoteRequest, PoolTradeHint},
};
use solana_sdk::{pubkey, pubkey::Pubkey};
fn key(v: &Value) -> Result<Pubkey> {
    v.as_str().context("public key missing")?.parse().context("invalid public key")
}
#[path = "support/account_snapshot.rs"]
mod account_snapshot;

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("snapshot.json required")?;
    let f: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let snapshot = account_snapshot::AccountSnapshot::from_document(&f)?;
    let cache = snapshot.cache;
    let ctx = snapshot.context;
    let slot = ctx.slot;
    let time = snapshot.unix_timestamp;
    let quote = |h, amount| {
        cache
            .quote_exact_in(
                h,
                CachedQuoteRequest {
                    amount_in: amount,
                    slippage_basis_points: 0,
                    unix_timestamp: time,
                    maximum_arrays: 6,
                },
                ctx,
            )
            .map(|q| q.estimated_net_amount_out)
    };
    let sol = PoolTradeHint {
        pool: pubkey!("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"),
        input_mint: USDC_TOKEN_ACCOUNT,
        output_mint: WSOL_TOKEN_ACCOUNT,
    };
    let sol_per_dollar = quote(sol, 10_000_000)? / 10;
    let mut reports = Vec::new();
    for h in f["hints"].as_array().context("hints missing")? {
        let hint = PoolTradeHint {
            pool: key(&h["pool"])?,
            input_mint: key(&h["input_mint"])?,
            output_mint: key(&h["output_mint"])?,
        };
        ensure!(
            [USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT].contains(&hint.input_mint),
            "Expected liquid quote asset"
        );
        let unit = if hint.input_mint == USDC_TOKEN_ACCOUNT { 1_000_000 } else { sol_per_dollar };
        let audit = (|| -> Result<Value> {
            let small = quote(hint, unit)?;
            let large = quote(hint, unit * 10)?;
            let reverse = PoolTradeHint {
                input_mint: hint.output_mint,
                output_mint: hint.input_mint,
                ..hint
            };
            let sell_small = quote(reverse, small)?;
            let sell_large = quote(reverse, small.checked_mul(10).context("amount overflow")?)?;
            let buy_impact = 1.0 - large as f64 / (small as f64 * 10.0);
            let sell_impact = 1.0 - sell_large as f64 / (sell_small as f64 * 10.0);
            // Count only the pool's verified liquid-side vault, never unrelated balances.
            let account = &f["accounts"][hint.pool.to_string()];
            let bytes =
                STANDARD.decode(account["data"][0].as_str().context("pool data missing")?)?;
            let owner = key(&account["owner"])?;
            use sol_trade_sdk::instruction::utils::{
                meteora_dlmm as dl, raydium_amm_v4_types as amm, raydium_clmm as cl,
                raydium_cpmm as cp, raydium_cpmm_types as cpt, whirlpool as wp,
            };
            let (mints, vaults) = if owner == dl::PROGRAM_ID {
                let p = dl::decode_lb_pair(&bytes)?;
                ([p.token_x_mint, p.token_y_mint], [p.reserve_x, p.reserve_y])
            } else if owner == cl::PROGRAM_ID {
                let p = cl::decode_pool_state(&bytes)?;
                ([p.token_mint_0, p.token_mint_1], [p.token_vault_0, p.token_vault_1])
            } else if owner == wp::PROGRAM_ID {
                let p = wp::decode_whirlpool(&bytes)?;
                ([p.token_mint_a, p.token_mint_b], [p.token_vault_a, p.token_vault_b])
            } else if owner == cp::accounts::RAYDIUM_CPMM {
                ensure!(
                    bytes.get(..8) == Some(&cpt::POOL_STATE_DISCRIMINATOR),
                    "Invalid CPMM pool"
                );
                let p = cpt::pool_state_decode(&bytes[8..]).context("invalid CPMM pool")?;
                ([p.token0_mint, p.token1_mint], [p.token0_vault, p.token1_vault])
            } else if owner == pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8") {
                let p = amm::amm_info_decode(&bytes).context("invalid AMM pool")?;
                ([p.coin_mint, p.pc_mint], [p.token_coin, p.token_pc])
            } else {
                anyhow::bail!("Unsupported audit vault layout");
            };
            let side =
                mints.iter().position(|m| *m == hint.input_mint).context("quote mint missing")?;
            let vault_account = &f["accounts"][vaults[side].to_string()];
            let vault_data =
                STANDARD.decode(vault_account["data"][0].as_str().context("vault missing")?)?;
            ensure!(
                vault_data.len() >= 165 && vault_data[..32] == hint.input_mint.to_bytes(),
                "invalid quote vault"
            );
            ensure!(
                vault_account["owner"] == f["accounts"][hint.input_mint.to_string()]["owner"],
                "vault token program mismatch"
            );
            let reserve_usd =
                u64::from_le_bytes(vault_data[64..72].try_into()?) as f64 / unit as f64;
            Ok(
                json!({"liquid_side_reserve_usd":reserve_usd,"buy_10_usd_impact_bps":buy_impact*10000.0,"sell_10_usd_impact_bps":sell_impact*10000.0,"eligible":reserve_usd>=500.0,"buy_output":large,"sell_output":sell_large}),
            )
        })();
        reports.push(json!({"pool":hint.pool.to_string(),"audit":match audit { Ok(v)=>v,Err(e)=>json!({"eligible":false,"error":e.to_string()}) }}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"slot":slot,"unix_timestamp":time,"probe_usd":10,"reference_probe_usd":1,"minimum_liquid_side_usd":500,"reports":reports})
        )?
    );
    ensure!(
        !reports.is_empty() && reports.iter().all(|r| r["audit"]["eligible"] == true),
        "At least one candidate failed depth admission"
    );
    Ok(())
}
