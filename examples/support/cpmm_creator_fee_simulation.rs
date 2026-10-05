//! Read-only mainnet simulation and post-state assertions, shared by example and tests.
use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use sol_trade_sdk::{
    common::SolanaRpcClient,
    instruction::{
        raydium_cpmm::{collect_creator_fee, collect_creator_fee_permissionless},
        utils::{
            raydium_cpmm::{
                accounts, fetch_pool_state, get_creator_fee_share_pda,
                resolve_creator_fee_share_rate, split_creator_fee,
            },
            raydium_cpmm_types::{
                amm_config_decode, pool_state_decode, PoolState, AMM_CONFIG_DISCRIMINATOR,
                POOL_STATE_DISCRIMINATOR,
            },
        },
    },
};
use solana_client::rpc_config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account, hash::Hash, instruction::Instruction, message::Message, pubkey::Pubkey,
    signature::Signature, transaction::Transaction,
};
use spl_token_2022_interface::{
    extension::{
        transfer_fee::{TransferFeeAmount, TransferFeeConfig},
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account as TokenAccount, Mint},
};

pub const DEFAULT_POOL: &str = "BUVzsLLLG7GWoyJVoU31pXiBveazA6GXTavZ9VD3CwS9";
pub const DEFAULT_PAYER: &str = "BH23VsesJX61Ve3ZgHGzPKgcgCcMKMhpGGAqNeG3NFou";

type SimulationResponse = solana_client::rpc_response::Response<
    solana_client::rpc_response::RpcSimulateTransactionResult,
>;

pub(crate) fn parse_simulation_response(raw: Value) -> Result<SimulationResponse> {
    ensure!(
        raw.get("value").and_then(|v| v.get("err")).is_some(),
        "Simulation response missing explicit execution status"
    );
    ensure!(
        raw.get("context").and_then(|v| v.get("slot")).and_then(Value::as_u64).is_some(),
        "Simulation response missing slot"
    );
    Ok(serde_json::from_value(raw)?)
}

/// Explicit simulation only. Require an execution status before typed decoding,
/// because serde maps both a missing optional err and err:null to None.
pub(crate) async fn simulate_strict(
    rpc: &SolanaRpcClient,
    tx: &Transaction,
    mut config: RpcSimulateTransactionConfig,
) -> Result<SimulationResponse> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    config.encoding = Some(solana_transaction_status_client_types::UiTransactionEncoding::Base64);
    config.inner_instructions = true;
    let wire = STANDARD.encode(wincode::serialize(tx)?);
    let raw = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for attempt in 0..4 {
            match rpc
                .send::<Value>(
                    solana_client::rpc_request::RpcRequest::SimulateTransaction,
                    json!([wire, config]),
                )
                .await
            {
                Ok(raw) => return Ok(raw),
                Err(error)
                    if attempt < 3
                        && matches!(
                            error.kind(),
                            solana_client::client_error::ClientErrorKind::RpcError(
                                solana_client::rpc_request::RpcError::RpcResponseError {
                                    code: -32016,
                                    ..
                                }
                            )
                        ) =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    })
    .await
    .context("Simulation RPC exceeded 30 second deadline")??;
    if let Some(directory) = std::env::var_os("CPMM_SIMULATION_DIR") {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        let path = directory
            .join(format!("simulation-{:03}.json", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("Simulation evidence path already exists or cannot be created")?;
        // Save failures too, before validation. These unsigned funding-prefix
        // transactions verify execution only and must never be broadcast.
        file.write_all(&serde_json::to_vec_pretty(&json!({
            "wire": wire, "response": {"jsonrpc": "2.0", "result": raw}, "broadcast": false,
            "signature_verification": false, "simulation_only_funding": true,
        }))?)?;
        file.write_all(b"\n")?;
    }
    let response = parse_simulation_response(raw)?;
    if let Some(min_slot) = config.min_context_slot {
        ensure!(response.context.slot >= min_slot, "Simulation bank predates requested snapshot");
    }
    Ok(response)
}

pub(crate) fn required<'a>(list: &'a [Option<Account>], index: usize) -> Result<&'a Account> {
    list.get(index)
        .and_then(Option::as_ref)
        .ok_or_else(|| anyhow!("Missing snapshot account {index}"))
}
pub(crate) fn decode_pool(account: &Account) -> Result<PoolState> {
    ensure!(account.owner == accounts::RAYDIUM_CPMM, "Pool owner mismatch");
    ensure!(
        account.data.get(..8) == Some(POOL_STATE_DISCRIMINATOR.as_slice()),
        "Pool discriminator mismatch"
    );
    pool_state_decode(&account.data[8..]).context("Invalid PoolState")
}
pub(crate) fn decode_config(
    account: &Account,
) -> Result<sol_trade_sdk::instruction::utils::raydium_cpmm_types::AmmConfig> {
    ensure!(account.owner == accounts::RAYDIUM_CPMM, "Config owner mismatch");
    ensure!(
        account.data.get(..8) == Some(AMM_CONFIG_DISCRIMINATOR.as_slice()),
        "Config discriminator mismatch"
    );
    amm_config_decode(&account.data[8..]).context("Invalid AmmConfig")
}
pub(crate) fn token_amount(
    account: &Account,
    mint: &Pubkey,
    owner: &Pubkey,
    program: &Pubkey,
) -> Result<u64> {
    ensure!(account.owner == *program, "Token account program mismatch");
    ensure!(
        *program == sol_trade_sdk::constants::TOKEN_PROGRAM
            || *program == spl_token_2022_interface::ID,
        "Unsupported token account program"
    );
    let state = StateWithExtensions::<TokenAccount>::unpack(&account.data)
        .context("Invalid or uninitialized token account")?;
    ensure!(state.base.mint == *mint, "Token account mint mismatch");
    ensure!(state.base.owner == *owner, "Token account authority mismatch");
    Ok(state.base.amount)
}
pub(crate) fn transfer_fee(mint: &Account, epoch: u64, amount: u64) -> Result<u64> {
    if mint.owner == sol_trade_sdk::constants::TOKEN_PROGRAM {
        return Ok(0);
    }
    ensure!(
        mint.owner.to_bytes() == spl_token_2022_interface::ID.to_bytes(),
        "Unsupported mint program"
    );
    let state = StateWithExtensions::<Mint>::unpack(&mint.data)?;
    if !state.get_extension_types()?.contains(&ExtensionType::TransferFeeConfig) {
        return Ok(0);
    }
    state
        .get_extension::<TransferFeeConfig>()?
        .calculate_epoch_fee(epoch, amount)
        .context("Transfer fee overflow")
}
pub(crate) fn withheld_amount(account: &Account) -> Result<u64> {
    if account.owner == sol_trade_sdk::constants::TOKEN_PROGRAM {
        return Ok(0);
    }
    ensure!(
        account.owner.to_bytes() == spl_token_2022_interface::ID.to_bytes(),
        "Unsupported token account program"
    );
    let state = StateWithExtensions::<TokenAccount>::unpack(&account.data)?;
    if !state.get_extension_types()?.contains(&ExtensionType::TransferFeeAmount) {
        return Ok(0);
    }
    Ok(state.get_extension::<TransferFeeAmount>()?.withheld_amount.into())
}

/// Preserve explicit RPC nulls, but never turn a malformed account into a null.
pub(crate) fn decode_simulated_accounts(
    accounts: &[Option<solana_account_decoder::UiAccount>],
    expected_len: usize,
) -> Result<Vec<Option<Account>>> {
    ensure!(accounts.len() == expected_len, "Incomplete simulated accounts");
    accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            let Some(account) = account else {
                return Ok(None);
            };
            Ok(Some(Account {
                lamports: account.lamports,
                data: account
                    .data
                    .decode()
                    .with_context(|| format!("Invalid simulated account data at index {index}"))?,
                owner: account
                    .owner
                    .parse()
                    .with_context(|| format!("Invalid simulated account owner at index {index}"))?,
                executable: account.executable,
                rent_epoch: account.rent_epoch,
            }))
        })
        .collect()
}

pub(crate) fn unchanged(before: &[Option<Account>], after: &[Option<Account>]) -> bool {
    before.len() == after.len()
        && before.iter().zip(after).all(|(a, b)| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            (None, Some(a)) | (Some(a), None) => {
                a.lamports == 0
                    && a.data.is_empty()
                    && a.owner == Pubkey::default()
                    && !a.executable
            }
        })
}

pub(crate) fn snapshot(keys: &[Pubkey], accounts: &[Option<Account>]) -> Value {
    use base64::{engine::general_purpose::STANDARD, Engine};
    json!(keys
        .iter()
        .zip(accounts)
        .map(|(key, account)| {
            account.as_ref().map(|a|json!({
        "pubkey":key.to_string(),"owner":a.owner.to_string(),"lamports":a.lamports,
        "executable":a.executable,"rent_epoch":a.rent_epoch,"data_base64":STANDARD.encode(&a.data)
    }))
        })
        .collect::<Vec<_>>())
}

/// Only simulateTransaction is used. Default signatures skip authentication but
/// preserve signer account metas; this verifies instruction execution, not key ownership.
pub async fn verify_collection(
    rpc: &SolanaRpcClient,
    pool_address: Pubkey,
    payer: Pubkey,
    permissionless: bool,
) -> Result<Value> {
    for attempt in 0..4 {
        let bootstrap = fetch_pool_state(rpc, &pool_address).await?;
        let ix = if permissionless {
            collect_creator_fee_permissionless(&payer, &pool_address, &bootstrap)
        } else {
            collect_creator_fee(&pool_address, &bootstrap)
        };
        let signer = if permissionless { payer } else { bootstrap.pool_creator };
        let keys = [
            pool_address,
            bootstrap.amm_config,
            get_creator_fee_share_pda(&bootstrap.pool_creator, &bootstrap.amm_config),
            bootstrap.token0_vault,
            bootstrap.token1_vault,
            ix.accounts[8].pubkey,
            ix.accounts[9].pubkey,
            bootstrap.token0_mint,
            bootstrap.token1_mint,
        ];
        let before =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        let pool = decode_pool(required(&before.value, 0)?)?;
        ensure!(
            pool.amm_config == bootstrap.amm_config
                && pool.pool_creator == bootstrap.pool_creator
                && pool.token0_vault == bootstrap.token0_vault
                && pool.token1_vault == bootstrap.token1_vault
                && pool.token0_mint == bootstrap.token0_mint
                && pool.token1_mint == bootstrap.token1_mint
                && pool.token0_program == bootstrap.token0_program
                && pool.token1_program == bootstrap.token1_program,
            "Pool identity changed during bootstrap"
        );
        ensure!(
            pool.creator_fees_token0 != 0 || pool.creator_fees_token1 != 0,
            "Pool has no accrued creator fees; choose another pool"
        );
        let config = decode_config(required(&before.value, 1)?)?;
        let rate = resolve_creator_fee_share_rate(
            &config,
            &pool.pool_creator,
            &pool.amm_config,
            before.value[2].as_ref(),
        )?;
        let tx = unsigned_transaction(ix.clone(), signer);
        let simulation = simulate_strict(
            rpc,
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::confirmed()),
                min_context_slot: Some(before.context.slot),
                inner_instructions: true,
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                    addresses: keys.iter().map(ToString::to_string).collect(),
                }),
                ..Default::default()
            },
        )
        .await?;
        if let Some(error) = simulation.value.err.as_ref() {
            bail!("Collection simulation failed: {error:?}; logs: {:?}", simulation.value.logs);
        }
        // Discard comparisons if the chain changed between reads and simulation.
        // Same-slot snapshots are exact; stable bracketing reads also allow a later bank.
        let after =
            rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::confirmed()).await?;
        if after.context.slot < simulation.context.slot || !unchanged(&before.value, &after.value) {
            if attempt == 3 {
                bail!(
                    "Pool/config/token state changed during simulation; retry with a quieter pool"
                );
            }
            continue;
        }
        let ui =
            simulation.value.accounts.as_ref().context("RPC did not return simulated accounts")?;
        let post = decode_simulated_accounts(ui, keys.len())?;
        let post_pool = decode_pool(required(&post, 0)?)?;
        let mut checks = Vec::new();
        for (i, gross, protocol_before, protocol_after, mint, program) in [
            (
                0,
                pool.creator_fees_token0,
                pool.protocol_fees_token0,
                post_pool.protocol_fees_token0,
                pool.token0_mint,
                pool.token0_program,
            ),
            (
                1,
                pool.creator_fees_token1,
                pool.protocol_fees_token1,
                post_pool.protocol_fees_token1,
                pool.token1_mint,
                pool.token1_program,
            ),
        ] {
            let (creator, protocol) = split_creator_fee(gross, rate)?;
            ensure!(
                protocol_after
                    == protocol_before.checked_add(protocol).context("Protocol fee overflow")?,
                "Protocol share mismatch token{i}"
            );
            let vault_before = token_amount(
                required(&before.value, 3 + i)?,
                &mint,
                &accounts::AUTHORITY,
                &program,
            )?;
            let vault_after =
                token_amount(required(&post, 3 + i)?, &mint, &accounts::AUTHORITY, &program)?;
            ensure!(
                vault_before.checked_sub(vault_after) == Some(creator),
                "Vault payout mismatch token{i}"
            );
            let recipient_before = before.value[5 + i]
                .as_ref()
                .map(|a| token_amount(a, &mint, &pool.pool_creator, &program))
                .transpose()?
                .unwrap_or(0);
            let recipient_after =
                token_amount(required(&post, 5 + i)?, &mint, &pool.pool_creator, &program)?;
            let fee =
                transfer_fee(required(&before.value, 7 + i)?, post_pool.recent_epoch, creator)?;
            ensure!(
                recipient_after.checked_sub(recipient_before) == Some(creator - fee),
                "Creator net payout mismatch token{i}"
            );
            let withheld_before =
                before.value[5 + i].as_ref().map(withheld_amount).transpose()?.unwrap_or(0);
            let withheld_after = withheld_amount(required(&post, 5 + i)?)?;
            ensure!(
                withheld_after.checked_sub(withheld_before) == Some(fee),
                "Recipient withheld transfer fee mismatch token{i}"
            );
            ensure!(
                withheld_amount(required(&before.value, 3 + i)?)?
                    == withheld_amount(required(&post, 3 + i)?)?,
                "Vault withheld fees changed token{i}"
            );
            let fund = if i == 0 { pool.fund_fees_token0 } else { pool.fund_fees_token1 };
            let reserve_before = u128::from(vault_before)
                .checked_sub(u128::from(protocol_before) + u128::from(fund) + u128::from(gross))
                .context("Invalid pre-collection reserves")?;
            let reserve_after = u128::from(vault_after)
                .checked_sub(u128::from(protocol_after) + u128::from(fund))
                .context("Invalid post-collection reserves")?;
            ensure!(reserve_before == reserve_after, "LP reserves changed token{i}");
            checks.push(json!({"token":i,"gross":gross.to_string(),"creator_before_transfer_fee":creator.to_string(),
                "protocol_share":protocol.to_string(),"transfer_fee":fee.to_string(),"net_creator_payout":(creator-fee).to_string(),
                "vault_before":vault_before.to_string(),"vault_after":vault_after.to_string(),
                "recipient_before":recipient_before.to_string(),"recipient_after":recipient_after.to_string(),
                "protocol_before":protocol_before.to_string(),"protocol_after":protocol_after.to_string(),
                "withheld_before":withheld_before.to_string(),"withheld_after":withheld_after.to_string(),
                "lp_reserve_before":reserve_before.to_string(),"lp_reserve_after":reserve_after.to_string()}));
        }
        ensure!(
            post_pool.creator_fees_token0 == 0 && post_pool.creator_fees_token1 == 0,
            "Creator fees not cleared"
        );
        let mut expected = pool.clone();
        expected.creator_fees_token0 = 0;
        expected.creator_fees_token1 = 0;
        expected.protocol_fees_token0 = post_pool.protocol_fees_token0;
        expected.protocol_fees_token1 = post_pool.protocol_fees_token1;
        expected.recent_epoch = post_pool.recent_epoch;
        ensure!(post_pool == expected, "Collection unexpectedly changed other PoolState fields");
        ensure!(
            unchanged(&before.value[1..3], &post[1..3]),
            "Collection changed config/share account"
        );
        // Negative control: old clients omit the appended account(s) and must fail 3005.
        let mut legacy = ix.clone();
        legacy.accounts.truncate(14);
        let failure = simulate_strict(
            rpc,
            &unsigned_transaction(legacy, signer),
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::confirmed()),
                min_context_slot: Some(simulation.context.slot),
                ..Default::default()
            },
        )
        .await?;
        let legacy_error = serde_json::to_value(&failure.value.err)?;
        ensure!(
            legacy_error == json!({"InstructionError":[0,{"Custom":3005}]}),
            "Expected AccountNotEnoughKeys (3005), got {legacy_error}"
        );
        let mut wrong_pda = ix.clone();
        wrong_pda.accounts.last_mut().unwrap().pubkey = pool.amm_config;
        let wrong = simulate_strict(
            rpc,
            &unsigned_transaction(wrong_pda, signer),
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::confirmed()),
                min_context_slot: Some(simulation.context.slot),
                ..Default::default()
            },
        )
        .await?;
        let wrong_pda_error = serde_json::to_value(&wrong.value.err)?;
        ensure!(
            wrong_pda_error == json!({"InstructionError":[0,{"Custom":2006}]}),
            "Expected ConstraintSeeds (2006), got {wrong_pda_error}"
        );
        // Repeating collection in one transaction must fail after the first clears fees.
        let duplicate = simulate_strict(
            rpc,
            &unsigned_instructions(&[ix.clone(), ix.clone()], signer),
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::confirmed()),
                min_context_slot: Some(simulation.context.slot),
                ..Default::default()
            },
        )
        .await?;
        let duplicate_error = serde_json::to_value(&duplicate.value.err)?;
        ensure!(
            duplicate_error == json!({"InstructionError":[1,{"Custom":6014}]}),
            "Expected second collection NoFeeCollect (6014), got {duplicate_error}"
        );
        let missing_share_error = if permissionless {
            let mut no_share = ix.clone();
            no_share.accounts.truncate(15);
            let missing = simulate_strict(
                rpc,
                &unsigned_transaction(no_share, signer),
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::confirmed()),
                    min_context_slot: Some(simulation.context.slot),
                    ..Default::default()
                },
            )
            .await?;
            let error = serde_json::to_value(&missing.value.err)?;
            ensure!(
                error == json!({"InstructionError":[0,{"Custom":3005}]}),
                "Missing share PDA did not fail with 3005: {error}"
            );
            Some(error)
        } else {
            None
        };
        use base64::{engine::general_purpose::STANDARD, Engine};
        return Ok(
            json!({"pool":pool_address.to_string(),"creator":pool.pool_creator.to_string(),"payer":signer.to_string(),
            "permissionless":permissionless,"account_count":ix.accounts.len(),"share_pda":keys[2].to_string(),
            "share_account_exists":before.value[2].as_ref().is_some_and(|a|a.owner==accounts::RAYDIUM_CPMM&&!a.data.is_empty()),
            "share_rate":rate,"snapshot_slot":before.context.slot,"simulation_slot":simulation.context.slot,
            "verification_slot":after.context.slot,"units_consumed":simulation.value.units_consumed,
            "validated":true,"checks":checks,"legacy_error":legacy_error,"wrong_pda_error":wrong_pda_error,
            "duplicate_collection_error":duplicate_error,"permissionless_missing_share_error":missing_share_error,
            "config_share_rate":config.creator_fee_share_rate,
            "before_accounts":snapshot(&keys,&before.value),"simulated_accounts":snapshot(&keys,&post),
            "instruction":{"program_id":ix.program_id.to_string(),"data_base64":STANDARD.encode(&ix.data),
                "accounts":ix.accounts.iter().map(|a|a.pubkey.to_string()).collect::<Vec<_>>()},
            "logs":simulation.value.logs}),
        );
    }
    unreachable!()
}
fn unsigned_transaction(ix: Instruction, payer: Pubkey) -> Transaction {
    unsigned_instructions(&[ix], payer)
}
pub(crate) fn unsigned_instructions(instructions: &[Instruction], payer: Pubkey) -> Transaction {
    let message = Message::new_with_blockhash(instructions, Some(&payer), &Hash::default());
    Transaction {
        signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
        message,
    }
}

pub async fn run_from_env() -> Result<Value> {
    let rpc_url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://solana-rpc.publicnode.com".to_owned());
    let pool = std::env::var("CPMM_POOL").unwrap_or_else(|_| DEFAULT_POOL.to_owned()).parse()?;
    let payer = std::env::var("CPMM_PAYER").unwrap_or_else(|_| DEFAULT_PAYER.to_owned()).parse()?;
    let rpc = SolanaRpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let mut results = Vec::new();
    for permissionless in [false, true] {
        results.push(verify_collection(&rpc, pool, payer, permissionless).await?);
    }
    if std::env::var("CPMM_REQUIRE_TRANSFER_FEE").as_deref() == Ok("1") {
        ensure!(
            results.iter().all(|r| r["checks"].as_array().is_some_and(|checks| checks.iter().any(
                |c| c["transfer_fee"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|fee| fee > 0)
            ))),
            "Expected a nonzero Token-2022 transfer fee in both collection modes"
        );
    }
    let report = json!({"kind":"mainnet_unsigned_simulation","broadcast":false,"signature_verification":false,"results":results});
    if let Ok(path) = std::env::var("CPMM_REPORT_PATH") {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn captured_token_2022_mint_transfer_fee_rounds_up_and_caps() {
        let report: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/cpmm_creator_fee_default_mainnet.json"
        ))
        .unwrap();
        let mint = &report["results"][0]["before_accounts"][8];
        let account = Account {
            owner: mint["owner"].as_str().unwrap().parse().unwrap(),
            data: STANDARD.decode(mint["data_base64"].as_str().unwrap()).unwrap(),
            ..Default::default()
        };
        // Captured mint's older/newer schedules both use 300 bps and a 10^15 cap.
        for epoch in [1029, 1048, u64::MAX] {
            assert_eq!(transfer_fee(&account, epoch, 0).unwrap(), 0);
            assert_eq!(transfer_fee(&account, epoch, 1).unwrap(), 1);
            assert_eq!(transfer_fee(&account, epoch, 333).unwrap(), 10);
            assert_eq!(transfer_fee(&account, epoch, 334).unwrap(), 11);
            assert_eq!(transfer_fee(&account, epoch, 10_000).unwrap(), 300);
            assert_eq!(transfer_fee(&account, epoch, u64::MAX).unwrap(), 1_000_000_000_000_000);
        }
    }
}
