use anyhow::{anyhow, Result};
use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
};
use std::{str::FromStr, sync::Arc, time::Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{
    common::PriorityFee,
    swqos::{SwqosClient, SwqosType, TradeType},
    trading::{common::build_transaction, BuyParams, MiddlewareManager, SellParams},
};

pub async fn buy_parallel_execute(
    params: BuyParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_clients,
        params.payer,
        instructions,
        params.priority_fee,
        params.lookup_table_key,
        params.recent_blockhash,
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
    params: SellParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_clients,
        params.payer,
        instructions,
        params.priority_fee,
        params.lookup_table_key,
        params.recent_blockhash,
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
    instructions: Vec<Instruction>,
    priority_fee: Arc<PriorityFee>,
    lookup_table_key: Option<Pubkey>,
    recent_blockhash: Hash,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
) -> Result<Signature> {
    let cores = core_affinity::get_core_ids().unwrap();
    let mut handles: Vec<JoinHandle<Result<Signature>>> = Vec::with_capacity(swqos_clients.len());
    if is_buy
        && (swqos_clients.len() > priority_fee.buy_tip_fees.len()
            || priority_fee.buy_tip_fees.is_empty())
    {
        return Err(anyhow!("Number of tip clients exceeds the configured buy tip fees. Please configure buy_tip_fees to match swqos_clients"));
    }
    if !is_buy
        && !with_tip
        && (swqos_clients.len() > priority_fee.sell_tip_fees.len()
            || priority_fee.sell_tip_fees.is_empty())
    {
        return Err(anyhow!("Number of tip clients exceeds the configured sell tip fees. Please configure sell_tip_fees to match swqos_clients"));
    }

    let instructions = Arc::new(instructions);

    for i in 0..swqos_clients.len() {
        let swqos_client = swqos_clients[i].clone();
        if !with_tip && !matches!(swqos_client.get_swqos_type(), SwqosType::Default) {
            continue;
        }
        let payer = payer.clone();
        let instructions = instructions.clone();
        let priority_fee = priority_fee.clone();
        let core_id = cores[i % cores.len()];

        let middleware_manager = middleware_manager.clone();

        let handle = tokio::spawn(async move {
            core_affinity::set_for_current(core_id);

            let swqos_type = swqos_client.get_swqos_type();
            let mut start = Instant::now();

            let tip_account_str = swqos_client.get_tip_account()?;
            let tip_account = Arc::new(Pubkey::from_str(&tip_account_str).unwrap_or_default());

            let tip_amount = if priority_fee.buy_tip_fees.len() > i {
                priority_fee.buy_tip_fees[i]
            } else {
                priority_fee.buy_tip_fees[0]
            };
            let tip_amount = if with_tip { tip_amount } else { 0.0 };

            let transaction = build_transaction(
                payer,
                &priority_fee,
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
            )
            .await?;

            println!(
                "[{:?}] - Building transaction instructions: {:?}",
                swqos_type,
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
                "[{:?}] - Submitting transaction instructions: {:?}",
                swqos_type,
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
    let (tx, mut rx) = mpsc::channel(swqos_clients.len());

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
