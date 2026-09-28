//! Meteora DLMM pairs: the SDK's quote from accounts read together, against the
//! output `swap2` credits in the simulation.

use base64::Engine;
use sol_trade_sdk::{
    common::SolanaRpcClient,
    constants::WSOL_TOKEN_ACCOUNT,
    instruction::{
        meteora_dlmm::{swap2, MeteoraDlmmSwap2Accounts},
        utils::meteora_dlmm::{
            bin_array_pda, bin_id_to_bin_array_index, bitmap_extension_pda, decode_lb_pair,
            BinArrayBitmapExtension,
        },
    },
    trading::core::params::{DlmmQuoteAccounts, MeteoraDlmmParams, CLOCK_SYSVAR},
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::{v0, VersionedMessage};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;

use super::{
    ata, create_ata_idempotent, input_source, redact, snapshot, snapshot_since, swap_units,
    sync_native, token_amount, Check, Error, Snapshot, SwapSide, ATTEMPTS, VAULT_FRACTIONS,
};

/// Bin arrays holding liquidity read each way on the first try.
const ARRAYS_EACH_WAY: usize = 4;

pub(crate) async fn check_dlmm_pool(
    rpc: &SolanaRpcClient,
    funder: Pubkey,
    pool: Pubkey,
) -> Result<Vec<&'static str>, Error> {
    let first = snapshot(rpc, &[pool]).await?;
    let pair = decode_lb_pair(&first.accounts.get(&pool).ok_or("no pair")?.data)?;
    println!(
        "{pool}: {} / {} bin_step={} collect_fee_mode={} function_type={} limit_orders={:?} active_array={} pair_type={}",
        pair.token_x_mint,
        pair.token_y_mint,
        pair.bin_step,
        pair.collect_fee_mode,
        pair.function_type,
        pair.support_limit_order().ok(),
        bin_id_to_bin_array_index(pair.active_id),
        pair.pair_type,
    );
    let mints = snapshot(rpc, &[pair.token_x_mint, pair.token_y_mint]).await?;
    let program_of = |mint: &Pubkey| mints.accounts.get(mint).map(|a| a.owner);
    let (program_x, program_y) = (
        program_of(&pair.token_x_mint).ok_or("no mint x")?,
        program_of(&pair.token_y_mint).ok_or("no mint y")?,
    );
    let mut outcomes = Vec::new();
    for swap_for_y in [true, false] {
        let side = if swap_for_y {
            (
                pair.token_x_mint,
                pair.token_y_mint,
                program_x,
                program_y,
                pair.reserve_x,
                pair.reserve_y,
            )
        } else {
            (
                pair.token_y_mint,
                pair.token_x_mint,
                program_y,
                program_x,
                pair.reserve_y,
                pair.reserve_x,
            )
        };
        let (input_mint, output_mint, input_program, output_program, input_vault, output_vault) =
            side;
        let (owner, input_account, available) = match input_source(
            rpc,
            funder,
            input_mint,
            input_program,
            &[pair.reserve_x, pair.reserve_y],
        )
        .await
        {
            Ok(source) => source,
            Err(err) => {
                println!("  swap_for_y={swap_for_y}: no input source: {err}");
                outcomes.push("no input source");
                continue;
            }
        };
        let reserve = token_amount(&rpc.get_account_data(&input_vault).await?).unwrap_or(0);
        let mut amounts: Vec<u64> =
            VAULT_FRACTIONS.iter().map(|f| ((reserve as f64) * f) as u64).collect();
        // Also a swap that reaches the nearest limit order, and one past it.
        if let Some(amount) = limit_order_amount(rpc, pool, input_mint, available).await? {
            println!("  swap_for_y={swap_for_y}: limit orders filled from {amount}");
            amounts.extend([amount, amount.saturating_mul(2)]);
        }
        for amount in amounts {
            if amount == 0 || amount > available {
                println!("  swap_for_y={swap_for_y} in={amount}: skipped, holder has {available}");
                outcomes.push("skipped amount");
                continue;
            }
            let side = SwapSide {
                zero_for_one: swap_for_y,
                input_mint,
                output_mint,
                input_program,
                output_program,
                input_vault,
                output_vault,
                owner,
                input_account,
            };
            let label = match check_dlmm_swap(rpc, funder, pool, side, amount).await {
                Ok(Check::Match { quoted, arrays, limit_order_ticks, cu }) => {
                    let orders = limit_order_ticks > 0;
                    println!(
                        "  swap_for_y={swap_for_y} in={amount}: MATCH out={quoted} arrays={arrays} limit_orders={orders} cu={cu:?}"
                    );
                    if orders {
                        "match with limit orders"
                    } else {
                        "match"
                    }
                }
                Ok(Check::Mismatch { quoted, simulated }) => {
                    println!(
                        "  swap_for_y={swap_for_y} in={amount}: MISMATCH quoted={quoted} simulated={simulated} diff={}",
                        quoted as i128 - simulated as i128
                    );
                    "MISMATCH"
                }
                Ok(Check::BothRejected { quote, simulation }) => {
                    println!("  swap_for_y={swap_for_y} in={amount}: both reject (quote {quote}; program {simulation})");
                    "both reject"
                }
                Ok(Check::OnlyQuoteRejected { quote, simulated }) => {
                    println!("  swap_for_y={swap_for_y} in={amount}: QUOTE REJECTS ({quote}) but program gives {simulated}");
                    "QUOTE ONLY REJECTS"
                }
                Ok(Check::OnlyProgramRejected { quoted, simulation }) => {
                    println!("  swap_for_y={swap_for_y} in={amount}: PROGRAM REJECTS ({simulation}) but quote gives {quoted}");
                    "PROGRAM ONLY REJECTS"
                }
                Ok(Check::Unsettled) => {
                    println!("  swap_for_y={swap_for_y} in={amount}: pair kept moving");
                    "unsettled"
                }
                Err(err) => {
                    println!(
                        "  swap_for_y={swap_for_y} in={amount}: error {}",
                        redact(&err.to_string())
                    );
                    "error"
                }
            };
            outcomes.push(label);
        }
    }
    Ok(outcomes)
}

/// The pair as a quote reads it now, with bin arrays holding liquidity read
/// `arrays_each_way` each way.
async fn load_pair(
    rpc: &SolanaRpcClient,
    pool: Pubkey,
    arrays_each_way: usize,
) -> Result<MeteoraDlmmParams, Error> {
    let bitmap = bitmap_extension_pda(&pool);
    let head = snapshot(rpc, &[pool, bitmap]).await?;
    let pair = decode_lb_pair(&head.accounts.get(&pool).ok_or("no pair")?.data)?;
    let extension =
        head.accounts.get(&bitmap).map(|a| BinArrayBitmapExtension::decode(&a.data)).transpose()?;
    let keys: Vec<Pubkey> =
        MeteoraDlmmParams::quote_account_keys(&pool, &pair, extension.as_ref(), arrays_each_way)
            .into_iter()
            .filter(|key| *key != CLOCK_SYSVAR)
            .collect();
    let read = snapshot(rpc, &keys).await?;
    let data = |key: &Pubkey| read.accounts.get(key).map(|a| a.data.as_slice());
    let mint = |key: &Pubkey| {
        read.accounts.get(key).map(|a| (a.owner, a.data.as_slice())).ok_or("mint missing")
    };
    let clock = clock_bytes(&read);
    Ok(MeteoraDlmmParams::from_quote_accounts(
        pool,
        &DlmmQuoteAccounts {
            lb_pair: data(&pool).ok_or("no pair")?,
            bitmap_extension: data(&bitmap),
            bin_arrays: keys[4..].iter().filter_map(data).collect(),
            token_x_mint: mint(&pair.token_x_mint)?,
            token_y_mint: mint(&pair.token_y_mint)?,
            clock: &clock,
        },
    )?)
}

/// Whether the quote of `amount` fills limit orders: it changes once the
/// pair's swaps leave them out, as a liquidity mining pair's do.
fn fills_limit_orders(params: &MeteoraDlmmParams, input: &Pubkey, amount: u64) -> bool {
    let Ok(with) = params.quote_exact_in(input, amount) else {
        return false;
    };
    let mut without = params.clone();
    if let Some(state) = without.quote_state.as_mut() {
        state.pair.function_type = 1;
    }
    without.quote_exact_in(input, amount).map_or(true, |q| q.amount_out != with.amount_out)
}

/// The smallest doubling amount (up to `available`) whose quote fills a limit order.
async fn limit_order_amount(
    rpc: &SolanaRpcClient,
    pool: Pubkey,
    input: Pubkey,
    available: u64,
) -> Result<Option<u64>, Error> {
    let params = load_pair(rpc, pool, 2 * ARRAYS_EACH_WAY).await?;
    let mut amount = 1_000u64;
    while amount <= available {
        match params.quote_exact_in(&input, amount) {
            Ok(_) if fills_limit_orders(&params, &input, amount) => return Ok(Some(amount)),
            Ok(_) => {}
            Err(err) if err.to_string().contains("pays nothing") => {}
            Err(_) => return Ok(None),
        }
        amount = amount.saturating_mul(2);
    }
    Ok(None)
}

/// The clock as the SDK reads it: slot, epoch and unix time.
fn clock_bytes(read: &Snapshot) -> Vec<u8> {
    let mut clock = vec![0u8; 40];
    clock[0..8].copy_from_slice(&read.slot.to_le_bytes());
    clock[16..24].copy_from_slice(&read.epoch.to_le_bytes());
    clock[32..40].copy_from_slice(&(read.unix_timestamp as i64).to_le_bytes());
    clock
}

/// Writes a matched case as a fixture: the raw accounts the quote read, the
/// clock it read them at, and the output the program credited.
#[allow(clippy::too_many_arguments)]
fn dump_case(
    dir: &str,
    pool: Pubkey,
    side: &SwapSide,
    amount: u64,
    amount_out: u64,
    read: &Snapshot,
    params: &MeteoraDlmmParams,
    bin_arrays: &[Pubkey],
) -> Result<(), Error> {
    let b64 = |key: &Pubkey| {
        read.accounts.get(key).map(|a| base64::engine::general_purpose::STANDARD.encode(&a.data))
    };
    let mint = |key: &Pubkey| {
        let account = &read.accounts[key];
        serde_json::json!({
            "owner": account.owner.to_string(),
            "data": base64::engine::general_purpose::STANDARD.encode(&account.data),
        })
    };
    let case = serde_json::json!({
        "pool": pool.to_string(),
        "swap_for_y": side.zero_for_one,
        "amount_in": amount,
        "amount_out": amount_out,
        "slot": read.slot,
        "epoch": read.epoch,
        "unix_timestamp": read.unix_timestamp,
        "bin_arrays_used": bin_arrays.iter().map(|key| key.to_string()).collect::<Vec<_>>(),
        "accounts": {
            "lb_pair": b64(&pool),
            "bitmap_extension": b64(&bitmap_extension_pda(&pool)),
            "token_x_mint": mint(&params.token_x_mint),
            "token_y_mint": mint(&params.token_y_mint),
            "bin_arrays": bin_arrays.iter().map(b64).collect::<Vec<_>>(),
        },
    });
    let direction = if side.zero_for_one { "x2y" } else { "y2x" };
    std::fs::write(
        format!("{dir}/{pool}-{direction}-{amount}.json"),
        serde_json::to_string_pretty(&case)?,
    )?;
    Ok(())
}

async fn check_dlmm_swap(
    rpc: &SolanaRpcClient,
    funder: Pubkey,
    pool: Pubkey,
    side: SwapSide,
    amount: u64,
) -> Result<Check, Error> {
    let bitmap = bitmap_extension_pda(&pool);
    let mut arrays_each_way = ARRAYS_EACH_WAY;
    for _ in 0..ATTEMPTS {
        // Read everything the quote needs at one slot.
        let head = snapshot(rpc, &[pool, bitmap]).await?;
        let pair = decode_lb_pair(&head.accounts.get(&pool).ok_or("no pair")?.data)?;
        let extension = head
            .accounts
            .get(&bitmap)
            .map(|a| BinArrayBitmapExtension::decode(&a.data))
            .transpose()?;
        let keys: Vec<Pubkey> = MeteoraDlmmParams::quote_account_keys(
            &pool,
            &pair,
            extension.as_ref(),
            arrays_each_way,
        )
        .into_iter()
        .filter(|key| *key != CLOCK_SYSVAR)
        .collect();
        let before = snapshot(rpc, &keys).await?;
        let data = |key: &Pubkey| before.accounts.get(key).map(|a| a.data.as_slice());
        let mint = |key: &Pubkey| {
            before.accounts.get(key).map(|a| (a.owner, a.data.as_slice())).ok_or("mint missing")
        };
        let clock = clock_bytes(&before);
        let params = MeteoraDlmmParams::from_quote_accounts(
            pool,
            &DlmmQuoteAccounts {
                lb_pair: data(&pool).ok_or("no pair")?,
                bitmap_extension: data(&bitmap),
                // After the pair, bitmap extension and mints (the clock is left out).
                bin_arrays: keys[4..].iter().filter_map(data).collect(),
                token_x_mint: mint(&pair.token_x_mint)?,
                token_y_mint: mint(&pair.token_y_mint)?,
                clock: &clock,
            },
        )?;
        let quote = params.quote_exact_in(&side.input_mint, amount);
        if let Err(err) = &quote {
            if err.to_string().contains("needs bin array") {
                arrays_each_way += ARRAYS_EACH_WAY;
                continue;
            }
        }
        // Accounts the swap reads: those the quote walked, else the ones ahead.
        let bin_arrays: Vec<Pubkey> = match &quote {
            Ok(q) => q.bin_arrays.clone(),
            Err(_) => {
                let state = params.quote_state.as_ref().ok_or("no quote state")?;
                let start = bin_id_to_bin_array_index(state.pair.active_id);
                state
                    .pair
                    .liquid_bin_arrays(state.bitmap_extension.as_ref(), start, side.zero_for_one, 3)
                    .iter()
                    .map(|index| bin_array_pda(&pool, *index))
                    .collect()
            }
        };
        if bin_arrays.is_empty() {
            return Ok(Check::BothRejected {
                quote: quote.err().map(|e| e.to_string()).unwrap_or_default(),
                simulation: "no bin arrays to pass".to_string(),
            });
        }
        let receiver = Keypair::new().pubkey();
        let output_account = ata(&receiver, &side.output_mint, &side.output_program);
        let mut instructions = vec![ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)];
        if side.input_mint == WSOL_TOKEN_ACCOUNT {
            instructions.push(create_ata_idempotent(
                &funder,
                &funder,
                &side.input_mint,
                &side.input_program,
            ));
            instructions.push(system_instruction::transfer(&funder, &side.input_account, amount));
            instructions.push(sync_native(&side.input_account));
        }
        instructions.push(create_ata_idempotent(
            &funder,
            &receiver,
            &side.output_mint,
            &side.output_program,
        ));
        instructions.push(swap2(
            &MeteoraDlmmSwap2Accounts {
                lb_pair: pool,
                bitmap_extension: params.bitmap_extension,
                reserve_x: params.reserve_x,
                reserve_y: params.reserve_y,
                user_token_in: side.input_account,
                user_token_out: output_account,
                token_x_mint: params.token_x_mint,
                token_y_mint: params.token_y_mint,
                oracle: params.oracle,
                user: side.owner,
                token_x_program: params.token_x_program,
                token_y_program: params.token_y_program,
                bin_arrays: bin_arrays.clone(),
            },
            amount,
            0,
        )?);
        let blockhash = rpc.get_latest_blockhash().await?;
        let message = v0::Message::try_compile(&funder, &instructions, &[], blockhash)?;
        let tx = VersionedTransaction {
            signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
            message: VersionedMessage::V0(message),
        };
        let simulation = match rpc
            .simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::confirmed()),
                    encoding: Some(UiTransactionEncoding::Base64),
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        addresses: vec![output_account.to_string()],
                    }),
                    min_context_slot: Some(before.slot),
                    inner_instructions: false,
                },
            )
            .await
        {
            Ok(simulation) => simulation,
            // A node behind the read answers so; ask again.
            Err(err) if err.to_string().contains("-32016") => continue,
            Err(err) => return Err(err.into()),
        };
        // The simulation used the state read only if the pair did not move
        // between the read and a read no older than the simulation.
        let after = match snapshot_since(rpc, &keys, Some(simulation.context.slot)).await {
            Ok(after) => after,
            Err(_) => continue,
        };
        let unchanged = keys.iter().all(|key| {
            before.accounts.get(key).map(|a| &a.data) == after.accounts.get(key).map(|a| &a.data)
        });
        if !unchanged {
            continue;
        }
        if simulation.value.err.is_some() && quote.is_ok() {
            for log in simulation
                .value
                .logs
                .iter()
                .flatten()
                .rev()
                .take(12)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                println!("    program log: {log}");
            }
        }
        let simulated = match &simulation.value.err {
            Some(err) => Err(format!(
                "{err:?} {}",
                simulation
                    .value
                    .logs
                    .as_ref()
                    .and_then(|logs| logs.iter().rev().find(|l| l.contains("rror")).cloned())
                    .unwrap_or_default()
            )),
            None => {
                let account = simulation
                    .value
                    .accounts
                    .as_ref()
                    .and_then(|a| a.first().cloned().flatten())
                    .ok_or("no output account")?;
                let data = match &account.data {
                    solana_account_decoder::UiAccountData::Binary(raw, _) => {
                        base64::engine::general_purpose::STANDARD.decode(raw)?
                    }
                    _ => return Err("unexpected account encoding".into()),
                };
                Ok(token_amount(&data).ok_or("short token account")?)
            }
        };
        return Ok(match (quote, simulated) {
            (Ok(q), Ok(simulated)) if q.amount_out == simulated => {
                let orders = fills_limit_orders(&params, &side.input_mint, amount);
                println!("    bins={} limit_orders={orders}", q.bins);
                if let Ok(dir) = std::env::var("DUMP_DIR") {
                    dump_case(
                        &dir,
                        pool,
                        &side,
                        amount,
                        simulated,
                        &before,
                        &params,
                        &q.bin_arrays,
                    )?;
                }
                Check::Match {
                    quoted: q.amount_out,
                    arrays: q.bin_arrays.len(),
                    limit_order_ticks: usize::from(orders),
                    cu: swap_units(&simulation.value.logs),
                }
            }
            (Ok(q), Ok(simulated)) => Check::Mismatch { quoted: q.amount_out, simulated },
            (Err(quote), Err(simulation)) => {
                Check::BothRejected { quote: quote.to_string(), simulation }
            }
            (Err(quote), Ok(simulated)) => {
                Check::OnlyQuoteRejected { quote: quote.to_string(), simulated }
            }
            (Ok(q), Err(simulation)) => {
                Check::OnlyProgramRejected { quoted: q.amount_out, simulation }
            }
        });
    }
    Ok(Check::Unsettled)
}
