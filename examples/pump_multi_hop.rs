//! Official PumpSwap multi_hop_swap: Pump.fun curves and migrated PumpSwap pools.
//! Current-account construction, with optional unsigned RPC simulation. See --help.
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::{
    common::{
        bonding_curve::BondingCurveAccount,
        spl_associated_token_account::{
            create_associated_token_account_idempotent as create_ata,
            get_associated_token_address_with_program_id as ata,
        },
        SolanaRpcClient,
    },
    constants::{TOKEN_PROGRAM, TOKEN_PROGRAM_2022, WSOL_TOKEN_ACCOUNT as WSOL},
    instruction::{
        pump_compact_accounts::{PumpMultiHop, PumpMultiHopVenue},
        pump_route::select_pump_route,
        utils::{
            pumpfun::accounts::PUMPFUN as PUMP,
            pumpswap::{accounts::AMM_PROGRAM as AMM, decode_global_config},
            pumpswap_types::{pool_decode, POOL_DISCRIMINATOR},
        },
    },
};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Signature};
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::{collections::HashMap, io::Write, time::Duration};

struct Options {
    // Always currency -> intermediate mint(s) -> target, including for sells.
    path: Vec<Pubkey>,
    venue: PumpMultiHopVenue,
    user: Pubkey,
    sell: bool,
    amount: u64,
    min_out: u64,
    simulate: bool,
    report: Option<String>,
}

fn options() -> Result<Option<Options>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!(
            r#"Official Pump multi_hop_swap (2 or 3 hops, one swap instruction).
cargo run --example pump_multi_hop -- \
  --venue pumpfun|pumpswap --path SOL,INTERMEDIATE_MINT,TARGET_MINT \
  --side buy|sell --user WALLET_PUBKEY --amount RAW --min-out RAW \
  [--simulate] [--report NEW_FILE]

--venue selects the TARGET's current venue, not every hop's venue.
--path is always written in buy order; sell reverses the hops automatically.
Currency may be SOL/WSOL or a mint such as USDC. Amounts are integer base units:
buy: amount in currency, min-out in target; sell: amount in target, min-out in currency.
Supply a fresh minimum quote. This example does not calculate route prices.

Reads one current account batch from SOLANA_RPC_URL (default: mainnet public RPC).
Default: construct and print; --simulate also calls simulateTransaction.
No private key, signatures or broadcast. The user must have real funds for simulation.
SOL buys wrap the input only when the currency hop is a pool; curve hops use native SOL.
Pool sells leave WSOL in the output ATA; curve sells pay native SOL.
Endpoint/buyback ATAs are created idempotently; no middle-token ATA is needed.
Use subscription snapshots in production; these RPC reads are example bootstrap only."#
        );
        return Ok(None);
    }
    let mut values = HashMap::new();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        ensure!(!values.contains_key(flag.as_str()), "Duplicate {flag}");
        let value = match flag.as_str() {
            "--simulate" => "true",
            "--venue" | "--path" | "--side" | "--user" | "--amount" | "--min-out" | "--report" => {
                iter.next()
                    .filter(|v| !v.starts_with("--"))
                    .with_context(|| format!("Missing value for {flag}"))?
                    .as_str()
            }
            _ => bail!("Unknown argument {flag}; use --help"),
        };
        values.insert(flag.as_str(), value);
    }
    let required = |flag| values.get(flag).copied().with_context(|| format!("Missing {flag}"));
    let path: Vec<Pubkey> = required("--path")?
        .split(',')
        .map(|s| match s {
            "SOL" | "WSOL" => Ok(WSOL),
            _ => s.parse().context("Invalid mint in --path"),
        })
        .collect::<Result<_>>()?;
    ensure!((3..=4).contains(&path.len()), "Provide a currency and 2 or 3 hop mints");
    ensure!(
        path.iter().collect::<std::collections::HashSet<_>>().len() == path.len(),
        "Repeated mint in route"
    );
    let options = Options {
        path,
        venue: match required("--venue")? {
            "pumpfun" => PumpMultiHopVenue::Curve,
            "pumpswap" => PumpMultiHopVenue::Pool,
            _ => bail!("--venue must be pumpfun or pumpswap"),
        },
        user: required("--user")?.parse()?,
        sell: match required("--side")? {
            "buy" => false,
            "sell" => true,
            _ => bail!("--side must be buy or sell"),
        },
        amount: required("--amount")?.parse()?,
        min_out: required("--min-out")?.parse()?,
        simulate: values.contains_key("--simulate"),
        report: values.get("--report").map(|s| s.to_string()),
    };
    ensure!(options.amount > 0 && options.min_out > 0, "amount and min-out must be positive");
    if let Some(path) = &options.report {
        ensure!(!std::path::Path::new(path).exists(), "Report exists; choose a new file");
    }
    Ok(Some(options))
}

fn curve_address(mint: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &PUMP).0
}

fn pool_address(base: Pubkey, quote: Pubkey) -> Pubkey {
    let authority = Pubkey::find_program_address(&[b"pool-authority", base.as_ref()], &PUMP).0;
    Pubkey::find_program_address(
        &[b"pool", &0u16.to_le_bytes(), authority.as_ref(), base.as_ref(), quote.as_ref()],
        &AMM,
    )
    .0
}

async fn load_route(
    rpc: &SolanaRpcClient,
    options: &Options,
) -> Result<(u64, Vec<PumpMultiHop>, Pubkey)> {
    let global = Pubkey::find_program_address(&[b"global_config"], &AMM).0;
    let mut keys = options.path.clone();
    for pair in options.path.windows(2) {
        keys.extend([curve_address(pair[1]), pool_address(pair[1], pair[0])]);
    }
    keys.push(global);
    // One bank snapshot for every mint, curve, candidate pool and GlobalConfig.
    let batch =
        rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
    let accounts: HashMap<_, _> = keys.into_iter().zip(batch.value).collect();
    let get = |key: &Pubkey| {
        accounts.get(key).and_then(Option::as_ref).with_context(|| format!("Missing account {key}"))
    };
    let mut programs = HashMap::new();
    for mint in &options.path {
        let account = get(mint)?;
        ensure!(
            [TOKEN_PROGRAM, TOKEN_PROGRAM_2022].contains(&account.owner),
            "Invalid mint owner: {mint}"
        );
        spl_token_2022_interface::extension::StateWithExtensions::<
            spl_token_2022_interface::state::Mint,
        >::unpack(&account.data)
        .with_context(|| format!("Invalid mint {mint}"))?;
        programs.insert(*mint, account.owner);
    }
    let config_account = get(&global)?;
    ensure!(config_account.owner == AMM, "Invalid PumpSwap GlobalConfig owner");
    let config = decode_global_config(&config_account.data).context("Invalid GlobalConfig")?;
    let recipient = config
        .buyback_fee_recipients
        .into_iter()
        .find(|p| *p != Pubkey::default())
        .context("No current buyback recipient")?;
    let mut hops = Vec::new();
    for pair in options.path.windows(2) {
        let (quote, base) = (pair[0], pair[1]);
        let address = curve_address(base);
        let curve = if let Some(account) = accounts[&address].as_ref() {
            ensure!(
                account.owner == PUMP
                    && account.data.starts_with(&[23, 183, 248, 55, 96, 216, 172, 96]),
                "Invalid bonding curve {address}"
            );
            Some(
                BondingCurveAccount::decode_body(&account.data[8..])
                    .context("Invalid curve body")?,
            )
        } else {
            None
        };
        let hop = if let Some(curve) = curve.filter(|c| !c.complete) {
            ensure!(
                curve.effective_quote_mint() == quote,
                "Curve quote does not match --path: {base}"
            );
            PumpMultiHop {
                venue: PumpMultiHopVenue::Curve,
                address,
                base_mint: base,
                quote_mint: quote,
                base_vault: ata(&address, &base, &programs[&base]),
                quote_vault: ata(&address, &quote, &programs[&quote]),
                base_token_program: programs[&base],
                quote_token_program: programs[&quote],
                mayhem: curve.is_mayhem_mode,
                cashback: curve.is_cashback_coin,
                complete: curve.complete,
                index: 0,
                creator: curve.creator,
            }
        } else {
            let address = pool_address(base, quote);
            let account = get(&address)
                .context("Curve complete/missing and no canonical pool for this pair")?;
            ensure!(
                account.owner == AMM && account.data.starts_with(&POOL_DISCRIMINATOR),
                "Invalid PumpSwap pool"
            );
            let pool = pool_decode(&account.data[8..]).context("Invalid pool body")?;
            ensure!(pool.base_mint == base && pool.quote_mint == quote, "Pool mints mismatch");
            PumpMultiHop {
                venue: PumpMultiHopVenue::Pool,
                address,
                base_mint: base,
                quote_mint: quote,
                base_vault: pool.pool_base_token_account,
                quote_vault: pool.pool_quote_token_account,
                base_token_program: programs[&base],
                quote_token_program: programs[&quote],
                mayhem: pool.is_mayhem_mode,
                cashback: pool.is_cashback_coin,
                complete: true,
                index: pool.index,
                creator: pool.creator,
            }
        };
        hops.push(hop);
    }
    ensure!(
        hops.last().context("Empty route")?.venue == options.venue,
        "Target is currently {:?}, not requested {:?}; check --venue and refresh the route",
        hops.last().unwrap().venue,
        options.venue
    );
    Ok((batch.context.slot, hops, recipient))
}

fn build_instructions(
    options: &Options,
    mut hops: Vec<PumpMultiHop>,
    recipient: Pubkey,
) -> Result<Vec<Instruction>> {
    let currency = hops[0];
    let target = *hops.last().context("Empty route")?;
    let (input, output, input_program, output_program) = if options.sell {
        hops.reverse();
        (
            target.base_mint,
            currency.quote_mint,
            target.base_token_program,
            currency.quote_token_program,
        )
    } else {
        (
            currency.quote_mint,
            target.base_mint,
            currency.quote_token_program,
            target.base_token_program,
        )
    };
    let input_ata = ata(&options.user, &input, &input_program);
    let mut instructions = vec![
        create_ata(&options.user, &options.user, &input, &input_program),
        create_ata(&options.user, &options.user, &output, &output_program),
        create_ata(&options.user, &recipient, &currency.quote_mint, &currency.quote_token_program),
    ];
    if !options.sell && input == WSOL && currency.venue == PumpMultiHopVenue::Pool {
        instructions.push(solana_system_interface::instruction::transfer(
            &options.user,
            &input_ata,
            options.amount,
        ));
        instructions
            .push(spl_token_2022_interface::instruction::sync_native(&TOKEN_PROGRAM, &input_ata)?);
    }
    instructions.push(
        select_pump_route(
            sol_trade_sdk::PumpRouteOptions {
                execution: sol_trade_sdk::PumpRouteExecution::Official,
                minimum_output_amount: Some(options.min_out),
                buyback_recipient: Some(recipient),
            },
            options.user,
            input,
            output,
            options.amount,
            Some(&hops),
        )?
        .context("Official execution was required")?,
    );
    Ok(instructions)
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(options) = options()? else { return Ok(()) };
    let rpc = SolanaRpcClient::new_with_timeout_and_commitment(
        std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into()),
        Duration::from_secs(30),
        CommitmentConfig::confirmed(),
    );
    let (slot, hops, recipient) = load_route(&rpc, &options).await?;
    let route: Vec<_> = hops
        .iter()
        .map(|h| {
            serde_json::json!({
                "venue":format!("{:?}", h.venue), "address":h.address.to_string(),
                "base_mint":h.base_mint.to_string(), "quote_mint":h.quote_mint.to_string(),
            })
        })
        .collect();
    let instructions = build_instructions(&options, hops, recipient)?;
    let config = solana_message::v1::TransactionConfig::empty()
        .with_compute_unit_limit(400_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let message =
        solana_message::VersionedMessage::V1(solana_message::v1::Message::try_compile_with_config(
            &options.user,
            &instructions,
            rpc.get_latest_blockhash().await?,
            config,
        )?);
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); message.header().num_required_signatures as usize],
        message,
    };
    let wire = wincode::serialize(&tx)?;
    ensure!(wire.len() <= solana_message::v1::MAX_TRANSACTION_SIZE, "Transaction too large");
    let mut report = serde_json::json!({
        "signed":false, "broadcast":false, "snapshot_slot":slot,
        "mode":if options.simulate {"simulate"} else {"build"}, "message_version":1,
        "side":if options.sell {"sell"} else {"buy"}, "route_in_buy_order":route,
        "user":options.user.to_string(), "amount_in":options.amount, "min_amount_out":options.min_out,
        "wire_bytes":wire.len(), "wire":STANDARD.encode(wire),
        "instructions":instructions.iter().map(|ix| serde_json::json!({
            "program_id":ix.program_id.to_string(), "data":ix.data,
            "accounts":ix.accounts.iter().map(|a| serde_json::json!({
                "pubkey":a.pubkey.to_string(), "signer":a.is_signer, "writable":a.is_writable,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if options.simulate {
        report["response"] = serde_json::to_value(
            rpc.simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    min_context_slot: Some(slot),
                    inner_instructions: true,
                    commitment: Some(CommitmentConfig::confirmed()),
                    encoding: Some(UiTransactionEncoding::Base64),
                    ..Default::default()
                },
            )
            .await?,
        )?;
    }
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = options.report {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(text.as_bytes())?;
    }
    println!("{text}");
    if options.simulate {
        let error = report.pointer("/response/value/err").context("Missing simulation status")?;
        ensure!(error.is_null(), "Simulation failed: {error}; inspect response.value.logs");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_swap_and_sol_funding_follow_each_route_direction() {
        // Construction cases cover all four curve/pool combinations. They do
        // not claim that these generated accounts exist on mainnet.
        let user = Pubkey::new_unique();
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let make_hop = |base, quote, venue| {
            let address = if venue == PumpMultiHopVenue::Curve {
                curve_address(base)
            } else {
                pool_address(base, quote)
            };
            PumpMultiHop {
                venue,
                address,
                base_mint: base,
                quote_mint: quote,
                base_vault: ata(&address, &base, &TOKEN_PROGRAM),
                quote_vault: ata(&address, &quote, &TOKEN_PROGRAM),
                base_token_program: TOKEN_PROGRAM,
                quote_token_program: TOKEN_PROGRAM,
                mayhem: false,
                cashback: false,
                complete: false,
                index: 0,
                creator: Pubkey::find_program_address(&[b"pool-authority", base.as_ref()], &PUMP).0,
            }
        };
        for currency in [PumpMultiHopVenue::Curve, PumpMultiHopVenue::Pool] {
            for venue in [PumpMultiHopVenue::Curve, PumpMultiHopVenue::Pool] {
                for sell in [false, true] {
                    let options = Options {
                        path: vec![WSOL, a, b],
                        venue,
                        user,
                        sell,
                        amount: 12_345,
                        min_out: 456,
                        simulate: false,
                        report: None,
                    };
                    let ixs = build_instructions(
                        &options,
                        vec![make_hop(a, WSOL, currency), make_hop(b, a, venue)],
                        Pubkey::new_unique(),
                    )
                    .unwrap();
                    let swap = ixs.last().unwrap();
                    assert_eq!(ixs.iter().filter(|ix| ix.program_id == AMM).count(), 1);
                    assert_eq!(swap.data[..8], [43, 100, 73, 19, 233, 246, 111, 148]);
                    assert_eq!(u64::from_le_bytes(swap.data[8..16].try_into().unwrap()), 12_345);
                    assert_eq!(u64::from_le_bytes(swap.data[16..24].try_into().unwrap()), 456);
                    assert_eq!(swap.accounts[16].pubkey, if sell { b } else { a });
                    assert_eq!(swap.accounts[21].pubkey, if sell { a } else { b });
                    for (index, mint) in
                        [(1, if sell { b } else { WSOL }), (2, if sell { WSOL } else { b })]
                    {
                        assert_eq!(swap.accounts[index].pubkey, ata(&user, &mint, &TOKEN_PROGRAM));
                    }
                    // Native curve buys/sells must not spend an extra wrapped input.
                    let wraps = !sell && currency == PumpMultiHopVenue::Pool;
                    assert_eq!(ixs.len(), if wraps { 6 } else { 4 });
                    if wraps {
                        assert_eq!(
                            ixs[3],
                            solana_system_interface::instruction::transfer(
                                &user,
                                &swap.accounts[1].pubkey,
                                options.amount
                            )
                        );
                        assert_eq!(ixs[4].data, [17]); // SPL SyncNative
                    }
                    assert!(ixs[..3].iter().all(|ix| ix.data == [1])); // idempotent ATAs
                }
            }
        }
    }
}
