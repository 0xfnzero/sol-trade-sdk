//! Compare real swaps with and without an immediately preceding fee collection.
#[allow(dead_code)]
#[path = "cpmm_creator_fee_simulation.rs"]
pub(crate) mod collection;

use anyhow::{bail, ensure, Context, Result};
use collection::{
    decode_config, decode_pool, required, snapshot, token_amount, transfer_fee, unchanged,
    unsigned_instructions,
};
use serde_json::{json, Value};
use sol_trade_sdk::{
    common::{
        fast_fn::{
            create_associated_token_account_idempotent_fast_use_seed,
            get_associated_token_address_with_program_id_fast_use_seed,
        },
        GasFeeStrategy, SolanaRpcClient,
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
            raydium_cpmm_types::{AmmConfig, PoolState},
        },
    },
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, RaydiumCpmmParams, SwapParams, TokenTransferFee},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_cpmm::compute_swap_amount_for_pool,
};
use solana_client::rpc_config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
        StateWithExtensions,
    },
    state::Mint,
};
use std::sync::Arc;

const DEFAULT_SWAP_POOL: &str = "EjqGHevzN2pgaTf5CZnbMBuiYdPpWYx4pmeY7abwqyrM";

pub(crate) fn fee_schedule(account: &Account, epoch: u64) -> Result<TokenTransferFee> {
    if account.owner == sol_trade_sdk::constants::TOKEN_PROGRAM {
        return Ok(TokenTransferFee::default());
    }
    ensure!(
        account.owner.to_bytes() == spl_token_2022_interface::ID.to_bytes(),
        "Unknown mint program"
    );
    let mint = StateWithExtensions::<Mint>::unpack(&account.data)?;
    if !mint.get_extension_types()?.contains(&ExtensionType::TransferFeeConfig) {
        return Ok(TokenTransferFee::default());
    }
    let fee = mint.get_extension::<TransferFeeConfig>()?.get_epoch_fee(epoch);
    Ok(TokenTransferFee {
        basis_points: fee.transfer_fee_basis_points.into(),
        maximum_fee: fee.maximum_fee.into(),
    })
}
pub(crate) fn protocol_params(
    pool_address: Pubkey,
    pool: &PoolState,
    config: &AmmConfig,
    accounts: &[Option<Account>],
    epoch: u64,
) -> Result<RaydiumCpmmParams> {
    let balance0 = token_amount(
        required(accounts, 3)?,
        &pool.token0_mint,
        &accounts::AUTHORITY,
        &pool.token0_program,
    )?;
    let balance1 = token_amount(
        required(accounts, 4)?,
        &pool.token1_mint,
        &accounts::AUTHORITY,
        &pool.token1_program,
    )?;
    Ok(RaydiumCpmmParams {
        pool_state: pool_address,
        amm_config: pool.amm_config,
        base_mint: pool.token0_mint,
        quote_mint: pool.token1_mint,
        base_reserve: balance0
            .checked_sub(pool.protocol_fees_token0)
            .and_then(|a| a.checked_sub(pool.fund_fees_token0))
            .and_then(|a| a.checked_sub(pool.creator_fees_token0))
            .context("Invalid token0 reserves")?,
        quote_reserve: balance1
            .checked_sub(pool.protocol_fees_token1)
            .and_then(|a| a.checked_sub(pool.fund_fees_token1))
            .and_then(|a| a.checked_sub(pool.creator_fees_token1))
            .context("Invalid token1 reserves")?,
        base_vault: pool.token0_vault,
        quote_vault: pool.token1_vault,
        base_token_program: pool.token0_program,
        quote_token_program: pool.token1_program,
        observation_state: pool.observation_key,
        trade_fee_rate: config.trade_fee_rate,
        protocol_fee_rate: config.protocol_fee_rate,
        fund_fee_rate: config.fund_fee_rate,
        creator_fee_rate: config.creator_fee_rate,
        creator_fee_on: pool.creator_fee_on,
        enable_creator_fee: pool.enable_creator_fee,
        base_transfer_fee: fee_schedule(required(accounts, 7)?, epoch)?,
        quote_transfer_fee: fee_schedule(required(accounts, 8)?, epoch)?,
    })
}
pub(crate) struct Simulation {
    pub(crate) slot: u64,
    pub(crate) accounts: Vec<Option<Account>>,
    pub(crate) logs: Vec<String>,
    pub(crate) units: Option<u64>,
}
pub(crate) async fn simulate(
    rpc: &SolanaRpcClient,
    instructions: &[Instruction],
    payer: Pubkey,
    keys: &[Pubkey],
    min_slot: u64,
) -> Result<Simulation> {
    let response = collection::simulate_strict(
        rpc,
        &unsigned_instructions(instructions, payer),
        RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::confirmed()),
            min_context_slot: Some(min_slot),
            accounts: Some(RpcSimulateTransactionAccountsConfig {
                encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                addresses: keys.iter().map(ToString::to_string).collect(),
            }),
            ..Default::default()
        },
    )
    .await?;
    if let Some(error) = response.value.err {
        bail!("Swap simulation failed: {error:?}; {:?}", response.value.logs);
    }
    let ui = response.value.accounts.context("Missing simulated accounts")?;
    let accounts = collection::decode_simulated_accounts(&ui, keys.len())?;
    Ok(Simulation {
        slot: response.context.slot,
        accounts,
        logs: response.value.logs.unwrap_or_default(),
        units: response.value.units_consumed,
    })
}
fn swap_event(
    simulation: &Simulation,
) -> Result<sol_parser_sdk::core::events::RaydiumCpmmSwapEvent> {
    let mut events = simulation.logs.iter().filter_map(|log| {
        sol_parser_sdk::logs::raydium_cpmm::parse_log(
            log,
            Signature::default(),
            simulation.slot,
            0,
            None,
            0,
        )
    });
    let Some(sol_parser_sdk::core::events::DexEvent::RaydiumCpmmSwap(event)) = events.next() else {
        bail!("Missing CPMM swap log");
    };
    ensure!(events.next().is_none(), "Unexpected extra CPMM swap event");
    Ok(event)
}

pub async fn verify_swap_pair(
    rpc: &SolanaRpcClient,
    pool_address: Pubkey,
    payer: Pubkey,
    permissionless: bool,
    exact_output: bool,
) -> Result<Value> {
    const AMOUNT_IN: u64 = 1_000_000;
    const FUNDING_AMOUNT: u64 = 2_000_000;
    for attempt in 0..4 {
        let bootstrap = fetch_pool_state(rpc, &pool_address).await?;
        let wallet = Arc::new(Keypair::new());
        let trader = wallet.pubkey();
        let creator = bootstrap.pool_creator;
        let collect = if permissionless {
            collect_creator_fee_permissionless(&payer, &pool_address, &bootstrap)
        } else {
            collect_creator_fee(&pool_address, &bootstrap)
        };
        let user0 = get_associated_token_address_with_program_id_fast_use_seed(
            &trader,
            &bootstrap.token0_mint,
            &bootstrap.token0_program,
            false,
        );
        let user1 = get_associated_token_address_with_program_id_fast_use_seed(
            &trader,
            &bootstrap.token1_mint,
            &bootstrap.token1_program,
            false,
        );
        let keys = [
            pool_address,
            bootstrap.amm_config,
            get_creator_fee_share_pda(&creator, &bootstrap.amm_config),
            bootstrap.token0_vault,
            bootstrap.token1_vault,
            collect.accounts[8].pubkey,
            collect.accounts[9].pubkey,
            bootstrap.token0_mint,
            bootstrap.token1_mint,
            user0,
            user1,
        ];
        let before =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        let pool = decode_pool(required(&before.value, 0)?)?;
        ensure!(
            pool.pool_creator == creator
                && pool.amm_config == bootstrap.amm_config
                && pool.token0_mint == bootstrap.token0_mint
                && pool.token1_mint == bootstrap.token1_mint,
            "Pool identity changed"
        );
        ensure!(
            pool.creator_fees_token0 != 0 || pool.creator_fees_token1 != 0,
            "Pool has no creator fees"
        );
        let config = decode_config(required(&before.value, 1)?)?;
        let rate = resolve_creator_fee_share_rate(
            &config,
            &creator,
            &pool.amm_config,
            before.value[2].as_ref(),
        )?;
        ensure!(
            token_amount(
                required(&before.value, 6)?,
                &pool.token1_mint,
                &creator,
                &pool.token1_program
            )? >= FUNDING_AMOUNT,
            "Creator token1 ATA cannot fund simulation"
        );
        let epoch = rpc.get_epoch_info().await.context("Read epoch for swap comparison")?.epoch;
        let protocol = protocol_params(pool_address, &pool, &config, &before.value, epoch)?;
        let quote = compute_swap_amount_for_pool(&protocol, false, AMOUNT_IN, 0)?;
        ensure!(quote.amount_out > 1, "Quote is too small for exact-output comparison");
        let mut params = swap_params(
            wallet.clone(),
            TradeType::Buy,
            pool.token1_mint,
            pool.token0_mint,
            AMOUNT_IN,
            0,
            DexParamEnum::RaydiumCpmm(protocol),
        );
        params.create_input_mint_ata = false;
        params.create_output_mint_ata = false;
        params.fixed_output_amount = if exact_output { Some(quote.amount_out / 2) } else { None };
        let swap = RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await?;
        ensure!(
            swap.len() == 1 && swap[0].program_id == accounts::RAYDIUM_CPMM,
            "Expected one SDK-built swap"
        );
        let mut setup =
            vec![solana_system_interface::instruction::transfer(&creator, &trader, 20_000_000)];
        for (mint, program) in
            [(pool.token0_mint, pool.token0_program), (pool.token1_mint, pool.token1_program)]
        {
            setup.extend(create_associated_token_account_idempotent_fast_use_seed(
                &trader, &trader, &mint, &program, false,
            ));
        }
        setup.push(spl_token_2022_interface::instruction::transfer_checked(
            &pool.token1_program,
            &keys[6],
            &pool.token1_mint,
            &user1,
            &creator,
            &[],
            FUNDING_AMOUNT,
            pool.mint1_decimals,
        )?);
        let mut baseline_ixs = setup.clone();
        baseline_ixs.extend(swap.clone());
        let mut collected_ixs = setup;
        collected_ixs.push(collect);
        collected_ixs.extend(swap.clone());
        let baseline = simulate(rpc, &baseline_ixs, creator, &keys, before.context.slot)
            .await
            .context("Baseline swap simulation")?;
        let collected = simulate(rpc, &collected_ixs, creator, &keys, baseline.slot)
            .await
            .context("Collection plus swap simulation")?;
        let after =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        if after.context.slot < collected.slot || !unchanged(&before.value, &after.value) {
            if attempt == 3 {
                bail!("Pool changed during comparison; choose a quieter pool");
            }
            continue;
        }
        let a = decode_pool(required(&baseline.accounts, 0)?)?;
        let b = decode_pool(required(&collected.accounts, 0)?)?;
        ensure!(
            a.recent_epoch == epoch && b.recent_epoch == epoch,
            "Epoch changed during comparison"
        );
        let input_a = token_amount(
            required(&baseline.accounts, 10)?,
            &pool.token1_mint,
            &trader,
            &pool.token1_program,
        )?;
        let input_b = token_amount(
            required(&collected.accounts, 10)?,
            &pool.token1_mint,
            &trader,
            &pool.token1_program,
        )?;
        let output_a = token_amount(
            required(&baseline.accounts, 9)?,
            &pool.token0_mint,
            &trader,
            &pool.token0_program,
        )?;
        let output_b = token_amount(
            required(&collected.accounts, 9)?,
            &pool.token0_mint,
            &trader,
            &pool.token0_program,
        )?;
        ensure!(input_a == input_b && output_a == output_b, "Collection changed swap input/output");
        let funded =
            FUNDING_AMOUNT - transfer_fee(required(&before.value, 8)?, epoch, FUNDING_AMOUNT)?;
        let spent = funded.checked_sub(input_a).context("Invalid simulated input balance")?;
        if exact_output {
            ensure!(
                output_a == params.fixed_output_amount.unwrap() && spent <= AMOUNT_IN,
                "Exact-output amount/input cap mismatch"
            );
        } else {
            ensure!(
                output_a == quote.amount_out && spent == AMOUNT_IN,
                "Exact-input SDK quote does not match mainnet execution"
            );
        }
        let a_params = protocol_params(pool_address, &a, &config, &baseline.accounts, epoch)?;
        let b_params = protocol_params(pool_address, &b, &config, &collected.accounts, epoch)?;
        ensure!(
            a_params.base_reserve == b_params.base_reserve
                && a_params.quote_reserve == b_params.quote_reserve,
            "Collection changed LP reserves after swap"
        );
        let next_a = compute_swap_amount_for_pool(&a_params, false, AMOUNT_IN, 0)?;
        let next_b = compute_swap_amount_for_pool(&b_params, false, AMOUNT_IN, 0)?;
        ensure!(next_a.amount_out == next_b.amount_out, "Collection changed subsequent SDK quote");
        for (index, gross, creator_a, creator_b, protocol_a, protocol_b) in [
            (
                0,
                pool.creator_fees_token0,
                a.creator_fees_token0,
                b.creator_fees_token0,
                a.protocol_fees_token0,
                b.protocol_fees_token0,
            ),
            (
                1,
                pool.creator_fees_token1,
                a.creator_fees_token1,
                b.creator_fees_token1,
                a.protocol_fees_token1,
                b.protocol_fees_token1,
            ),
        ] {
            let (payout, shared) = split_creator_fee(gross, rate)?;
            ensure!(
                creator_a.checked_sub(gross) == Some(creator_b)
                    && protocol_a.checked_add(shared) == Some(protocol_b),
                "Collection/swap accounting mismatch token{index}"
            );
            let mint = if index == 0 { pool.token0_mint } else { pool.token1_mint };
            let program = if index == 0 { pool.token0_program } else { pool.token1_program };
            let vault_a = token_amount(
                required(&baseline.accounts, 3 + index)?,
                &mint,
                &accounts::AUTHORITY,
                &program,
            )?;
            let vault_b = token_amount(
                required(&collected.accounts, 3 + index)?,
                &mint,
                &accounts::AUTHORITY,
                &program,
            )?;
            ensure!(
                vault_a.checked_sub(vault_b) == Some(payout),
                "Collection vault delta mismatch token{index}"
            );
        }
        let event_a = swap_event(&baseline)?;
        let event_b = swap_event(&collected)?;
        ensure!(
            event_a.pool_id == pool_address
                && event_b.pool_id == pool_address
                && event_a.base_input == !exact_output
                && event_b.base_input == !exact_output,
            "Parser swap identity/mode mismatch"
        );
        ensure!(
            event_a.input_amount == event_b.input_amount
                && event_a.output_amount == event_b.output_amount
                && event_a.input_transfer_fee == event_b.input_transfer_fee
                && event_a.output_transfer_fee == event_b.output_transfer_fee,
            "Parsed swap log amounts differ"
        );
        ensure!(
            event_a.input_amount.checked_add(event_a.input_transfer_fee) == Some(spent)
                && event_a.output_amount.checked_sub(event_a.output_transfer_fee) == Some(output_a),
            "Parsed log amounts do not match token deltas"
        );
        return Ok(
            json!({"validated":true,"pool":pool_address.to_string(),"trader":trader.to_string(),"permissionless":permissionless,"exact_output":exact_output,
            "share_rate":rate,"input_budget":AMOUNT_IN.to_string(),"funding_amount":FUNDING_AMOUNT.to_string(),"funded_trader_amount":funded.to_string(),"quoted_exact_input_output":quote.amount_out.to_string(),"actual_input":spent.to_string(),"actual_output":output_a.to_string(),
            "next_quote":next_a.amount_out.to_string(),"snapshot_slot":before.context.slot,"baseline_slot":baseline.slot,"collected_slot":collected.slot,"verification_slot":after.context.slot,
            "baseline_units":baseline.units,"collected_units":collected.units,"baseline_logs":baseline.logs,"collected_logs":collected.logs,
            "before_accounts":snapshot(&keys,&before.value),"baseline_accounts":snapshot(&keys,&baseline.accounts),"collected_accounts":snapshot(&keys,&collected.accounts)}),
        );
    }
    unreachable!()
}

pub async fn run_from_env() -> Result<Value> {
    let url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_owned());
    let pool =
        std::env::var("CPMM_POOL").unwrap_or_else(|_| DEFAULT_SWAP_POOL.to_owned()).parse()?;
    let payer = std::env::var("CPMM_PAYER")
        .unwrap_or_else(|_| collection::DEFAULT_PAYER.to_owned())
        .parse()?;
    let rpc = SolanaRpcClient::new_with_commitment(url, CommitmentConfig::confirmed());
    let mut comparisons = Vec::new();
    for permissionless in [false, true] {
        for exact_output in [false, true] {
            comparisons
                .push(verify_swap_pair(&rpc, pool, payer, permissionless, exact_output).await?);
        }
    }
    let report = json!({"kind":"mainnet_collection_swap_comparison","broadcast":false,"signature_verification":false,"comparisons":comparisons});
    if let Ok(path) = std::env::var("CPMM_REPORT_PATH") {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
    }
    Ok(report)
}
pub(crate) fn swap_params(
    wallet: Arc<Keypair>,
    trade_type: TradeType,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    slippage_bps: u64,
    protocol_params: DexParamEnum,
) -> SwapParams {
    SwapParams {
        rpc: None,
        payer: wallet,
        trade_type,
        input_mint,
        input_token_program: None,
        output_mint,
        output_token_program: None,
        input_amount: Some(amount),
        slippage_basis_points: Some(slippage_bps),
        address_lookup_table_accounts: Vec::new(),
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params,
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
        gas_fee_strategy: GasFeeStrategy::new(),
        simulate: true,
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

/// Revalidate captured RPC account bytes and event logs without network access.
#[cfg(test)]
pub(super) fn verify_saved_report(report: &Value) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    fn decode_accounts(value: &Value) -> Result<Vec<Option<Account>>> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        value
            .as_array()
            .context("Missing account snapshots")?
            .iter()
            .map(|a| {
                if a.is_null() {
                    return Ok(None);
                }
                Ok(Some(Account {
                    owner: a["owner"].as_str().context("Missing owner")?.parse()?,
                    lamports: a["lamports"].as_u64().context("Missing lamports")?,
                    executable: a["executable"].as_bool().context("Missing executable")?,
                    rent_epoch: a["rent_epoch"].as_u64().context("Missing rent epoch")?,
                    data: STANDARD
                        .decode(a["data_base64"].as_str().context("Missing account bytes")?)?,
                }))
            })
            .collect()
    }
    ensure!(
        report["broadcast"] == false && report["signature_verification"] == false,
        "Unexpected report execution mode"
    );
    let rows = report["comparisons"].as_array().context("Missing comparisons")?;
    ensure!(rows.len() == 4, "Expected all four comparison modes");
    let mut modes = std::collections::HashSet::new();
    for r in rows {
        let permissionless = r["permissionless"].as_bool().context("Missing collection mode")?;
        let exact_output = r["exact_output"].as_bool().context("Missing swap mode")?;
        ensure!(modes.insert((permissionless, exact_output)), "Duplicate comparison mode");
        let before = decode_accounts(&r["before_accounts"])?;
        let baseline = decode_accounts(&r["baseline_accounts"])?;
        let collected = decode_accounts(&r["collected_accounts"])?;
        ensure!(
            before.len() == 11 && baseline.len() == 11 && collected.len() == 11,
            "Wrong snapshot shape"
        );
        let initial = decode_pool(required(&before, 0)?)?;
        let a = decode_pool(required(&baseline, 0)?)?;
        let b = decode_pool(required(&collected, 0)?)?;
        let pool = r["pool"].as_str().context("Missing pool")?.parse()?;
        let trader = r["trader"].as_str().context("Missing trader")?.parse()?;
        let config = decode_config(required(&before, 1)?)?;
        let rate = resolve_creator_fee_share_rate(
            &config,
            &initial.pool_creator,
            &initial.amm_config,
            before[2].as_ref(),
        )?;
        ensure!(r["share_rate"] == rate, "Reported rate differs from raw account data");
        let budget = r["input_budget"].as_str().context("Missing input budget")?.parse::<u64>()?;
        let initial_params = protocol_params(pool, &initial, &config, &before, a.recent_epoch)?;
        let quote = compute_swap_amount_for_pool(&initial_params, false, budget, 0)?;
        ensure!(
            r["quoted_exact_input_output"] == quote.amount_out.to_string(),
            "Captured SDK quote mismatch"
        );
        let funding =
            r["funding_amount"].as_str().context("Missing funding amount")?.parse::<u64>()?;
        let funded = funding - transfer_fee(required(&before, 8)?, a.recent_epoch, funding)?;
        let get_balances = |accounts: &[Option<Account>]| -> Result<(u64, u64)> {
            Ok((
                token_amount(
                    required(accounts, 10)?,
                    &initial.token1_mint,
                    &trader,
                    &initial.token1_program,
                )?,
                token_amount(
                    required(accounts, 9)?,
                    &initial.token0_mint,
                    &trader,
                    &initial.token0_program,
                )?,
            ))
        };
        let (remaining, output) = get_balances(&baseline)?;
        ensure!(get_balances(&collected)? == (remaining, output), "Raw trader balances differ");
        let spent = funded.checked_sub(remaining).context("Invalid trader input balance")?;
        ensure!(
            r["funded_trader_amount"] == funded.to_string()
                && r["actual_input"] == spent.to_string()
                && r["actual_output"] == output.to_string(),
            "Reported amounts differ from raw balances"
        );
        if exact_output {
            ensure!(spent <= budget && output == quote.amount_out / 2, "Exact-output mismatch");
        } else {
            ensure!(spent == budget && output == quote.amount_out, "Exact-input quote mismatch");
        }
        let a_params = protocol_params(pool, &a, &config, &baseline, a.recent_epoch)?;
        let b_params = protocol_params(pool, &b, &config, &collected, b.recent_epoch)?;
        ensure!(
            a_params.base_reserve == b_params.base_reserve
                && a_params.quote_reserve == b_params.quote_reserve,
            "LP reserves differ"
        );
        let next_a = compute_swap_amount_for_pool(&a_params, false, budget, 0)?;
        let next_b = compute_swap_amount_for_pool(&b_params, false, budget, 0)?;
        ensure!(
            next_a.amount_out == next_b.amount_out
                && r["next_quote"] == next_a.amount_out.to_string(),
            "Subsequent quotes differ"
        );
        for (index, gross, creator_a, creator_b, protocol_a, protocol_b) in [
            (
                0,
                initial.creator_fees_token0,
                a.creator_fees_token0,
                b.creator_fees_token0,
                a.protocol_fees_token0,
                b.protocol_fees_token0,
            ),
            (
                1,
                initial.creator_fees_token1,
                a.creator_fees_token1,
                b.creator_fees_token1,
                a.protocol_fees_token1,
                b.protocol_fees_token1,
            ),
        ] {
            let (payout, shared) = split_creator_fee(gross, rate)?;
            ensure!(
                creator_a.checked_sub(gross) == Some(creator_b)
                    && protocol_a.checked_add(shared) == Some(protocol_b),
                "Raw fee accounting mismatch token{index}"
            );
            let mint = if index == 0 { initial.token0_mint } else { initial.token1_mint };
            let program = if index == 0 { initial.token0_program } else { initial.token1_program };
            ensure!(
                token_amount(
                    required(&baseline, 3 + index)?,
                    &mint,
                    &accounts::AUTHORITY,
                    &program
                )?
                .checked_sub(token_amount(
                    required(&collected, 3 + index)?,
                    &mint,
                    &accounts::AUTHORITY,
                    &program
                )?) == Some(payout),
                "Raw vault accounting mismatch"
            );
        }
        let make_sim = |logs: &str, slot: &str| -> Result<Simulation> {
            Ok(Simulation {
                slot: r[slot].as_u64().context("Missing simulation slot")?,
                accounts: Vec::new(),
                logs: serde_json::from_value(r[logs].clone())?,
                units: None,
            })
        };
        let event_a = swap_event(&make_sim("baseline_logs", "baseline_slot")?)?;
        let event_b = swap_event(&make_sim("collected_logs", "collected_slot")?)?;
        ensure!(
            event_a.input_amount == event_b.input_amount
                && event_a.output_amount == event_b.output_amount,
            "Captured parsed log amounts differ"
        );
        ensure!(
            event_a.base_input == !exact_output
                && event_b.base_input == !exact_output
                && event_a.pool_id == pool
                && event_b.pool_id == pool,
            "Captured event mode/pool mismatch"
        );
        ensure!(
            event_a.input_amount.checked_add(event_a.input_transfer_fee) == Some(spent)
                && event_a.output_amount.checked_sub(event_a.output_transfer_fee) == Some(output),
            "Captured events do not match token deltas"
        );
        // Parse post-swap pool bytes through the parser SDK as an independent decoder.
        for phase in ["baseline_accounts", "collected_accounts"] {
            let raw = &r[phase][0];
            let data =
                STANDARD.decode(raw["data_base64"].as_str().context("Missing pool bytes")?)?;
            let event = sol_parser_sdk::accounts::parse_account_unified(
                &sol_parser_sdk::accounts::AccountData {
                    pubkey: pool,
                    owner: accounts::RAYDIUM_CPMM,
                    data,
                    lamports: 1,
                    executable: false,
                    rent_epoch: 0,
                },
                sol_parser_sdk::core::events::EventMetadata::default(),
                None,
            )
            .context("Parser failed to decode post-swap pool")?;
            let sol_parser_sdk::core::events::DexEvent::RaydiumCpmmPoolStateAccount(e) = event
            else {
                bail!("Wrong parser account event");
            };
            let expected = if phase == "baseline_accounts" { &a } else { &b };
            ensure!(
                e.pool_state.creator_fees_token_0 == expected.creator_fees_token0
                    && e.pool_state.creator_fees_token_1 == expected.creator_fees_token1
                    && e.pool_state.protocol_fees_token_0 == expected.protocol_fees_token0
                    && e.pool_state.protocol_fees_token_1 == expected.protocol_fees_token1,
                "Parser/trade post-swap fee counters differ"
            );
        }
    }
    Ok(())
}
