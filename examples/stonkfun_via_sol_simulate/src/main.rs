//! StonkFun ViaSol mainnet simulation example.
//!
//! Creates an ephemeral wallet, loads real mainnet pools, virtually funds the
//! wallet inside `simulateTransaction` (`sigVerify=false`), and dry-runs the
//! `SOL ↔ quote ↔ meme` route. Nothing is submitted on-chain.
//!
//! ```bash
//! export RPC_URL=https://mainnet.helius-rpc.com/?api-key=...
//! cargo run -p stonkfun_via_sol_simulate
//! cargo run -p stonkfun_via_sol_simulate -- --curve
//! ```

use sol_trade_sdk::{
    common::{address_lookup::fetch_address_lookup_table_account, SolanaRpcClient},
    instruction::stonkfun::StonkFunInstructionBuilder,
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, RaydiumCpmmParams, StonkFunParams, StonkFunViaSolParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::{v0, AddressLookupTableAccount, VersionedMessage};
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::sync::Arc;

const CURVE_POOL: Pubkey = pubkey!("84XZdJNyBVVBqGe3BHY8n6x1jbcnxNWA5x4GetwQsjgp");
const CURVE_MEME: Pubkey = pubkey!("BJ56gcrMNKDzVwjQXKToya9cAcMZvN9pz6ZzUejxQary");
const WSOL_CARDS_CPMM: Pubkey = pubkey!("3kMBV4dFBLoAaNFBcXcnaY2sY6k4k45Pcuo2zXSJHXQx");
const WSOL_CARDS_LUT: Pubkey = pubkey!("2gyDuEaj39reuVjrQxGtFsK7625MbA9qDzNmHZUibrN2");

const GRAD_POOL: Pubkey = pubkey!("BUVzsLLLG7GWoyJVoU31pXiBveazA6GXTavZ9VD3CwS9");
const GRAD_MEME_KNOTS: Pubkey = pubkey!("8RVBk8vxLiUHueLUW1f4izFVqN3nWippLhkohKg6EGkS");
const WSOL_STONK_CPMM: Pubkey = pubkey!("EKPjNvowpSFPaZcroeUcgAPtdUTZZrP3v8sCKKyfpe5x");
const WSOL_STONK_LUT: Pubkey = pubkey!("8vp6JD2W19rM6Vbs3cBRCuQ3nXc8aykRMRrMEA6zoigR");

const SIM_FUNDER_CANDIDATES: [Pubkey; 4] = [
    pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"),
    pubkey!("5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9"),
    pubkey!("2ojv9BAiHUrvsm9gxDeBFNCuoK1xKdcHGa5Y6XD7Tcuv"),
    pubkey!("FWznbcNXWQuHTawe9RxvQ2LdCENssh12dsznf4RiouN5"),
];
const SIM_FUND_LAMPORTS: u64 = 100_000_000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let curve_mode = std::env::args().any(|arg| arg == "--curve");
    let rpc_url = std::env::var("RPC_URL")
        .or_else(|_| std::env::var("SOLANA_RPC_URL"))
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let rpc = SolanaRpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());

    let wallet = Arc::new(Keypair::new());
    println!("created ephemeral wallet={}", wallet.pubkey());

    let funder = pick_funder(&rpc).await?;
    println!("simulation funder={funder}");

    let (meme, via, amount, slip, lut) = if curve_mode {
        let curve = StonkFunParams::from_pool_by_rpc(&rpc, &CURVE_POOL).await?;
        let sol_hop = RaydiumCpmmParams::from_pool_address_by_rpc(&rpc, &WSOL_CARDS_CPMM).await?;
        println!("mode=curve pool={CURVE_POOL} quote={}", curve.quote_mint);
        (
            CURVE_MEME,
            StonkFunViaSolParams::curve_with_cpmm(curve, sol_hop),
            10_000u64,
            500u64,
            WSOL_CARDS_LUT,
        )
    } else {
        let graduated = RaydiumCpmmParams::from_pool_address_by_rpc(&rpc, &GRAD_POOL).await?;
        let sol_hop = RaydiumCpmmParams::from_pool_address_by_rpc(&rpc, &WSOL_STONK_CPMM).await?;
        println!(
            "mode=graduated pool={GRAD_POOL} base={} quote={}",
            graduated.base_mint, graduated.quote_mint
        );
        (
            GRAD_MEME_KNOTS,
            StonkFunViaSolParams::graduated_with_cpmm(graduated, sol_hop),
            50_000u64,
            300u64,
            WSOL_STONK_LUT,
        )
    };

    let params = SwapParams {
        rpc: None,
        payer: wallet.clone(),
        trade_type: TradeType::Buy,
        input_mint: sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT,
        input_token_program: None,
        output_mint: meme,
        output_token_program: None,
        input_amount: Some(amount),
        slippage_basis_points: Some(slip),
        address_lookup_table_accounts: Vec::new(),
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params: DexParamEnum::StonkFunViaSol(via),
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
    };

    let business = StonkFunInstructionBuilder.build_buy_instructions(&params).await?;
    println!("business ix count={}", business.len());

    let alt = fetch_address_lookup_table_account(&rpc, &lut).await?;
    let blockhash = rpc.get_latest_blockhash().await?;
    let tx = build_unsigned_simulation_tx(funder, &wallet, business, blockhash, &[alt]);
    let result = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Processed }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot: None,
                inner_instructions: true,
            },
        )
        .await?;

    if let Some(err) = result.value.err {
        println!("simulate FAILED: {err:?}");
        if let Some(logs) = result.value.logs {
            for log in logs {
                println!("  {log}");
            }
        }
        std::process::exit(1);
    }

    println!("simulate OK cu={:?}", result.value.units_consumed);
    if let Some(logs) = result.value.logs {
        for log in logs.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            println!("  {log}");
        }
    }
    Ok(())
}

async fn pick_funder(rpc: &SolanaRpcClient) -> Result<Pubkey, Box<dyn std::error::Error>> {
    for candidate in SIM_FUNDER_CANDIDATES {
        let lamports = rpc.get_balance(&candidate).await?;
        if lamports >= SIM_FUND_LAMPORTS * 10 {
            return Ok(candidate);
        }
    }
    Err("no simulation funder with enough SOL".into())
}

fn build_unsigned_simulation_tx(
    funder: Pubkey,
    wallet: &Keypair,
    business_instructions: Vec<Instruction>,
    recent_blockhash: Hash,
    lookup_tables: &[AddressLookupTableAccount],
) -> VersionedTransaction {
    let mut instructions = Vec::with_capacity(business_instructions.len() + 3);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(600_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(100_000));
    instructions.push(system_instruction::transfer(
        &funder,
        &wallet.pubkey(),
        SIM_FUND_LAMPORTS,
    ));
    instructions.extend(business_instructions);

    let message = v0::Message::try_compile(&funder, &instructions, lookup_tables, recent_blockhash)
        .expect("compile simulation message");
    VersionedTransaction {
        signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
        message: VersionedMessage::V0(message),
    }
}
