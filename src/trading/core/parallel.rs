use anyhow::{anyhow, Result};
use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
};
use std::{str::FromStr, sync::Arc, time::Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{
    swqos::{settings::SwqosSettings, SwqosType, TradeType},
    trading::{
        common::build_transaction, InternalBuyParams, InternalSellParams, MiddlewareManager,
    },
};

pub async fn buy_parallel_execute(
    params: InternalBuyParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_settings,
        params.payer,
        instructions,
        params.lookup_table_key,
        params.recent_blockhash,
        params.data_size_limit,
        params.middleware_manager,
        protocol_name,
        true,
        params.wait_transaction_confirmed,
        true,
        params.custom_cu_limit,
    )
    .await
}

pub async fn sell_parallel_execute(
    params: InternalSellParams,
    instructions: Vec<Instruction>,
    protocol_name: &'static str,
) -> Result<Signature> {
    parallel_execute(
        params.swqos_settings,
        params.payer,
        instructions,
        params.lookup_table_key,
        params.recent_blockhash,
        0,
        params.middleware_manager,
        protocol_name,
        false,
        params.wait_transaction_confirmed,
        params.with_tip,
        params.custom_cu_limit,
    )
    .await
}

/// Generic function for parallel transaction execution
async fn parallel_execute(
    swqos_settings: Vec<Arc<SwqosSettings>>,
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    lookup_table_key: Option<Pubkey>,
    recent_blockhash: Hash,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
    custom_cu_limit: Option<u32>,
) -> Result<Signature> {
    if swqos_settings.is_empty() {
        return Err(anyhow!("swqos_settings is empty"));
    }
    if !with_tip
        && swqos_settings
            .iter()
            .find(|swqos| {
                matches!(swqos.swqos_client.as_ref().unwrap().get_swqos_type(), SwqosType::Default)
            })
            .is_none()
    {
        return Err(anyhow!("No Rpc Default Swqos configured"));
    }
    let cores = core_affinity::get_core_ids().unwrap();
    let mut handles: Vec<JoinHandle<Result<Signature>>> = Vec::with_capacity(swqos_settings.len());

    let instructions = Arc::new(instructions);

    for i in 0..swqos_settings.len() {
        if let Some(swqos_client) = swqos_settings[i].swqos_client.as_ref() {
            if !with_tip && !matches!(swqos_client.get_swqos_type(), SwqosType::Default) {
                continue;
            }
            let payer = payer.clone();
            let instructions = instructions.clone();
            let core_id = cores[i % cores.len()];

            let middleware_manager = middleware_manager.clone();
            let swqos_client = swqos_client.clone();
            let buy_tip_fee = swqos_settings[i].buy_tip_fee;
            let sell_tip_fee = swqos_settings[i].sell_tip_fee;
            let mut unit_limit = swqos_settings[i].unit_limit;
            let unit_price = swqos_settings[i].unit_price;

            if let Some(custom_cu_limit) = custom_cu_limit {
                unit_limit = custom_cu_limit;
            }

            let handle = tokio::spawn(async move {
                core_affinity::set_for_current(core_id);

                let swqos_type = swqos_client.get_swqos_type();
                let mut start = Instant::now();

                let tip_account_str = swqos_client.get_tip_account()?;
                let tip_account = Arc::new(Pubkey::from_str(&tip_account_str).unwrap_or_default());

                let tip_amount = if with_tip {
                    if is_buy {
                        buy_tip_fee
                    } else {
                        sell_tip_fee
                    }
                } else {
                    0.0
                };

                let transaction = build_transaction(
                    payer,
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
