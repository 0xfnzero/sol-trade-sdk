//! Simulate both CPMM creator-fee collection instructions and verify post-state.
//! No private key or transaction broadcast. See docs/cpmm-creator-fee-share.md.
#[path = "support/cpmm_creator_fee_simulation.rs"]
mod simulation;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let report = simulation::run_from_env().await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
