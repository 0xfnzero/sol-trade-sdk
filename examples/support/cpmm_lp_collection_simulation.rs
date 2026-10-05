//! IDL-built LP operations compared using unsigned mainnet simulations.
//! LP instruction construction here is test support, not a public SDK LP API.
#[allow(dead_code)]
#[path = "cpmm_swap_collection_simulation.rs"]
mod swap;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sol_trade_sdk::{
    common::{
        fast_fn::{
            create_associated_token_account_idempotent_fast_use_seed as create_ata,
            get_associated_token_address_with_program_id_fast_use_seed as ata,
        },
        SolanaRpcClient,
    },
    instruction::{
        raydium_cpmm::{
            collect_creator_fee, collect_creator_fee_permissionless, RaydiumCpmmInstructionBuilder,
        },
        utils::{
            raydium_cpmm::{
                accounts, fetch_pool_state, get_creator_fee_share_pda,
                resolve_creator_fee_share_rate, split_creator_fee,
            },
            raydium_cpmm_types::PoolState,
        },
    },
    swqos::TradeType,
    trading::core::{params::DexParamEnum, traits::InstructionBuilder},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::sync::Arc;
use swap::{collection::*, protocol_params, simulate, swap_params};

fn lp_instruction(
    pool: Pubkey,
    p: &PoolState,
    user: Pubkey,
    lp: u64,
    withdraw: bool,
) -> Instruction {
    let spl = sol_trade_sdk::constants::TOKEN_PROGRAM;
    let mut metas = vec![
        AccountMeta::new_readonly(user, true),
        AccountMeta::new_readonly(accounts::AUTHORITY, false),
        AccountMeta::new(pool, false),
        AccountMeta::new(ata(&user, &p.lp_mint, &spl, false), false),
        AccountMeta::new(ata(&user, &p.token0_mint, &p.token0_program, false), false),
        AccountMeta::new(ata(&user, &p.token1_mint, &p.token1_program, false), false),
        AccountMeta::new(p.token0_vault, false),
        AccountMeta::new(p.token1_vault, false),
        AccountMeta::new_readonly(spl, false),
        AccountMeta::new_readonly(spl_token_2022_interface::ID, false),
        AccountMeta::new_readonly(p.token0_mint, false),
        AccountMeta::new_readonly(p.token1_mint, false),
        AccountMeta::new(p.lp_mint, false),
    ];
    if withdraw {
        metas.push(AccountMeta::new_readonly(
            "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr".parse().unwrap(),
            false,
        ));
    }
    let mut data = if withdraw {
        vec![183, 18, 70, 156, 148, 109, 161, 34]
    } else {
        vec![242, 35, 198, 137, 82, 225, 242, 182]
    };
    data.extend(lp.to_le_bytes());
    for _ in 0..2 {
        data.extend(if withdraw { 0u64 } else { u64::MAX }.to_le_bytes());
    }
    Instruction { program_id: accounts::RAYDIUM_CPMM, accounts: metas, data }
}
fn amount(list: &[Option<Account>], i: usize, p: &PoolState, user: Pubkey) -> Result<u64> {
    let (mint, program, owner) = match i {
        3 => (p.token0_mint, p.token0_program, accounts::AUTHORITY),
        4 => (p.token1_mint, p.token1_program, accounts::AUTHORITY),
        5 | 9 => (p.token0_mint, p.token0_program, if i == 5 { p.pool_creator } else { user }),
        6 | 10 => (p.token1_mint, p.token1_program, if i == 6 { p.pool_creator } else { user }),
        11 => (p.lp_mint, sol_trade_sdk::constants::TOKEN_PROGRAM, user),
        _ => anyhow::bail!("Invalid token index"),
    };
    token_amount(required(list, i)?, &mint, &owner, &program)
}
fn supply(list: &[Option<Account>]) -> Result<u64> {
    let a = required(list, 12)?;
    ensure!(a.owner == sol_trade_sdk::constants::TOKEN_PROGRAM, "Wrong LP mint program");
    Ok(u64::from_le_bytes(a.data.get(36..44).context("Short LP mint")?.try_into()?))
}
fn decode_snapshots(v: &Value) -> Result<Vec<Option<Account>>> {
    v.as_array()
        .context("Missing snapshot")?
        .iter()
        .map(|a| {
            if a.is_null() {
                return Ok(None);
            }
            Ok(Some(Account {
                owner: a["owner"].as_str().context("owner")?.parse()?,
                data: STANDARD.decode(a["data_base64"].as_str().context("bytes")?)?,
                lamports: a["lamports"].as_u64().context("lamports")?,
                executable: a["executable"].as_bool().context("executable")?,
                rent_epoch: a["rent_epoch"].as_u64().context("rent epoch")?,
            }))
        })
        .collect()
}
// LpChangeEvent has no user or requested LP amount. Verify its actual on-chain fields.
fn lp_event(logs: &Value) -> Result<Vec<u8>> {
    // sha256("event:LpChangeEvent")[..8]
    const DISCRIMINATOR: [u8; 8] = [121, 163, 205, 201, 57, 218, 117, 60];
    logs.as_array()
        .context("logs")?
        .iter()
        .filter_map(|l| {
            let data = STANDARD.decode(l.as_str()?.strip_prefix("Program data: ")?).ok()?;
            (data.get(..8) == Some(DISCRIMINATOR.as_slice())).then_some(data)
        })
        .last()
        .context("Missing LpChangeEvent")
}
pub fn verify_row(r: &Value) -> Result<()> {
    let prep = decode_snapshots(&r["prepared_accounts"])?;
    let baseline = decode_snapshots(&r["baseline_accounts"])?;
    let collected = decode_snapshots(&r["collected_accounts"])?;
    ensure!(prep.len() == 13 && baseline.len() == 13 && collected.len() == 13, "Snapshot shape");
    let p = decode_pool(required(&prep, 0)?)?;
    let a = decode_pool(required(&baseline, 0)?)?;
    let b = decode_pool(required(&collected, 0)?)?;
    let user: Pubkey = r["trader"].as_str().context("trader")?.parse()?;
    let pool: Pubkey = r["pool"].as_str().context("pool")?.parse()?;
    let lp: u64 = r["lp_amount"].as_str().context("lp")?.parse()?;
    let withdraw = r["withdraw"].as_bool().context("operation")?;
    for (key, index) in [("token0_delta", 9), ("token1_delta", 10), ("lp_delta", 11)] {
        let delta = i128::from(amount(&baseline, index, &p, user)?)
            - i128::from(amount(&prep, index, &p, user)?);
        if let Some(reported) = r.get(key) {
            ensure!(
                reported.as_str() == Some(delta.to_string().as_str()),
                "Reported {key} differs from simulated account balances"
            );
        }
    }
    let config = decode_config(required(&prep, 1)?)?;
    let rate =
        resolve_creator_fee_share_rate(&config, &p.pool_creator, &p.amm_config, prep[2].as_ref())?;
    ensure!(r["share_rate"] == rate, "Reported share rate mismatch");
    ensure!(
        lp > 0
            && p.lp_supply > 0
            && rate > 0
            && (p.creator_fees_token0 > 0 || p.creator_fees_token1 > 0),
        "Comparison must exercise collection and LP changes"
    );
    let pre = protocol_params(pool, &p, &config, &prep, p.recent_epoch)?;
    let pa = protocol_params(pool, &a, &config, &baseline, p.recent_epoch)?;
    let pb = protocol_params(pool, &b, &config, &collected, p.recent_epoch)?;
    let expected_supply =
        if withdraw { p.lp_supply.checked_sub(lp) } else { p.lp_supply.checked_add(lp) };
    ensure!(Some(a.lp_supply) == expected_supply && a.lp_supply == b.lp_supply, "LP supply delta");
    let mut expected_pool = p.clone();
    expected_pool.lp_supply = a.lp_supply;
    expected_pool.recent_epoch = a.recent_epoch;
    ensure!(a == expected_pool, "LP changed unrelated pool fields");
    expected_pool.creator_fees_token0 = 0;
    expected_pool.creator_fees_token1 = 0;
    expected_pool.protocol_fees_token0 = expected_pool
        .protocol_fees_token0
        .checked_add(split_creator_fee(p.creator_fees_token0, rate)?.1)
        .context("Protocol token0 fee overflow")?;
    expected_pool.protocol_fees_token1 = expected_pool
        .protocol_fees_token1
        .checked_add(split_creator_fee(p.creator_fees_token1, rate)?.1)
        .context("Protocol token1 fee overflow")?;
    ensure!(b == expected_pool, "Collection changed unrelated pool fields");
    let expected_mint =
        if withdraw { supply(&prep)?.checked_sub(lp) } else { supply(&prep)?.checked_add(lp) };
    ensure!(
        Some(supply(&baseline)?) == expected_mint && supply(&baseline)? == supply(&collected)?,
        "LP mint delta"
    );
    let user_lp = amount(&prep, 11, &p, user)?;
    let expected_lp = if withdraw { user_lp.checked_sub(lp) } else { user_lp.checked_add(lp) };
    ensure!(
        Some(amount(&baseline, 11, &p, user)?) == expected_lp
            && amount(&baseline, 11, &p, user)? == amount(&collected, 11, &p, user)?,
        "User LP delta"
    );
    ensure!(
        pa.base_reserve == pb.base_reserve && pa.quote_reserve == pb.quote_reserve,
        "Collection changed adjusted LP reserves"
    );
    let event = lp_event(&r["baseline_logs"])?;
    ensure!(event == lp_event(&r["collected_logs"])?, "Collection changed LP event");
    ensure!(
        event.len() == 97 && event[8..40] == pool.to_bytes() && event[96] == u8::from(withdraw),
        "LP event identity/layout"
    );
    let read = |i: usize| u64::from_le_bytes(event[40 + i * 8..48 + i * 8].try_into().unwrap());
    ensure!(
        read(0) == p.lp_supply && read(1) == pre.base_reserve && read(2) == pre.quote_reserve,
        "LP event pre-state"
    );
    for i in 0..2 {
        let reserve = if i == 0 { pre.base_reserve } else { pre.quote_reserve };
        let product = u128::from(lp) * u128::from(reserve);
        let net_reserve_amount = u64::try_from(if withdraw {
            product / u128::from(p.lp_supply)
        } else {
            product.div_ceil(u128::from(p.lp_supply))
        })?;
        let fees = if i == 0 { pre.base_transfer_fee } else { pre.quote_transfer_fee };
        let fee = if withdraw {
            fees.calculate(net_reserve_amount)
        } else {
            fees.calculate_inverse(net_reserve_amount)
        };
        let user_delta = if withdraw { net_reserve_amount - fee } else { net_reserve_amount + fee };
        let before = amount(&prep, 9 + i, &p, user)?;
        let after = amount(&baseline, 9 + i, &p, user)?;
        ensure!(after == amount(&collected, 9 + i, &p, user)?, "Collection changed user token{i}");
        ensure!(
            if withdraw { after.checked_sub(before) } else { before.checked_sub(after) }
                == Some(user_delta),
            "LP token{i} formula mismatch"
        );
        let post_reserve = if i == 0 { pa.base_reserve } else { pa.quote_reserve };
        ensure!(
            if withdraw {
                reserve.checked_sub(post_reserve)
            } else {
                post_reserve.checked_sub(reserve)
            } == Some(net_reserve_amount),
            "Reserve token{i} formula mismatch"
        );
        ensure!(
            read(3 + i) == if withdraw { user_delta } else { net_reserve_amount }
                && read(5 + i) == fee,
            "LP event token{i} amounts"
        );
        for index in [3 + i, 9 + i] {
            ensure!(
                withheld_amount(required(&baseline, index)?)?
                    == withheld_amount(required(&collected, index)?)?,
                "Collection changed LP transfer-fee withholding"
            );
        }
        let destination = if withdraw { 9 + i } else { 3 + i };
        ensure!(
            withheld_amount(required(&baseline, destination)?)?
                .checked_sub(withheld_amount(required(&prep, destination)?)?)
                == Some(fee),
            "LP withheld fee token{i}"
        );
        let gross = if i == 0 { p.creator_fees_token0 } else { p.creator_fees_token1 };
        let (payout, shared) = split_creator_fee(gross, rate)?;
        let (ca, cb, fa, fb) = if i == 0 {
            (
                a.creator_fees_token0,
                b.creator_fees_token0,
                a.protocol_fees_token0,
                b.protocol_fees_token0,
            )
        } else {
            (
                a.creator_fees_token1,
                b.creator_fees_token1,
                a.protocol_fees_token1,
                b.protocol_fees_token1,
            )
        };
        ensure!(
            ca == gross && cb == 0 && fa.checked_add(shared) == Some(fb),
            "Collection fee counters token{i}"
        );
        ensure!(
            amount(&baseline, 3 + i, &p, user)?.checked_sub(amount(&collected, 3 + i, &p, user)?)
                == Some(payout),
            "Collection vault delta token{i}"
        );
        let transfer = transfer_fee(required(&prep, 7 + i)?, p.recent_epoch, payout)?;
        ensure!(
            amount(&collected, 5 + i, &p, user)?.checked_sub(amount(&baseline, 5 + i, &p, user)?)
                == Some(payout - transfer),
            "Creator receipt token{i}"
        );
        ensure!(
            withheld_amount(required(&collected, 5 + i)?)?
                .checked_sub(withheld_amount(required(&baseline, 5 + i)?)?)
                == Some(transfer),
            "Creator withheld token{i}"
        );
    }
    ensure!(
        unchanged(&prep[1..3], &baseline[1..3]) && unchanged(&prep[1..3], &collected[1..3]),
        "Config/share changed"
    );
    ensure!(
        unchanged(&baseline[11..13], &collected[11..13]),
        "Collection changed LP account or mint"
    );
    ensure!(unchanged(&baseline[7..9], &collected[7..9]), "Underlying mints changed");
    // Independently decode LP supply and collection counters through the parser SDK.
    for (raw, expected) in [(&baseline, &a), (&collected, &b)] {
        let account = required(raw, 0)?;
        let event = sol_parser_sdk::accounts::parse_account_unified(
            &sol_parser_sdk::accounts::AccountData {
                pubkey: pool,
                owner: account.owner,
                data: account.data.clone(),
                lamports: account.lamports,
                executable: account.executable,
                rent_epoch: account.rent_epoch,
            },
            Default::default(),
            None,
        )
        .context("Parser pool")?;
        let sol_parser_sdk::core::events::DexEvent::RaydiumCpmmPoolStateAccount(e) = event else {
            anyhow::bail!("Wrong parser event")
        };
        ensure!(
            e.pool_state.lp_supply == expected.lp_supply
                && e.pool_state.protocol_fees_token_0 == expected.protocol_fees_token0
                && e.pool_state.protocol_fees_token_1 == expected.protocol_fees_token1
                && e.pool_state.creator_fees_token_0 == expected.creator_fees_token0
                && e.pool_state.creator_fees_token_1 == expected.creator_fees_token1,
            "Parser/trade pool mismatch"
        );
    }
    Ok(())
}
async fn compare(
    rpc: &SolanaRpcClient,
    pool_address: Pubkey,
    payer: Pubkey,
    permissionless: bool,
    withdraw: bool,
) -> Result<Value> {
    for attempt in 0..4 {
        let pool = fetch_pool_state(rpc, &pool_address).await?;
        let wallet = Arc::new(Keypair::new());
        let user = wallet.pubkey();
        let creator = pool.pool_creator;
        let collect = if permissionless {
            collect_creator_fee_permissionless(&payer, &pool_address, &pool)
        } else {
            collect_creator_fee(&pool_address, &pool)
        };
        let spl = sol_trade_sdk::constants::TOKEN_PROGRAM;
        let keys = [
            pool_address,
            pool.amm_config,
            get_creator_fee_share_pda(&creator, &pool.amm_config),
            pool.token0_vault,
            pool.token1_vault,
            collect.accounts[8].pubkey,
            collect.accounts[9].pubkey,
            pool.token0_mint,
            pool.token1_mint,
            ata(&user, &pool.token0_mint, &pool.token0_program, false),
            ata(&user, &pool.token1_mint, &pool.token1_program, false),
            ata(&user, &pool.lp_mint, &spl, false),
            pool.lp_mint,
        ];
        let before =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        let initial = decode_pool(required(&before.value, 0)?)?;
        ensure!(
            initial.pool_creator == creator && initial.amm_config == pool.amm_config,
            "Pool identity changed"
        );
        let config = decode_config(required(&before.value, 1)?)?;
        let epoch = rpc.get_epoch_info().await?.epoch;
        let protocol = protocol_params(pool_address, &initial, &config, &before.value, epoch)?;
        let mut params = swap_params(
            wallet,
            TradeType::Buy,
            pool.token1_mint,
            pool.token0_mint,
            1_000_000,
            0,
            DexParamEnum::RaydiumCpmm(protocol),
        );
        params.create_input_mint_ata = false;
        params.create_output_mint_ata = false;
        let mut setup =
            vec![solana_system_interface::instruction::transfer(&creator, &user, 20_000_000)];
        for (mint, program) in [
            (pool.token0_mint, pool.token0_program),
            (pool.token1_mint, pool.token1_program),
            (pool.lp_mint, spl),
        ] {
            setup.extend(create_ata(&user, &user, &mint, &program, false));
        }
        setup.push(spl_token_2022_interface::instruction::transfer_checked(
            &pool.token1_program,
            &keys[6],
            &pool.token1_mint,
            &keys[10],
            &creator,
            &[],
            2_000_000,
            pool.mint1_decimals,
        )?);
        setup.extend(RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await?);
        let funded = simulate(rpc, &setup, creator, &keys, before.context.slot)
            .await
            .context("LP funding swap")?;
        let p = decode_pool(required(&funded.accounts, 0)?)?;
        let protocol = protocol_params(pool_address, &p, &config, &funded.accounts, epoch)?;
        // Half the affordable LP position leaves ample room for inverse transfer fees.
        let lp = u64::try_from(
            (u128::from(amount(&funded.accounts, 9, &p, user)?) * u128::from(p.lp_supply)
                / u128::from(protocol.base_reserve))
            .min(
                u128::from(amount(&funded.accounts, 10, &p, user)?) * u128::from(p.lp_supply)
                    / u128::from(protocol.quote_reserve),
            ) / 2,
        )?;
        ensure!(lp > 1, "Insufficient simulated LP funding");
        let prepared = if withdraw {
            setup.push(lp_instruction(pool_address, &p, user, lp, false));
            simulate(rpc, &setup, creator, &keys, funded.slot)
                .await
                .context("Prepare LP for withdrawal")?
        } else {
            funded
        };
        let p = decode_pool(required(&prepared.accounts, 0)?)?;
        let target_lp = if withdraw { lp / 2 } else { lp };
        let mut baseline_ixs = setup.clone();
        baseline_ixs.push(lp_instruction(pool_address, &p, user, target_lp, withdraw));
        let mut collected_ixs = setup;
        collected_ixs.push(collect);
        collected_ixs.push(lp_instruction(pool_address, &p, user, target_lp, withdraw));
        let baseline = simulate(rpc, &baseline_ixs, creator, &keys, prepared.slot)
            .await
            .context("Baseline LP")?;
        let collected = simulate(rpc, &collected_ixs, creator, &keys, baseline.slot)
            .await
            .context("Collection plus LP")?;
        let after =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        if after.context.slot < collected.slot || !unchanged(&before.value, &after.value) {
            ensure!(attempt < 3, "Pool changed during all LP comparison attempts");
            continue;
        }
        ensure!(p.recent_epoch == epoch, "Epoch changed");
        let rate = resolve_creator_fee_share_rate(
            &config,
            &creator,
            &p.amm_config,
            prepared.accounts[2].as_ref(),
        )?;
        let r = json!({"pool":pool_address.to_string(),"trader":user.to_string(),"permissionless":permissionless,"withdraw":withdraw,"lp_amount":target_lp.to_string(),"share_rate":rate,
            "token0_delta":(i128::from(amount(&baseline.accounts,9,&p,user)?)-i128::from(amount(&prepared.accounts,9,&p,user)?)).to_string(),
            "token1_delta":(i128::from(amount(&baseline.accounts,10,&p,user)?)-i128::from(amount(&prepared.accounts,10,&p,user)?)).to_string(),
            "lp_delta":(i128::from(amount(&baseline.accounts,11,&p,user)?)-i128::from(amount(&prepared.accounts,11,&p,user)?)).to_string(),
            "snapshot_slot":before.context.slot,"prepared_slot":prepared.slot,"baseline_slot":baseline.slot,"collected_slot":collected.slot,"verification_slot":after.context.slot,
            "baseline_units":baseline.units,"collected_units":collected.units,"baseline_logs":baseline.logs,"collected_logs":collected.logs,
            "prepared_accounts":snapshot(&keys,&prepared.accounts),"baseline_accounts":snapshot(&keys,&baseline.accounts),"collected_accounts":snapshot(&keys,&collected.accounts)});
        verify_row(&r)?;
        return Ok(r);
    }
    unreachable!()
}
pub fn verify_saved_report(report: &Value) -> Result<()> {
    ensure!(
        report["broadcast"] == false && report["signature_verification"] == false,
        "Unexpected execution mode"
    );
    let rows = report["comparisons"].as_array().context("comparisons")?;
    ensure!(rows.len() == 4, "Expected four LP comparisons");
    let mut modes = std::collections::HashSet::new();
    for r in rows {
        ensure!(
            modes.insert((r["permissionless"].as_bool(), r["withdraw"].as_bool())),
            "Duplicate mode"
        );
        verify_row(r)?;
    }
    Ok(())
}
pub async fn run_from_env() -> Result<Value> {
    let url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let pool = std::env::var("CPMM_POOL")
        .unwrap_or_else(|_| "EjqGHevzN2pgaTf5CZnbMBuiYdPpWYx4pmeY7abwqyrM".into())
        .parse()?;
    let payer = std::env::var("CPMM_PAYER").unwrap_or_else(|_| DEFAULT_PAYER.into()).parse()?;
    let rpc = SolanaRpcClient::new_with_commitment(url, CommitmentConfig::confirmed());
    let mut comparisons = Vec::new();
    for permissionless in [false, true] {
        for withdraw in [false, true] {
            comparisons.push(compare(&rpc, pool, payer, permissionless, withdraw).await?);
        }
    }
    let report = json!({"kind":"mainnet_collection_lp_comparison","broadcast":false,"signature_verification":false,"comparisons":comparisons});
    verify_saved_report(&report)?;
    if let Ok(path) = std::env::var("CPMM_REPORT_PATH") {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
    }
    Ok(report)
}
