//! Checks the SDK's exact hop quotes against the programs themselves.
//!
//! For each pool, direction and amount it reads the accounts a swap touches in
//! one call, quotes the swap with the SDK, then dry-runs the real swap
//! instruction in `simulateTransaction` (`sigVerify=false`: a large SOL holder
//! pays, the largest holder of the input token swaps) and compares the output
//! credited to a fresh account. When the pool moves between the read and the
//! simulation the check is repeated. Nothing is signed or sent.
//!
//! ```bash
//! export RPC_URL=https://mainnet.helius-rpc.com/?api-key=...
//! cargo run -p verify_hop_quotes -- clmm <pool> [<pool> ...]
//! ```

use base64::Engine;
use sol_trade_sdk::{
    common::SolanaRpcClient,
    constants::{TOKEN_PROGRAM, TOKEN_PROGRAM_2022, WSOL_TOKEN_ACCOUNT},
    instruction::{
        raydium_clmm::{swap_v2, RaydiumClmmSwapV2Accounts, RaydiumClmmSwapV2Args},
        utils::raydium_clmm::{tick_array_bitmap_extension, tick_array_pda},
    },
    trading::core::params::token_transfer_fee_for_epoch,
    utils::calc::raydium_clmm::{
        config::AmmConfig,
        quote::{quote_exact_in, QuoteError},
        state::{PoolState, TickArrayBitmapExtension, TickArrayState},
    },
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::{v0, VersionedMessage};
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::collections::BTreeMap;
use std::str::FromStr;

const ATA_PROGRAM: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
const CLOCK: Pubkey = pubkey!("SysvarC1ock11111111111111111111111111111111");
const FUNDERS: [Pubkey; 4] = [
    pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"),
    pubkey!("5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9"),
    pubkey!("2ojv9BAiHUrvsm9gxDeBFNCuoK1xKdcHGa5Y6XD7Tcuv"),
    pubkey!("FWznbcNXWQuHTawe9RxvQ2LdCENssh12dsznf4RiouN5"),
];
/// Swap sizes as fractions of the input vault: from one tick to many.
const VAULT_FRACTIONS: [f64; 4] = [0.0001, 0.001, 0.01, 0.05];
const ATTEMPTS: usize = 6;
/// Tick arrays read around the current one on the first try.
const ARRAYS_EACH_SIDE: i32 = 4;

type Error = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (venue, pools) = args.split_first().ok_or("usage: verify_hop_quotes clmm <pool>...")?;
    if venue != "clmm" {
        return Err(format!("unknown venue {venue}").into());
    }
    let rpc_url = std::env::var("RPC_URL").map_err(|_| "RPC_URL is not set")?;
    let rpc = SolanaRpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());
    let funder = pick_funder(&rpc).await?;

    let mut totals = BTreeMap::<&str, usize>::new();
    for pool in pools {
        let pool = Pubkey::from_str(pool)?;
        match check_clmm_pool(&rpc, funder, pool).await {
            Ok(outcomes) => {
                for outcome in outcomes {
                    *totals.entry(outcome).or_default() += 1;
                }
            }
            Err(err) => {
                println!("{pool}: skipped: {}", redact(&err.to_string()));
                *totals.entry("skipped pool").or_default() += 1;
            }
        }
    }
    println!("totals: {totals:?}");
    Ok(())
}

/// Endpoint URLs in errors carry API keys.
fn redact(text: &str) -> String {
    match text.find("api-key=") {
        Some(start) => {
            let end = text[start..].find(|c: char| c == '&' || c.is_whitespace() || c == '"');
            let end = end.map(|e| start + e).unwrap_or(text.len());
            format!("{}api-key=REDACTED{}", &text[..start], &text[end..])
        }
        None => text.to_string(),
    }
}

async fn pick_funder(rpc: &SolanaRpcClient) -> Result<Pubkey, Error> {
    for candidate in FUNDERS {
        if rpc.get_balance(&candidate).await? >= 10_000 * 1_000_000_000 {
            return Ok(candidate);
        }
    }
    Err("no simulation funder with enough SOL".into())
}

struct Snapshot {
    slot: u64,
    unix_timestamp: u64,
    epoch: u64,
    accounts: BTreeMap<Pubkey, Account>,
}

async fn snapshot(rpc: &SolanaRpcClient, keys: &[Pubkey]) -> Result<Snapshot, Error> {
    snapshot_since(rpc, keys, None).await
}

/// Accounts at a slot no older than `min_slot`: the RPC balances nodes that
/// can lag each other, so an unconstrained read may predate the simulation.
async fn snapshot_since(
    rpc: &SolanaRpcClient,
    keys: &[Pubkey],
    min_slot: Option<u64>,
) -> Result<Snapshot, Error> {
    let mut all = vec![CLOCK];
    all.extend_from_slice(keys);
    let mut accounts = BTreeMap::new();
    let mut slot = 0;
    let mut clock = None;
    for chunk in all.chunks(100) {
        let response = rpc
            .get_multiple_ui_accounts_with_config(
                chunk,
                RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    data_slice: None,
                    commitment: Some(CommitmentConfig::processed()),
                    min_context_slot: min_slot,
                },
            )
            .await?;
        if min_slot.is_some_and(|min| response.context.slot < min) {
            return Err("RPC answered from an older slot".into());
        }
        slot = slot.max(response.context.slot);
        for (key, account) in chunk.iter().zip(response.value) {
            if let Some(account) = account.and_then(to_account) {
                if *key == CLOCK {
                    clock = Some(account.data.clone());
                } else {
                    accounts.insert(*key, account);
                }
            }
        }
    }
    let clock = clock.ok_or("no clock")?;
    Ok(Snapshot {
        slot,
        epoch: u64::from_le_bytes(clock[16..24].try_into()?),
        unix_timestamp: i64::from_le_bytes(clock[32..40].try_into()?) as u64,
        accounts,
    })
}

fn to_account(ui: solana_account_decoder::UiAccount) -> Option<Account> {
    let data = match &ui.data {
        solana_account_decoder::UiAccountData::Binary(raw, UiAccountEncoding::Base64) => {
            base64::engine::general_purpose::STANDARD.decode(raw).ok()?
        }
        _ => return None,
    };
    Some(Account {
        lamports: ui.lamports,
        data,
        owner: Pubkey::from_str(&ui.owner).ok()?,
        executable: ui.executable,
        rent_epoch: ui.rent_epoch,
    })
}

fn ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    )
    .0
}

fn create_ata_idempotent(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        ATA_PROGRAM,
        &[1],
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(ata(owner, mint, token_program), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
    )
}

fn sync_native(account: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(TOKEN_PROGRAM, &[17], vec![AccountMeta::new(*account, false)])
}

fn token_amount(data: &[u8]) -> Option<u64> {
    data.get(64..72).map(|raw| u64::from_le_bytes(raw.try_into().unwrap()))
}

/// Where the input comes from: the funder's wrapped SOL, or the largest holder
/// of the token that is not one of `exclude` (the pool's own vaults).
async fn input_source(
    rpc: &SolanaRpcClient,
    funder: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    exclude: &[Pubkey],
) -> Result<(Pubkey, Pubkey, u64), Error> {
    if mint == WSOL_TOKEN_ACCOUNT {
        let balance = rpc.get_balance(&funder).await?;
        return Ok((funder, ata(&funder, &mint, &token_program), balance - 5_000_000_000));
    }
    // Mints with too many holders (USDC) cannot be ranked by the RPC; the
    // funders are exchange wallets and hold them.
    let candidates: Vec<Pubkey> = match rpc.get_token_largest_accounts(&mint).await {
        Ok(holders) => holders.iter().filter_map(|h| Pubkey::from_str(&h.address).ok()).collect(),
        Err(_) => FUNDERS.iter().map(|funder| ata(funder, &mint, &token_program)).collect(),
    };
    let mut best: Option<(Pubkey, Pubkey, u64)> = None;
    for account in candidates {
        if exclude.contains(&account) {
            continue;
        }
        let Ok(data) = rpc.get_account_data(&account).await else { continue };
        let owner = Pubkey::new_from_array(data[32..64].try_into()?);
        let balance = token_amount(&data).unwrap_or(0);
        if best.is_none_or(|(_, _, b)| balance > b) {
            best = Some((owner, account, balance));
        }
    }
    best.filter(|(_, _, balance)| *balance > 0).ok_or_else(|| format!("no holder of {mint}").into())
}

async fn check_clmm_pool(
    rpc: &SolanaRpcClient,
    funder: Pubkey,
    pool: Pubkey,
) -> Result<Vec<&'static str>, Error> {
    let first = snapshot(rpc, &[pool]).await?;
    let state = PoolState::decode(&first.accounts.get(&pool).ok_or("no pool")?.data)?;
    let flags = format!(
        "fee_on={} dynamic_fee={} tick_spacing={}",
        state.fee_on,
        state.get_dynamic_fee_info().is_some(),
        state.tick_spacing
    );
    let ticks_per_array = TickArrayState::tick_count(state.tick_spacing);
    let current = TickArrayState::get_array_start_index(state.tick_current, state.tick_spacing);
    let nearby: Vec<Pubkey> = (-ARRAYS_EACH_SIDE..=ARRAYS_EACH_SIDE)
        .map(|k| tick_array_pda(&pool, current + k * ticks_per_array))
        .collect();
    let limit_order_ticks: usize = snapshot(rpc, &nearby)
        .await?
        .accounts
        .values()
        .filter_map(|account| TickArrayState::decode(&account.data).ok())
        .map(|array| array.ticks.iter().filter(|tick| tick.has_limit_orders()).count())
        .sum();
    println!(
        "{pool}: {} / {} {flags} limit_order_ticks_near_price={limit_order_ticks}",
        state.token_mint_0, state.token_mint_1
    );
    let mint_accounts = snapshot(rpc, &[state.token_mint_0, state.token_mint_1]).await?;
    let program_of = |mint: &Pubkey| mint_accounts.accounts.get(mint).map(|a| a.owner);
    let (program_0, program_1) = (
        program_of(&state.token_mint_0).ok_or("no mint 0")?,
        program_of(&state.token_mint_1).ok_or("no mint 1")?,
    );

    let mut outcomes = Vec::new();
    for zero_for_one in [true, false] {
        let (input_mint, output_mint, input_program, output_program, input_vault, output_vault) =
            if zero_for_one {
                (
                    state.token_mint_0,
                    state.token_mint_1,
                    program_0,
                    program_1,
                    state.token_vault_0,
                    state.token_vault_1,
                )
            } else {
                (
                    state.token_mint_1,
                    state.token_mint_0,
                    program_1,
                    program_0,
                    state.token_vault_1,
                    state.token_vault_0,
                )
            };
        let (owner, input_account, available) = match input_source(
            rpc,
            funder,
            input_mint,
            input_program,
            &[state.token_vault_0, state.token_vault_1],
        )
        .await
        {
            Ok(source) => source,
            Err(err) => {
                println!("  zero_for_one={zero_for_one}: no input source: {err}");
                outcomes.push("no input source");
                continue;
            }
        };
        let vault_balance = token_amount(&rpc.get_account_data(&input_vault).await?).unwrap_or(0);
        let mut amounts: Vec<u64> =
            VAULT_FRACTIONS.iter().map(|f| ((vault_balance as f64) * f) as u64).collect();
        // Also a swap that reaches the nearest limit order, and one past it.
        if let Some(amount) =
            limit_order_amount(rpc, pool, zero_for_one, input_mint, input_program, available)
                .await?
        {
            println!("  zero_for_one={zero_for_one}: first limit order filled from {amount}");
            amounts.extend([amount, amount.saturating_mul(2)]);
        }
        for amount in amounts {
            if amount == 0 || amount > available {
                println!(
                    "  zero_for_one={zero_for_one} in={amount}: skipped, holder has {available}"
                );
                outcomes.push("skipped amount");
                continue;
            }
            let outcome = check_clmm_swap(
                rpc,
                funder,
                pool,
                SwapSide {
                    zero_for_one,
                    input_mint,
                    output_mint,
                    input_program,
                    output_program,
                    input_vault,
                    output_vault,
                    owner,
                    input_account,
                },
                amount,
            )
            .await;
            let label = match outcome {
                Ok(Check::Match { quoted, arrays, limit_order_ticks }) => {
                    println!(
                        "  zero_for_one={zero_for_one} in={amount}: MATCH out={quoted} arrays={arrays} limit_order_ticks={limit_order_ticks}"
                    );
                    if limit_order_ticks > 0 {
                        "match with limit orders"
                    } else {
                        "match"
                    }
                }
                Ok(Check::Mismatch { quoted, simulated }) => {
                    println!(
                        "  zero_for_one={zero_for_one} in={amount}: MISMATCH quoted={quoted} simulated={simulated} diff={}",
                        quoted as i128 - simulated as i128
                    );
                    "MISMATCH"
                }
                Ok(Check::BothRejected { quote, simulation }) => {
                    println!("  zero_for_one={zero_for_one} in={amount}: both reject (quote {quote}; program {simulation})");
                    "both reject"
                }
                Ok(Check::OnlyQuoteRejected { quote, simulated }) => {
                    println!("  zero_for_one={zero_for_one} in={amount}: QUOTE REJECTS ({quote}) but program gives {simulated}");
                    "QUOTE ONLY REJECTS"
                }
                Ok(Check::OnlyProgramRejected { quoted, simulation }) => {
                    println!("  zero_for_one={zero_for_one} in={amount}: PROGRAM REJECTS ({simulation}) but quote gives {quoted}");
                    "PROGRAM ONLY REJECTS"
                }
                Ok(Check::Unsettled) => {
                    println!("  zero_for_one={zero_for_one} in={amount}: pool kept moving");
                    "unsettled"
                }
                Err(err) => {
                    println!(
                        "  zero_for_one={zero_for_one} in={amount}: error {}",
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

/// The smallest doubling amount (up to `available`) whose quote fills a limit order.
async fn limit_order_amount(
    rpc: &SolanaRpcClient,
    pool: Pubkey,
    zero_for_one: bool,
    input_mint: Pubkey,
    input_program: Pubkey,
    available: u64,
) -> Result<Option<u64>, Error> {
    let first = snapshot(rpc, &[pool]).await?;
    let state = PoolState::decode(&first.accounts[&pool].data)?;
    let ticks_per_array = TickArrayState::tick_count(state.tick_spacing);
    let current = TickArrayState::get_array_start_index(state.tick_current, state.tick_spacing);
    let bitmap = tick_array_bitmap_extension(&pool);
    let starts: Vec<i32> =
        (-ARRAYS_EACH_SIDE..=ARRAYS_EACH_SIDE).map(|k| current + k * ticks_per_array).collect();
    let mut keys = vec![pool, state.amm_config, bitmap, input_mint];
    keys.extend(starts.iter().map(|start| tick_array_pda(&pool, *start)));
    let read = snapshot(rpc, &keys).await?;
    let state = PoolState::decode(&read.accounts[&pool].data)?;
    let arrays: Vec<TickArrayState> = starts
        .iter()
        .filter_map(|start| read.accounts.get(&tick_array_pda(&pool, *start)))
        .filter_map(|account| TickArrayState::decode(&account.data).ok())
        .collect();
    if !arrays.iter().any(|array| array.ticks.iter().any(|tick| tick.has_limit_orders())) {
        return Ok(None);
    }
    let config = AmmConfig::decode(&read.accounts[&state.amm_config].data)?;
    let extension = read
        .accounts
        .get(&bitmap)
        .map(|a| TickArrayBitmapExtension::decode(&a.data))
        .transpose()?;
    let input_fee =
        token_transfer_fee_for_epoch(&read.accounts[&input_mint].data, input_program, read.epoch)?;
    let mut amount = 1_000u64;
    while amount <= available {
        match quote_exact_in(
            &state,
            &config,
            &arrays,
            extension.as_ref(),
            amount - input_fee.calculate(amount),
            zero_for_one,
            read.unix_timestamp,
        ) {
            Ok(quote) if quote.limit_order_ticks > 0 => return Ok(Some(amount)),
            Ok(_) => {}
            Err(_) => return Ok(None),
        }
        amount = amount.saturating_mul(2);
    }
    Ok(None)
}

#[derive(Clone, Copy)]
struct SwapSide {
    zero_for_one: bool,
    input_mint: Pubkey,
    output_mint: Pubkey,
    input_program: Pubkey,
    output_program: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    owner: Pubkey,
    input_account: Pubkey,
}

enum Check {
    Match { quoted: u64, arrays: usize, limit_order_ticks: usize },
    Mismatch { quoted: u64, simulated: u64 },
    BothRejected { quote: String, simulation: String },
    OnlyQuoteRejected { quote: String, simulated: u64 },
    OnlyProgramRejected { quoted: u64, simulation: String },
    Unsettled,
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
    state: &PoolState,
    tick_array_starts: &[i32],
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
        "zero_for_one": side.zero_for_one,
        "amount_in": amount,
        "amount_out": amount_out,
        "epoch": read.epoch,
        "unix_timestamp": read.unix_timestamp,
        "accounts": {
            "pool": b64(&pool),
            "amm_config": b64(&state.amm_config),
            "bitmap_extension": b64(&tick_array_bitmap_extension(&pool)),
            "token_0_mint": mint(&state.token_mint_0),
            "token_1_mint": mint(&state.token_mint_1),
            "tick_arrays": tick_array_starts.iter().map(|start| b64(&tick_array_pda(&pool, *start))).collect::<Vec<_>>(),
        },
    });
    let path =
        format!("{dir}/{pool}-{}-{amount}.json", if side.zero_for_one { "0to1" } else { "1to0" });
    std::fs::write(path, serde_json::to_string_pretty(&case)?)?;
    Ok(())
}

async fn check_clmm_swap(
    rpc: &SolanaRpcClient,
    funder: Pubkey,
    pool: Pubkey,
    side: SwapSide,
    amount: u64,
) -> Result<Check, Error> {
    let bitmap = tick_array_bitmap_extension(&pool);
    let mut extra_arrays: Vec<i32> = Vec::new();
    for _ in 0..ATTEMPTS {
        // Read everything the quote needs at one slot.
        let pool_snapshot = snapshot(rpc, &[pool]).await?;
        let state = PoolState::decode(&pool_snapshot.accounts[&pool].data)?;
        let ticks_per_array = TickArrayState::tick_count(state.tick_spacing);
        let current = TickArrayState::get_array_start_index(state.tick_current, state.tick_spacing);
        let mut starts: Vec<i32> =
            (-ARRAYS_EACH_SIDE..=ARRAYS_EACH_SIDE).map(|k| current + k * ticks_per_array).collect();
        starts.extend(&extra_arrays);
        let mut keys = vec![pool, state.amm_config, bitmap, side.input_mint, side.output_mint];
        keys.extend(starts.iter().map(|start| tick_array_pda(&pool, *start)));
        let before = snapshot(rpc, &keys).await?;
        let state = PoolState::decode(&before.accounts[&pool].data)?;
        let config = AmmConfig::decode(&before.accounts[&state.amm_config].data)?;
        let extension = before
            .accounts
            .get(&bitmap)
            .map(|a| TickArrayBitmapExtension::decode(&a.data))
            .transpose()?;
        let arrays: Vec<TickArrayState> = starts
            .iter()
            .filter_map(|start| before.accounts.get(&tick_array_pda(&pool, *start)))
            .map(|account| TickArrayState::decode(&account.data))
            .collect::<Result<_, _>>()?;
        let input_fee = token_transfer_fee_for_epoch(
            &before.accounts[&side.input_mint].data,
            side.input_program,
            before.epoch,
        )?;
        let output_fee = token_transfer_fee_for_epoch(
            &before.accounts[&side.output_mint].data,
            side.output_program,
            before.epoch,
        )?;

        let quote = quote_exact_in(
            &state,
            &config,
            &arrays,
            extension.as_ref(),
            amount - input_fee.calculate(amount),
            side.zero_for_one,
            before.unix_timestamp,
        );
        let quote = match quote {
            Err(QuoteError::MissingTickArray(start)) => {
                extra_arrays.push(start);
                continue;
            }
            other => other,
        };
        // Accounts the swap reads: those the quote crossed, else the current ones.
        let tick_arrays: Vec<Pubkey> = match &quote {
            Ok(q) => q.tick_array_start_indexes.iter().map(|s| tick_array_pda(&pool, *s)).collect(),
            Err(_) => {
                arrays.iter().take(3).map(|a| tick_array_pda(&pool, a.start_tick_index)).collect()
            }
        };
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
        instructions.push(swap_v2(
            &RaydiumClmmSwapV2Accounts {
                payer: side.owner,
                amm_config: state.amm_config,
                pool_state: pool,
                input_token_account: side.input_account,
                output_token_account: output_account,
                input_vault: side.input_vault,
                output_vault: side.output_vault,
                observation_state: state.observation_key,
                token_program: TOKEN_PROGRAM,
                token_program_2022: TOKEN_PROGRAM_2022,
                input_vault_mint: side.input_mint,
                output_vault_mint: side.output_mint,
                tick_array_bitmap_extension: extension.as_ref().map(|_| bitmap),
                tick_arrays,
            },
            RaydiumClmmSwapV2Args {
                amount,
                other_amount_threshold: 0,
                sqrt_price_limit_x64: 0,
                is_base_input: true,
            },
        )?);
        let blockhash = rpc.get_latest_blockhash().await?;
        let message = v0::Message::try_compile(&funder, &instructions, &[], blockhash)?;
        let tx = VersionedTransaction {
            signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
            message: VersionedMessage::V0(message),
        };
        let simulation = rpc
            .simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::processed()),
                    encoding: Some(UiTransactionEncoding::Base64),
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        addresses: vec![output_account.to_string()],
                    }),
                    min_context_slot: Some(before.slot),
                    inner_instructions: false,
                },
            )
            .await?;
        // The simulation used the state read only if the pool did not move
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
                    .and_then(|logs| logs.iter().rev().find(|l| l.contains("Error")).cloned())
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
            (Ok(q), Ok(simulated)) => {
                let quoted = q.amount_out - output_fee.calculate(q.amount_out);
                if quoted == simulated {
                    if let Ok(dir) = std::env::var("DUMP_DIR") {
                        dump_case(
                            &dir,
                            pool,
                            &side,
                            amount,
                            simulated,
                            &before,
                            &state,
                            &q.tick_array_start_indexes,
                        )?;
                    }
                    Check::Match {
                        quoted,
                        arrays: q.tick_array_start_indexes.len(),
                        limit_order_ticks: q.limit_order_ticks,
                    }
                } else {
                    Check::Mismatch { quoted, simulated }
                }
            }
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
