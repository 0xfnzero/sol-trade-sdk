//! Offline initialization-once + repeated RPC-free buys using captured account data.
//! cargo run --example pumpswap_global_routing --locked
#[path = "support/pumpswap_routing_demo.rs"]
mod demo;
use anyhow::{ensure, Result};
use sol_trade_sdk::{
    constants::ASSOCIATED_TOKEN_PROGRAM_ID, instruction::pumpswap::PumpSwapInstructionBuilder,
    trading::core::traits::InstructionBuilder, TradeTokenType,
};
use solana_sdk::{hash::Hash, signature::Keypair};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let snapshot = demo::historical_snapshot()?;
    let (client, routing) = demo::configured_client(&snapshot, Arc::new(Keypair::new()))?;
    routing.publish(snapshot.cache.clone(), snapshot.context, snapshot.unix_timestamp);
    for asset in [TradeTokenType::SOL, TradeTokenType::WSOL] {
        for attempt in 1..=2 {
            let params = client.prepare_simple_buy(demo::buy_request(
                &snapshot,
                asset,
                Hash::new_unique(),
            ))?;
            ensure!(params.rpc.is_none(), "Hot path retained RPC");
            let ixs = PumpSwapInstructionBuilder.build_buy_instructions(&params).await?;
            let creates: Vec<_> =
                ixs.iter().filter(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID).collect();
            ensure!(
                creates.len() == 4 && creates.iter().all(|ix| ix.data == [1]),
                "Expected four idempotent ATAs"
            );
            println!(
                "{asset:?} buy {attempt}: {} instructions, {} idempotent ATA setups, rpc=None",
                ixs.len(),
                creates.len()
            );
        }
    }
    println!("Historical fixture only; no RPC, simulation or broadcast.");
    Ok(())
}
