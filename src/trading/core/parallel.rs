use anyhow::{anyhow, Result};
use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
};
use std::{str::FromStr, sync::Arc, time::Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{
    common::nonce_cache::DurableNonceInfo,
    common::{GasFeeStrategy, SolanaRpcClient},
    swqos::{SwqosClient, SwqosType, TradeType},
    trading::{common::build_transaction, MiddlewareManager, SwapParams},
};

pub async fn buy_parallel_execute(
    params: SwapParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_clients,
        params.payer,
        params.rpc,
        instructions,
        params.lookup_table_key,
        params.recent_blockhash,
        params.durable_nonce,
        params.data_size_limit,
        params.middleware_manager,
        protocol_name,
        true,
        params.wait_transaction_confirmed,
        true,
    )
    .await
}

pub async fn sell_parallel_execute(
    params: SwapParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_clients,
        params.payer,
        params.rpc,
        instructions,
        params.lookup_table_key,
        params.recent_blockhash,
        params.durable_nonce,
        0,
        params.middleware_manager,
        protocol_name,
        false,
        params.wait_transaction_confirmed,
        params.with_tip,
    )
    .await
}

/// Generic function for parallel transaction execution
async fn parallel_execute(
    swqos_clients: Vec<Arc<SwqosClient>>,
    payer: Arc<Keypair>,
    rpc: Option<Arc<SolanaRpcClient>>,
    instructions: Vec<Instruction>,
    lookup_table_key: Option<Pubkey>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
) -> Result<Signature> {
    if swqos_clients.is_empty() {
        return Err(anyhow!("swqos_clients is empty"));
    }
    if !with_tip
        && swqos_clients
            .iter()
            .find(|swqos| matches!(swqos.get_swqos_type(), SwqosType::Default))
            .is_none()
    {
        return Err(anyhow!("No Rpc Default Swqos configured."));
    }
    let cores = core_affinity::get_core_ids().unwrap();
    let mut handles: Vec<JoinHandle<Result<Signature>>> = Vec::with_capacity(swqos_clients.len());

    let instructions = Arc::new(instructions);

    // 预先计算所有有效的组合
    let task_configs: Vec<_> = swqos_clients
        .iter()
        .enumerate()
        .filter(|(_, swqos_client)| {
            with_tip || matches!(swqos_client.get_swqos_type(), SwqosType::Default)
        })
        .flat_map(|(i, swqos_client)| {
            let gas_fee_strategy_configs = GasFeeStrategy::get_strategies(if is_buy {
                TradeType::Buy
            } else {
                TradeType::Sell
            });
            gas_fee_strategy_configs
                .into_iter()
                .filter(|config| config.0.eq(&swqos_client.get_swqos_type()))
                .map(move |config| (i, swqos_client.clone(), config))
        })
        .collect();

    if task_configs.is_empty() {
        return Err(anyhow!("No available gas fee strategy configs. Please configure GasFeeStrategy for specific SwqosType."));
    }

    for (i, swqos_client, gas_fee_strategy_config) in task_configs {
        let core_id = cores[i % cores.len()];
        let payer = payer.clone();
        let instructions = instructions.clone();
        let middleware_manager = middleware_manager.clone();
        let swqos_type = swqos_client.get_swqos_type();
        let tip_account_str = swqos_client.get_tip_account()?;
        let tip_account = Arc::new(Pubkey::from_str(&tip_account_str).unwrap_or_default());

        let tip = gas_fee_strategy_config.2.tip;
        let unit_limit = gas_fee_strategy_config.2.cu_limit;
        let unit_price = gas_fee_strategy_config.2.cu_price;
        let swqos_type = swqos_type.clone();
        let tip_account = tip_account.clone();
        let rpc = rpc.clone();
        let durable_nonce = durable_nonce.clone();

        let handle = tokio::spawn(async move {
            core_affinity::set_for_current(core_id);

            let mut start = Instant::now();

            let tip_amount = if with_tip { tip } else { 0.0 };

            let transaction = build_transaction(
                payer,
                rpc,
                unit_limit,
                unit_price,
                instructions.as_ref().clone(),
                lookup_table_key,
                recent_blockhash,
                data_size_limit,
                middleware_manager,
                protocol_name,
                is_buy,
                swqos_type != SwqosType::Default,
                &tip_account,
                tip_amount,
                durable_nonce,
                // current_nonce,
            )
            .await?;

            println!(
                "[{:?}] - [{:?}] - Building transaction instructions: {:?}",
                swqos_type,
                gas_fee_strategy_config.1,
                start.elapsed()
            );

            start = Instant::now();

            swqos_client
                .send_transaction(
                    if is_buy { TradeType::Buy } else { TradeType::Sell },
                    &transaction,
                )
                .await?;

            println!(
                "[{:?}] - [{:?}] - Submitting transaction instructions: {:?}",
                swqos_type,
                gas_fee_strategy_config.1,
                start.elapsed()
            );

            transaction
                .signatures
                .first()
                .ok_or_else(|| anyhow!("Transaction has no signatures"))
                .cloned()
        });

        handles.push(handle);
    }
    // Return as soon as any one succeeds
    let (tx, mut rx) = mpsc::channel(handles.len());

    // Start monitoring tasks
    for handle in handles {
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = handle.await;
            let _ = tx.send(result).await;
        });
    }
    drop(tx); // Close the sender

    // Wait for the first successful result
    let mut errors = Vec::new();

    if !wait_transaction_confirmed {
        if let Some(result) = rx.recv().await {
            match result {
                Ok(Ok(sig)) => return Ok(sig),
                Ok(Err(e)) => errors.push(format!("Task error: {}", e)),
                Err(e) => errors.push(format!("Join error: {}", e)),
            }
        }
        return Err(anyhow!("No transaction signature available"));
    }

    while let Some(result) = rx.recv().await {
        match result {
            Ok(Ok(sig)) => {
                return Ok(sig);
            }
            Ok(Err(e)) => errors.push(format!("Task error: {}", e)),
            Err(e) => errors.push(format!("Join error: {}", e)),
        }
    }

    // If no success, return error
    return Err(anyhow!("All transactions failed: {:?}", errors));
}
