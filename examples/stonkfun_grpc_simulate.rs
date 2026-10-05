//! PublicNode gRPC → subscription cache → local quote/build → RPC simulation.
//! cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --asset SOL
//! --curve selects LaunchLab; --sell is an independent sell; --asset accepts
//! SOL/WSOL/USDC/quote. --direct-usdc selects a direct USDC/stock pool. --venue whirlpool|clmm|dlmm tests conversion alone.
//! Defaults to V1 (4 KiB, no ALTs); --version v0 uses optional cached ALTs.
//! GRPC_URL/GRPC_TOKEN select the provider; legacy aliases remain supported. No private key is needed.
//! Only cold bootstrap / artificial funding / simulation use RPC. Never submits.
use anyhow::{anyhow, ensure, Context, Result};
use base64::Engine;
use sol_parser_sdk::{
    accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    grpc::{AccountFilter, ClientConfig, EventType, EventTypeFilter, YellowstoneGrpc},
    DexEvent,
};
use sol_trade_sdk::{
    common::{address_lookup::decode_address_lookup_table_account, SolanaRpcClient},
    constants::*,
    instruction::{
        stonkfun::StonkFunInstructionBuilder,
        utils::{
            bonk_types, meteora_dlmm as dl, raydium_clmm as cl, raydium_cpmm,
            raydium_cpmm_types as cp, whirlpool as wp,
        },
    },
    swqos::TradeType,
    trading::core::{params::*, traits::InstructionBuilder},
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_client::GetConfirmedSignaturesForAddress2Config,
    rpc_config::{
        RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig, RpcTransactionConfig,
    },
};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::{v0, v1, VersionedMessage};
use solana_rpc_client_api::request::RpcRequest;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

const FUNDER: Pubkey = pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM");
const WP: Pubkey = pubkey!("HJPjoWUrhoZzkNfRpHuieeFk9WcZWjwy6PBjZ81ngndJ");
const CL: Pubkey = pubkey!("3ucNos4NbumPLZNWztqGHNFFgkHeRMBQAVemeeomsUxv");
const DL: Pubkey = pubkey!("BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y");

fn arg(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}
fn positive_argument(name: &str, default: u64) -> Result<u64> {
    positive_value(name, arg(name).as_deref(), default)
}
fn positive_value(name: &str, value: Option<&str>, default: u64) -> Result<u64> {
    let value = value
        .map(|value| value.parse::<u64>())
        .transpose()
        .with_context(|| format!("{name} requires an unsigned integer"))?
        .unwrap_or(default);
    ensure!(value > 0, "{name} must be positive");
    Ok(value)
}
fn pools_ready(required: bool, pools: &[Pubkey], live: &HashSet<Pubkey>) -> bool {
    !required || pools.iter().all(|pool| live.contains(pool))
}
fn parse_simulation_result(
    raw: serde_json::Value,
) -> Result<
    solana_client::rpc_response::Response<
        solana_client::rpc_response::RpcSimulateTransactionResult,
    >,
> {
    ensure!(
        raw.get("value").and_then(|value| value.get("err")).is_some(),
        "Simulation response missing explicit execution status"
    );
    ensure!(
        raw.get("context")
            .and_then(|value| value.get("slot"))
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "Simulation response missing slot"
    );
    Ok(serde_json::from_value(raw)?)
}
fn direct_usdc_steps(
    pool: Pubkey,
    quote: Pubkey,
    asset: Pubkey,
    sell: bool,
) -> Vec<CachedRouteStep> {
    if asset == quote {
        return vec![];
    }
    let mut steps = Vec::new();
    let native = asset == SOL_TOKEN_ACCOUNT || asset == WSOL_TOKEN_ACCOUNT;
    if native && !sell {
        steps.push(CachedRouteStep {
            pool: PoolTradeHint {
                pool: CL,
                input_mint: WSOL_TOKEN_ACCOUNT,
                output_mint: USDC_TOKEN_ACCOUNT,
            },
            input_amount: None,
        });
    }
    steps.push(CachedRouteStep {
        pool: PoolTradeHint {
            pool,
            input_mint: if sell { quote } else { USDC_TOKEN_ACCOUNT },
            output_mint: if sell { USDC_TOKEN_ACCOUNT } else { quote },
        },
        input_amount: None,
    });
    if native && sell {
        steps.push(CachedRouteStep {
            pool: PoolTradeHint {
                pool: CL,
                input_mint: USDC_TOKEN_ACCOUNT,
                output_mint: WSOL_TOKEN_ACCOUNT,
            },
            input_amount: None,
        });
    }
    steps
}
fn validate_arguments(arguments: &[String]) -> Result<()> {
    let values = [
        "--amount",
        "--timeout",
        "--simulation-out",
        "--asset",
        "--venue",
        "--version",
        "--compute-unit-limit",
    ];
    let switches = ["--sell", "--curve", "--direct-usdc", "--require-pool-updates"];
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < arguments.len() {
        let name = arguments[index].as_str();
        ensure!(values.contains(&name) || switches.contains(&name), "Unknown argument: {name}");
        ensure!(seen.insert(name), "Repeated argument: {name}");
        if values.contains(&name) {
            ensure!(
                arguments.get(index + 1).is_some_and(|value| !value.starts_with("--")),
                "{name} requires a value"
            );
            index += 1;
        }
        index += 1;
    }
    Ok(())
}
fn dependencies(pool: Pubkey, owner: Pubkey, d: &[u8]) -> Result<Vec<Pubkey>> {
    let mut keys = vec![pool];
    if owner == raydium_cpmm::accounts::RAYDIUM_CPMM {
        let p = cp::pool_state_decode(d.get(8..).context("Truncated CPMM")?).context("CPMM")?;
        keys.extend([p.amm_config, p.token0_mint, p.token1_mint, p.token0_vault, p.token1_vault]);
    } else if owner == sol_trade_sdk::instruction::utils::bonk::accounts::BONK {
        let p = bonk_types::pool_state_decode(d.get(8..).context("Truncated LaunchLab")?)
            .context("LaunchLab")?;
        keys.extend([
            p.global_config,
            p.platform_config,
            p.base_mint,
            p.quote_mint,
            p.base_vault,
            p.quote_vault,
        ]);
    } else if owner == wp::PROGRAM_ID {
        let p = wp::decode_whirlpool(d)?;
        keys.extend([
            p.token_mint_a,
            p.token_mint_b,
            p.token_vault_a,
            p.token_vault_b,
            wp::oracle(&pool),
        ]);
        for offset in -6..=6 {
            keys.push(wp::tick_array_pda(
                &pool,
                wp::get_start_tick_index(p.tick_current_index, p.tick_spacing, offset),
            ));
        }
    } else if owner == cl::PROGRAM_ID {
        let p = cl::decode_pool_state(d)?;
        keys.extend([
            p.amm_config,
            p.token_mint_0,
            p.token_mint_1,
            p.token_vault_0,
            p.token_vault_1,
            cl::tick_array_bitmap_extension(&pool),
        ]);
        let start = cl::get_array_start_index(p.tick_current, p.tick_spacing);
        for offset in -6..=6 {
            keys.push(cl::tick_array_pda(&pool, start + offset * cl::tick_count(p.tick_spacing)));
        }
    } else if owner == dl::PROGRAM_ID {
        let p = dl::decode_lb_pair(d)?;
        keys.extend([
            p.token_x_mint,
            p.token_y_mint,
            p.reserve_x,
            p.reserve_y,
            p.oracle,
            dl::bitmap_extension_pda(&pool),
        ]);
        let start = dl::bin_id_to_bin_array_index(p.active_id);
        for offset in -6..=6 {
            keys.push(dl::bin_array_pda(&pool, start + offset));
        }
    } else {
        return Err(anyhow!("Unsupported pool owner"));
    }
    Ok(keys)
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "PublicNode gRPC cache → independent StonkFun trade → simulation (never submits)
Usage: cargo run --example stonkfun_grpc_simulate --features parser-adapter -- [options]
  --version v1|v0           Default v1: 4096 bytes, zero ALTs, inline CU config
  --direct-usdc               Stock/USDC fixture; adds WSOL/USDC leg for SOL or WSOL
  --asset SOL|WSOL|USDC|quote   Buy payment or sell receipt asset; default SOL
  --sell                   Independent sell; default is buy
  --curve                  LaunchLab inner pool; default graduated CPMM fixture
  --amount <base units>    Input amount in the selected input mint's base units
  --venue whirlpool|clmm|dlmm   Conversion-only fixture
  --timeout <seconds>      Subscription warmup bound; default 90
  --compute-unit-limit <units>  V1 inline CU limit; default 900000
  --simulation-out <path>  Save original wire and full response, including execution failures
  --require-pool-updates   Require a live gRPC update for every selected pool
Environment: GRPC_URL, GRPC_TOKEN, RPC_URL (aliases: GRPC_ENDPOINT, GRPC_AUTH_TOKEN)
Optional: SNAPSHOT_FILE, SIM_TOKEN_SOURCE; ALTS is only supported with --version v0
RPC is used only for cold bootstrap, simulator funding and simulation.
Example independent USDC buy: --version v1 --asset USDC
Example independent SOL sell: --version v1 --sell --asset SOL --amount 1000000000"
        );
        return Ok(());
    }
    let arguments: Vec<_> = std::env::args().collect();
    validate_arguments(&arguments[1..])?;
    let timeout_seconds = positive_argument("--timeout", 90)?;
    let timeout_duration = Duration::from_secs(timeout_seconds);
    Instant::now().checked_add(timeout_duration).context("--timeout exceeds clock range")?;
    positive_argument("--amount", 1)?; // Reject invalid requested amounts before network I/O.
    u32::try_from(positive_argument("--compute-unit-limit", 900_000)?)
        .context("--compute-unit-limit exceeds u32")?;
    let require_pool_updates = arguments.iter().any(|value| value == "--require-pool-updates");
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Log connection errors without printing endpoint credentials or tokens.
    let _ = tracing_subscriber::fmt()
        .with_env_filter("sol_parser_sdk=warn")
        .with_target(false)
        .try_init();
    let rpc = SolanaRpcClient::new_with_commitment(
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into()),
        CommitmentConfig::processed(),
    );
    let version = match arg("--version").as_deref().unwrap_or("v1") {
        "v1" => sol_trade_sdk::common::TradeTransactionVersion::V1,
        "v0" => sol_trade_sdk::common::TradeTransactionVersion::V0,
        _ => return Err(anyhow!("--version must be v1 or v0")),
    };
    let use_v1 = version == sol_trade_sdk::common::TradeTransactionVersion::V1;
    ensure!(
        !use_v1 || std::env::var("ALTS").is_err(),
        "V1 does not support ALTs; use --version v0 with ALTS"
    );
    let direct_usdc = std::env::args().any(|a| a == "--direct-usdc");
    let curve = direct_usdc || std::env::args().any(|a| a == "--curve");
    let sell = std::env::args().any(|a| a == "--sell");
    let venue = arg("--venue");
    let standalone = venue.is_some();
    let asset_name = arg("--asset").unwrap_or_else(|| "SOL".into());
    ensure!(
        !standalone || (!sell && !curve && asset_name != "quote"),
        "--venue is conversion-only: use SOL, WSOL or USDC; omit --sell and --curve"
    );
    ensure!(!direct_usdc || !standalone, "--direct-usdc cannot use --venue");
    let (meme_pool, meme, quote, funding_pool, alt) = if direct_usdc {
        (
            pubkey!("BmQj9pBopxouHecN5rYvVLqN7a48CVndfTkhESEZzWgN"),
            pubkey!("DQsYFPcRKaKZWTjY4TiqvmxjN87vjumWMECJ7s4U1HbN"),
            pubkey!("Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh"),
            pubkey!("49iMatQtoyabsYAQc8GafVq6aeBFVDxSRH44oiatyyw6"),
            pubkey!("58tUNfyBtwkMeU8gpyZYKEo4Hyb4E25ui12w5vKYQx3P"),
        )
    } else if curve {
        (
            pubkey!("84XZdJNyBVVBqGe3BHY8n6x1jbcnxNWA5x4GetwQsjgp"),
            pubkey!("BJ56gcrMNKDzVwjQXKToya9cAcMZvN9pz6ZzUejxQary"),
            pubkey!("CARDSccUMFKoPRZxt5vt3ksUbxEFEcnZ3H2pd3dKxYjp"),
            pubkey!("3kMBV4dFBLoAaNFBcXcnaY2sY6k4k45Pcuo2zXSJHXQx"),
            pubkey!("2gyDuEaj39reuVjrQxGtFsK7625MbA9qDzNmHZUibrN2"),
        )
    } else {
        (
            pubkey!("BUVzsLLLG7GWoyJVoU31pXiBveazA6GXTavZ9VD3CwS9"),
            pubkey!("8RVBk8vxLiUHueLUW1f4izFVqN3nWippLhkohKg6EGkS"),
            pubkey!("6GmAFSYs4gk3FDao5FzzySQpPZaWsa4rUJHacpMpUNgx"),
            pubkey!("EKPjNvowpSFPaZcroeUcgAPtdUTZZrP3v8sCKKyfpe5x"),
            pubkey!("8vp6JD2W19rM6Vbs3cBRCuQ3nXc8aykRMRrMEA6zoigR"),
        )
    };
    let asset = match asset_name.as_str() {
        "SOL" => SOL_TOKEN_ACCOUNT,
        "WSOL" => WSOL_TOKEN_ACCOUNT,
        "USDC" => USDC_TOKEN_ACCOUNT,
        "quote" => quote,
        _ => return Err(anyhow!("Unknown --asset")),
    };
    let standalone_pool = match venue.as_deref() {
        Some("whirlpool") => WP,
        Some("clmm") => CL,
        Some("dlmm") => DL,
        None => WP,
        _ => return Err(anyhow!("Unknown --venue")),
    };
    let pools = if standalone {
        vec![standalone_pool]
    } else {
        let mut p = vec![meme_pool];
        if asset != quote {
            p.push(funding_pool);
        }
        if asset == USDC_TOKEN_ACCOUNT && !direct_usdc {
            p.push(WP);
        }
        if direct_usdc && (asset == SOL_TOKEN_ACCOUNT || asset == WSOL_TOKEN_ACCOUNT) {
            p.push(CL);
        }
        p
    };
    // Cold dependency discovery only; production obtains pool identity from parser routes.
    let mut keys = vec![solana_sdk::sysvar::clock::ID];
    for pool in &pools {
        let a = rpc.get_account(pool).await?;
        keys.extend(dependencies(*pool, a.owner, &a.data)?);
    }
    let mut alt_keys = Vec::new();
    if !standalone && !use_v1 {
        alt_keys.push(alt);
    }
    if let Ok(value) = std::env::var("ALTS") {
        for key in value.split(',').filter(|s| !s.is_empty()) {
            alt_keys.push(key.parse::<Pubkey>()?);
        }
    } else if !standalone && !use_v1 && asset == USDC_TOKEN_ACCOUNT && !direct_usdc {
        // Cold-start ALT discovery from real recent conversion transactions.
        // Production can instead pass trusted ALT identities from parser/cache.
        let signatures = rpc
            .get_signatures_for_address_with_config(
                &WP,
                GetConfirmedSignaturesForAddress2Config {
                    limit: Some(3),
                    commitment: Some(CommitmentConfig::confirmed()),
                    ..Default::default()
                },
            )
            .await?;
        for signature in signatures {
            let tx = rpc
                .get_transaction_with_config(
                    &signature.signature.parse()?,
                    RpcTransactionConfig {
                        encoding: Some(UiTransactionEncoding::Base64),
                        commitment: Some(CommitmentConfig::confirmed()),
                        max_supported_transaction_version: Some(0),
                    },
                )
                .await?;
            if let Some(tx) = tx.transaction.transaction.decode() {
                if let VersionedMessage::V0(message) = tx.message {
                    alt_keys
                        .extend(message.address_table_lookups.into_iter().map(|a| a.account_key));
                }
            }
        }
    }
    alt_keys.sort();
    alt_keys.dedup();
    keys.extend(&alt_keys);
    keys.sort();
    keys.dedup();
    let grpc = YellowstoneGrpc::new_with_config(
        std::env::var("GRPC_URL")
            .or_else(|_| std::env::var("GRPC_ENDPOINT"))
            .unwrap_or_else(|_| "https://solana-yellowstone-grpc.publicnode.com:443".into()),
        std::env::var("GRPC_TOKEN").or_else(|_| std::env::var("GRPC_AUTH_TOKEN")).ok(),
        ClientConfig::default(),
    )
    .map_err(|e| anyhow!("gRPC setup: {e}"))?;
    let queue = grpc
        .subscribe_dex_events(
            vec![],
            vec![AccountFilter {
                account: keys.iter().map(ToString::to_string).collect(),
                owner: vec![],
                filters: vec![],
            }],
            Some(EventTypeFilter::include_only(vec![
                EventType::AccountRawSnapshot,
                EventType::BlockMeta,
            ])),
        )
        .await
        .map_err(|e| anyhow!("gRPC subscribe: {e}"))?;
    // Queue creation is asynchronous: wait for proof of a connected stream.
    // Bootstrap a completed confirmed bank while later updates buffer in queue.
    let ready_deadline =
        Instant::now().checked_add(timeout_duration).context("--timeout exceeds clock range")?;
    loop {
        if let Some(DexEvent::BlockMeta(_)) = queue.pop() {
            break;
        }
        if Instant::now() >= ready_deadline {
            grpc.stop().await;
            return Err(anyhow!(
                "No gRPC block metadata before deadline: check token and endpoint"
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let initial =
        rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
    let mut snapshots = HashMap::new();
    let mut cache = SubscriptionAccountCache::default();
    for (key, a) in keys.iter().zip(initial.value) {
        if let Some(a) = a {
            let event = RawAccountSnapshotEvent {
                metadata: sol_parser_sdk::core::events::EventMetadata {
                    slot: initial.context.slot,
                    ..Default::default()
                },
                account: sol_parser_sdk::accounts::AccountData {
                    pubkey: *key,
                    owner: a.owner,
                    data: a.data,
                    lamports: a.lamports,
                    executable: a.executable,
                    rent_epoch: a.rent_epoch,
                },
                write_version: 0,
                is_startup: true,
            };
            cache.update_from_parser_snapshot(&event)?;
            snapshots.insert(*key, event);
        }
    }
    println!(
        "cold bootstrap={} accounts; waiting for gRPC Clock + blockhash (unchanged pool/config accounts retain bootstrap state)",
        snapshots.len()
    );
    let deadline =
        Instant::now().checked_add(timeout_duration).context("--timeout exceeds clock range")?;
    let mut live_pools = HashSet::new();
    let mut live_clock = false;
    let mut block = None;
    let mut raw_updates = 0;
    loop {
        while let Some(event) = queue.pop() {
            match event {
                DexEvent::RawAccountSnapshot(e) => {
                    // Bootstrap write_version is unknown; ignore its completed
                    // slot and older writes instead of reverting current bytes.
                    if e.metadata.slot <= initial.context.slot {
                        continue;
                    }
                    if cache.update_from_parser_snapshot(&e)? {
                        raw_updates += 1;
                        if pools.contains(&e.account.pubkey) {
                            live_pools.insert(e.account.pubkey);
                        }
                        if e.account.pubkey == solana_sdk::sysvar::clock::ID {
                            live_clock = true;
                        }
                        snapshots.insert(e.account.pubkey, (*e).clone());
                    }
                }
                DexEvent::BlockMeta(e) => {
                    if let Some(hash) = e.metadata.recent_blockhash {
                        block = Some((e.metadata.slot, hash.parse::<Hash>()?));
                    }
                }
                _ => {}
            }
        }
        let latest = snapshots.values().map(|e| e.metadata.slot).max().unwrap_or(0);
        if live_clock
            && pools_ready(require_pool_updates, &pools, &live_pools)
            && block.as_ref().is_some_and(|b| b.0 >= latest)
        {
            break;
        }
        if Instant::now() >= deadline {
            grpc.stop().await;
            return Err(anyhow!("Timed out: raw_updates={raw_updates}, live_pools={}, live_clock={live_clock}, blockhash={}. Check gRPC token/availability; inactive pools may not emit updates",live_pools.len(),block.is_some()));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    grpc.stop().await;
    let (slot, hash) = block.context("Missing gRPC blockhash")?;
    let clock = &snapshots[&solana_sdk::sysvar::clock::ID].account.data;
    ensure!(clock.len() >= 40, "Truncated Clock");
    let epoch = u64::from_le_bytes(clock[16..24].try_into()?);
    let timestamp = u64::try_from(i64::from_le_bytes(clock[32..40].try_into()?))?;
    let ctx = CacheReadContext { slot, epoch, maximum_slot_age: 512 };
    let amount = positive_argument(
        "--amount",
        if sell {
            1_000
        } else if standalone {
            if asset == USDC_TOKEN_ACCOUNT {
                100_000
            } else {
                1_000_000
            }
        } else {
            50_000
        },
    )?;
    let request = CachedQuoteRequest {
        amount_in: amount,
        slippage_basis_points: 300,
        unix_timestamp: timestamp,
        maximum_arrays: 6,
    };
    let wallet = Arc::new(Keypair::new());
    let mut params = caller_params(wallet.clone());
    params.transaction_version = version;
    params.recent_blockhash = Some(hash);
    let minimum;
    if standalone {
        let (input, output) = if asset == USDC_TOKEN_ACCOUNT {
            (USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT)
        } else {
            (WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT)
        };
        let q = cache.quote_exact_in(
            PoolTradeHint { pool: standalone_pool, input_mint: input, output_mint: output },
            request,
            ctx,
        )?;
        minimum = q.minimum_net_amount_out;
        params.input_mint = if asset == SOL_TOKEN_ACCOUNT { SOL_TOKEN_ACCOUNT } else { input };
        params.output_mint = output;
        params.input_amount = Some(amount);
        params.protocol_params =
            DexParamEnum::StonkFunQuoteRoute(StonkFunQuoteRoute::buy(vec![q.hop]));
    } else {
        let mut steps = vec![];
        if direct_usdc {
            steps = direct_usdc_steps(funding_pool, quote, asset, sell);
        } else if asset != quote {
            if sell {
                steps.push(CachedRouteStep {
                    pool: PoolTradeHint {
                        pool: funding_pool,
                        input_mint: quote,
                        output_mint: WSOL_TOKEN_ACCOUNT,
                    },
                    input_amount: None,
                });
            } else if asset == USDC_TOKEN_ACCOUNT {
                steps.push(CachedRouteStep {
                    pool: PoolTradeHint {
                        pool: WP,
                        input_mint: USDC_TOKEN_ACCOUNT,
                        output_mint: WSOL_TOKEN_ACCOUNT,
                    },
                    input_amount: None,
                });
            }
            if !sell {
                steps.push(CachedRouteStep {
                    pool: PoolTradeHint {
                        pool: funding_pool,
                        input_mint: WSOL_TOKEN_ACCOUNT,
                        output_mint: quote,
                    },
                    input_amount: None,
                });
            } else if asset == USDC_TOKEN_ACCOUNT {
                steps.push(CachedRouteStep {
                    pool: PoolTradeHint {
                        pool: WP,
                        input_mint: WSOL_TOKEN_ACCOUNT,
                        output_mint: USDC_TOKEN_ACCOUNT,
                    },
                    input_amount: None,
                });
            }
        }
        let prepared = cache.prepare_stonkfun_trade(
            PoolTradeHint { pool: meme_pool, input_mint: quote, output_mint: meme },
            meme,
            asset,
            &steps,
            request,
            ctx,
            !sell,
        )?;
        minimum = prepared.minimum_amount_out;
        params = prepared.apply_to(params)?;
    }
    for key in alt_keys {
        let a = &snapshots.get(&key).context("Missing subscribed ALT snapshot")?.account;
        params
            .address_lookup_table_accounts
            .push(decode_address_lookup_table_account(&key, &a.owner, &a.data)?);
    }
    params.close_output_mint_ata = params.output_mint == SOL_TOKEN_ACCOUNT;
    params = params.without_rpc()?;
    println!("gRPC updates={raw_updates} pool_updates={} slot={slot} epoch={epoch}; hot path RPC=None input={} output={} amount={amount} net_min={minimum}",live_pools.len(),params.input_mint,params.output_mint);
    // Business instructions are constructed before any simulator funding RPC.
    let business = if sell && !standalone {
        StonkFunInstructionBuilder.build_sell_instructions(&params).await?
    } else {
        StonkFunInstructionBuilder.build_buy_instructions(&params).await?
    };
    simulate(&rpc, &params, business, &snapshots, minimum).await
}

fn caller_params(wallet: Arc<Keypair>) -> SwapParams {
    SwapParams {
        rpc: None,
        payer: wallet,
        trade_type: TradeType::Buy,
        input_mint: sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
        input_token_program: None,
        output_mint: Pubkey::default(),
        output_token_program: None,
        input_amount: None,
        slippage_basis_points: Some(300),
        address_lookup_table_accounts: Vec::new(),
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params: DexParamEnum::StonkFunQuoteRoute(StonkFunQuoteRoute::default()),
        open_seed_optimize: false,
        swqos_clients: Arc::new(Vec::new()),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: false,
        fixed_output_amount: None,
        gas_fee_strategy: sol_trade_sdk::common::GasFeeStrategy::new(),
        simulate: false,
        log_enabled: true,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 0,
        effective_core_ids: Arc::new(Vec::new()),
        check_min_tip: false,
        transaction_version: sol_trade_sdk::common::TradeTransactionVersion::V0,
        grpc_recv_us: None,
        use_exact_sol_amount: None,
    }
}

async fn simulate(
    rpc: &SolanaRpcClient,
    params: &SwapParams,
    business: Vec<Instruction>,
    snapshots: &HashMap<Pubkey, RawAccountSnapshotEvent>,
    minimum: u64,
) -> Result<()> {
    // Artificial funding exists only in sigVerify=false simulation. Never send
    // this transaction: source wallet/vault signatures are intentionally absent.
    let wallet = params.payer.pubkey();
    let input =
        if params.input_mint == SOL_TOKEN_ACCOUNT { WSOL_TOKEN_ACCOUNT } else { params.input_mint };
    let output = if params.output_mint == SOL_TOKEN_ACCOUNT {
        WSOL_TOKEN_ACCOUNT
    } else {
        params.output_mint
    };
    let input_program = snapshots.get(&input).context("Input mint snapshot missing")?.account.owner;
    let output_program =
        snapshots.get(&output).context("Output mint snapshot missing")?.account.owner;
    // V0 uses Solana's per-instruction default compute budget in this fixture.
    // An explicit override is optional; its extra program key can make a
    // virtual-funded multi-hop transaction exceed the packet limit.
    let mut ix = vec![system::transfer(&FUNDER, &wallet, 100_000_000)];
    let v1_mode = params.transaction_version == sol_trade_sdk::common::TradeTransactionVersion::V1;
    if !v1_mode {
        if let Some(limit) = arg("--compute-unit-limit") {
            ix.insert(0, ComputeBudgetInstruction::set_compute_unit_limit(limit.parse()?));
        }
    }
    if params.input_mint == WSOL_TOKEN_ACCOUNT {
        push_create_or_wrap_user_token_account(
            &mut ix,
            &wallet,
            &input,
            &input_program,
            params.input_amount.context("amount")?,
            false,
        );
    } else if params.input_mint != SOL_TOKEN_ACCOUNT {
        let amount =
            params.input_amount.context("amount")?.checked_mul(2).context("funding overflow")?;
        let source = if let Ok(key) = std::env::var("SIM_TOKEN_SOURCE") {
            key.parse::<Pubkey>()?
        } else if input == USDC_TOKEN_ACCOUNT {
            // Unrelated venue: artificial funding cannot alter quoted reserves.
            if !snapshots.contains_key(&CL) {
                let a = rpc.get_account(&CL).await?;
                cl::decode_pool_state(&a.data)?.token_vault_1
            } else {
                let a = rpc.get_account(&WP).await?;
                wp::decode_whirlpool(&a.data)?.token_vault_b
            }
        } else if input == pubkey!("6GmAFSYs4gk3FDao5FzzySQpPZaWsa4rUJHacpMpUNgx") {
            // Stock quote held in a separate conversion pool. Direct quote
            // trades do not execute that pool; dependency exclusion below
            // rejects it if a future fixture tries to quote the same vault.
            pubkey!("3jrKr4TLh5iFadGHS45KagGQapneAugZWEuKKFHGtBMR")
        } else if input == pubkey!("8RVBk8vxLiUHueLUW1f4izFVqN3nWippLhkohKg6EGkS") {
            // Unrelated holder observed in the checked-in mainnet fixture.
            pubkey!("vWPUUNVqjnQDgorpXnkCc24EpYUzpH51op6DseWbL7L")
        } else if input == pubkey!("DQsYFPcRKaKZWTjY4TiqvmxjN87vjumWMECJ7s4U1HbN") {
            // Unrelated holder for the direct USDC/stock LaunchLab fixture.
            // Its mint, current balance and dependency exclusion are checked below.
            pubkey!("FKzYwoYgJi9KXkn8s37QzqAtbqfDD5vQArge9SvgMVcc")
        } else {
            let candidates = rpc.get_token_largest_accounts(&input).await?;
            candidates
                .iter()
                .filter_map(|a| a.address.parse::<Pubkey>().ok())
                .find(|a| !snapshots.contains_key(a))
                .context("No unrelated funding holder; set SIM_TOKEN_SOURCE")?
        };
        ensure!(
            !snapshots.contains_key(&source),
            "Funding source must not mutate quoted pool dependencies"
        );
        let a = rpc.get_account(&source).await?;
        ensure!(
            a.owner == input_program && a.data.len() >= 165 && a.data[..32] == input.to_bytes(),
            "Invalid simulator token source"
        );
        ensure!(
            u64::from_le_bytes(a.data[64..72].try_into()?) >= amount,
            "Simulator source balance too small"
        );
        let authority = Pubkey::new_from_array(a.data[32..64].try_into()?);
        push_create_user_token_account(&mut ix, &wallet, &input, &input_program, false);
        let dest=sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(&wallet,&input,&input_program,false);
        let mut data = vec![12];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(snapshots[&input].account.data[44]);
        ix.push(Instruction::new_with_bytes(
            input_program,
            &data,
            vec![
                AccountMeta::new(source, false),
                AccountMeta::new_readonly(input, false),
                AccountMeta::new(dest, false),
                AccountMeta::new_readonly(authority, true),
            ],
        ));
        println!("simulation-only token funding source={source}; business pool reserves unchanged by funding");
    }
    ix.extend(business);
    let hash = params.recent_blockhash.context("gRPC blockhash")?;
    let message = if v1_mode {
        ensure!(params.address_lookup_table_accounts.is_empty(), "V1 cannot use ALTs");
        let limit =
            arg("--compute-unit-limit").map(|s| s.parse::<u32>()).transpose()?.unwrap_or(900_000);
        let config = v1::TransactionConfig::empty()
            .with_compute_unit_limit(limit)
            .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
        VersionedMessage::V1(v1::Message::try_compile_with_config(&FUNDER, &ix, hash, config)?)
    } else {
        VersionedMessage::V0(v0::Message::try_compile(
            &FUNDER,
            &ix,
            &params.address_lookup_table_accounts,
            hash,
        )?)
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); message.header().num_required_signatures as usize],
        message,
    };
    // V1 uses a different wire layout; serde/bincode is not its wire serializer.
    let size = wincode::serialized_size(&tx)? as usize;
    let maximum = if v1_mode { v1::MAX_TRANSACTION_SIZE } else { 1232 };
    ensure!(
        size <= maximum,
        "Transaction too large ({size}>{maximum}); select V1 or supply ALT coverage for V0"
    );
    println!(
        "transaction_version={} ALTs={} max_bytes={maximum}",
        if v1_mode { "v1" } else { "v0" },
        params.address_lookup_table_accounts.len()
    );
    let output_account =
        sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &wallet,
            &output,
            &output_program,
            false,
        );
    // For native SOL receipt, recover exact wallet credit by adding back rent
    // retained in all newly created user ATAs. The ephemeral wallet starts empty
    // and FUNDER pays transaction fees, so the simulation prefix is 100M lamports.
    let native_output = params.output_mint == SOL_TOKEN_ACCOUNT;
    let mut addresses = vec![output_account.to_string()];
    if native_output {
        addresses = vec![wallet.to_string()];
        for event in snapshots.values() {
            let a = &event.account;
            let is_mint = a.data.len() == 82 || (a.data.len() > 165 && a.data[165] == 1);
            if (a.owner == TOKEN_PROGRAM
                || a.owner == solana_sdk::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"))
                && is_mint
            {
                addresses.push(sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                    &wallet,&a.pubkey,&a.owner,false).to_string());
            }
        }
        addresses.sort();
        addresses.dedup();
    }
    let native_wsol_index = addresses.iter().position(|a| a == &output_account.to_string());
    let expected_accounts = addresses.len();
    let simulation_config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::processed()),
        encoding: Some(UiTransactionEncoding::Base64),
        inner_instructions: true,
        min_context_slot: Some(snapshots.values().map(|e| e.metadata.slot).max().unwrap_or(0)),
        accounts: Some(RpcSimulateTransactionAccountsConfig {
            encoding: Some(UiAccountEncoding::Base64),
            addresses,
        }),
        ..Default::default()
    };
    let mut result = None;
    let wire = base64::engine::general_purpose::STANDARD.encode(wincode::serialize(&tx)?);
    for attempt in 0..12 {
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            rpc.send::<serde_json::Value>(
                RpcRequest::SimulateTransaction,
                serde_json::json!([&wire, &simulation_config]),
            ),
        )
        .await
        .context("Simulation RPC exceeded 30 second deadline")?;
        let response = match response {
            Ok(raw) => Ok(parse_simulation_result(raw)?),
            Err(error) => Err(error),
        };
        match response {
            Ok(response)
                if response
                    .value
                    .err
                    .as_ref()
                    .is_some_and(|e| e.to_string().contains("Blockhash not found"))
                    && attempt < 11 =>
            {
                println!("simulator behind gRPC blockhash; retry {}", attempt + 1);
            }
            Ok(response) => {
                result = Some(response);
                break;
            }
            Err(error)
                if error.to_string().contains("Minimum context slot has not been reached")
                    && attempt < 11 =>
            {
                println!("simulator behind gRPC context; retry {}", attempt + 1);
            }
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
    let result = result.context("Simulator did not catch up to gRPC state")?;
    if let Some(path) = arg("--simulation-out") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "wire": wire,
                "response": {"jsonrpc": "2.0", "result": &result},
                "simulation_only_funding": true,
                "broadcasts": 0
            }))?,
        )?;
    }
    if let Some(err) = result.value.err {
        for log in result.value.logs.unwrap_or_default() {
            eprintln!("{log}");
        }
        return Err(anyhow!("Simulation failed: {err:?}"));
    }
    let accounts = result.value.accounts.context("Missing simulated accounts")?;
    ensure!(accounts.len() == expected_accounts, "Incomplete simulated account response");
    let actual = if native_output {
        let wsol_index = native_wsol_index.context("Native WSOL return-account index")?;
        ensure!(
            accounts.get(wsol_index).and_then(Option::as_ref).is_none_or(|a| a.lamports == 0),
            "SOL output left a funded WSOL account open"
        );
        let total = accounts.into_iter().flatten().try_fold(0u64, |sum, a| {
            sum.checked_add(a.lamports).context("Native balance overflow")
        })?;
        total.checked_sub(100_000_000).context("Native output/rent accounting underflow")?
    } else {
        let account =
            accounts.into_iter().next().flatten().context("Missing output token account")?;
        ensure!(account.owner == output_program.to_string(), "Unexpected output account owner");
        let data = account.data.decode().context("Output account encoding")?;
        ensure!(data.get(..32) == Some(output.as_ref()), "Unexpected output account mint");
        u64::from_le_bytes(data.get(64..72).context("Output token balance")?.try_into()?)
    };
    ensure!(
        actual >= minimum,
        "Simulation net output below minimum: actual={actual}, min={minimum}"
    );
    println!("SIMULATION PASSED slot={} CU={:?} transaction_bytes={size} net_output={actual} min={minimum}; never submitted",result.context.slot,result.value.units_consumed);
    if let Ok(path) = std::env::var("SNAPSHOT_FILE") {
        let mut events: Vec<_> = snapshots.values().collect();
        events.sort_by_key(|e| e.account.pubkey);
        std::fs::write(path, serde_json::to_vec_pretty(&events)?)?;
    }
    Ok(())
}

fn push_create_user_token_account(
    ix: &mut Vec<Instruction>,
    payer: &Pubkey,
    mint: &Pubkey,
    program: &Pubkey,
    use_seed: bool,
) {
    ix.extend(
        sol_trade_sdk::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
            payer, payer, mint, program, use_seed,
        ),
    );
}
fn push_create_or_wrap_user_token_account(
    ix: &mut Vec<Instruction>,
    payer: &Pubkey,
    mint: &Pubkey,
    program: &Pubkey,
    amount: u64,
    use_seed: bool,
) {
    if *mint == WSOL_TOKEN_ACCOUNT {
        ix.extend(sol_trade_sdk::trading::common::handle_wsol(payer, amount));
    } else {
        push_create_user_token_account(ix, payer, mint, program, use_seed);
    }
}

#[cfg(test)]
mod argument_tests {
    use super::positive_value;

    #[test]
    fn explicit_amount_cannot_silently_fall_back_to_default() {
        for value in ["abc", "-1", "0", "1.5", "18446744073709551616"] {
            assert!(positive_value("--amount", Some(value), 50_000).is_err());
        }
    }

    #[test]
    fn exact_u64_and_omitted_default_are_preserved() {
        assert_eq!(positive_value("--amount", Some("18446744073709551615"), 1).unwrap(), u64::MAX);
        assert_eq!(positive_value("--amount", None, 50_000).unwrap(), 50_000);
    }
    #[test]
    fn optional_cold_pool_policy_applies_to_conversion_only_routes() {
        let pool = solana_sdk::pubkey::Pubkey::new_unique();
        let live = std::collections::HashSet::new();
        assert!(super::pools_ready(false, &[pool], &live));
        assert!(!super::pools_ready(true, &[pool], &live));
        assert!(super::pools_ready(true, &[pool], &std::collections::HashSet::from([pool])));
    }
    #[test]
    fn typos_and_repeated_options_cannot_change_requested_trade_silently() {
        for args in [
            vec!["--ammount", "1000"],
            vec!["--asset", "SOL", "--asset", "WSOL"],
            vec!["--amount"],
            vec!["extra.json"],
        ] {
            assert!(super::validate_arguments(
                &args.into_iter().map(String::from).collect::<Vec<_>>()
            )
            .is_err());
        }
        assert!(super::validate_arguments(
            &["--asset", "WSOL", "--sell", "--amount", "1000"].map(String::from)
        )
        .is_ok());
    }
    #[test]
    fn stock_usdc_conversion_paths_connect_for_each_asset_and_direction() {
        use super::*;
        let pool = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        for asset in [SOL_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, quote] {
            for sell in [false, true] {
                let steps = direct_usdc_steps(pool, quote, asset, sell);
                if asset == quote {
                    assert!(steps.is_empty());
                    continue;
                }
                let endpoint = if asset == SOL_TOKEN_ACCOUNT { WSOL_TOKEN_ACCOUNT } else { asset };
                assert_eq!(
                    steps.first().unwrap().pool.input_mint,
                    if sell { quote } else { endpoint }
                );
                assert_eq!(
                    steps.last().unwrap().pool.output_mint,
                    if sell { endpoint } else { quote }
                );
                for pair in steps.windows(2) {
                    assert_eq!(pair[0].pool.output_mint, pair[1].pool.input_mint);
                    assert_ne!(pair[0].pool.pool, pair[1].pool.pool);
                }
                assert!(steps.iter().all(|step| step.input_amount.is_none()));
            }
        }
    }
    #[test]
    fn missing_rpc_error_field_cannot_be_defaulted_to_success() {
        let raw = serde_json::json!({"context":{"slot":1},"value":{"logs":null}});
        assert!(super::parse_simulation_result(raw).is_err());
    }
}
