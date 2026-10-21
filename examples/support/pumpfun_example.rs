//! SOL/WSOL/USDC -> stock quote -> Pump.fun bonding curve, and independent sells.
//! Default: synthetic offline construction. --live reads current accounts first.
//! Shared construction and simulation harness. Never broadcasts transactions.
use anyhow::{anyhow, ensure, Context, Result};
use sol_trade_sdk::{
    common::{
        bonding_curve::BondingCurveAccount, GasFeeStrategy, SolanaRpcClient,
        TradeTransactionVersion,
    },
    constants::{
        SOL_TOKEN_ACCOUNT, TOKEN_PROGRAM, TOKEN_PROGRAM_2022, USDC_TOKEN_ACCOUNT,
        WSOL_TOKEN_ACCOUNT,
    },
    instruction::{
        pumpfun::PumpFunInstructionBuilder,
        utils::pumpfun::{get_bonding_curve_pda, get_creator_vault_pda},
    },
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, PumpFunParams, RaydiumCpmmParams, SwapParams, TokenTransferFee},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_cpmm::compute_swap_amount_for_pool,
    PumpFunQuoteHop, PumpFunQuoteRoute, PumpFunQuoteVenue, PumpFunV3Params, PumpFunVersion,
};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use std::sync::Arc;

async fn simulate_instructions(
    params: &SwapParams,
    instructions: Vec<solana_sdk::instruction::Instruction>,
    version: PumpFunVersion,
    direct: bool,
    report: Option<&str>,
) -> Result<()> {
    use solana_commitment_config::CommitmentConfig;
    use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
    use solana_transaction_status_client_types::UiTransactionEncoding;
    let rpc = SolanaRpcClient::new_with_commitment(
        std::env::var("SOLANA_RPC_URL")?,
        CommitmentConfig::confirmed(),
    );
    let owner = params.payer.pubkey();
    ensure!(rpc.get_balance(&owner).await? > 0, "Simulation payer needs real SOL for fees/rent");
    if params.input_mint != SOL_TOKEN_ACCOUNT {
        let program = rpc.get_account(&params.input_mint).await?.owner;
        let ata = sol_trade_sdk::common::spl_associated_token_account::get_associated_token_address_with_program_id(&owner, &params.input_mint, &program);
        let balance = rpc.get_token_account_balance(&ata).await.context(
            "Simulation input ATA missing; fund it before simulating a sell/token-funded buy",
        )?;
        ensure!(
            balance.amount.parse::<u64>()? >= params.input_amount.unwrap_or(0),
            "Insufficient input tokens for independent simulation"
        );
    }
    let hash = rpc.get_latest_blockhash().await?;
    let config = solana_message::v1::TransactionConfig::empty()
        .with_compute_unit_limit(1_400_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let message = solana_message::VersionedMessage::V1(
        solana_message::v1::Message::try_compile_with_config(&owner, &instructions, hash, config)?,
    );
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); message.header().num_required_signatures as usize],
        message,
    };
    let wire = wincode::serialize(&tx)?;
    let bytes = wire.len();
    ensure!(
        bytes <= solana_message::v1::MAX_TRANSACTION_SIZE,
        "Simulation transaction exceeds V1 message size limit"
    );
    let response = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: false,
                commitment: Some(CommitmentConfig::confirmed()),
                encoding: Some(UiTransactionEncoding::Base64),
                inner_instructions: true,
                ..Default::default()
            },
        )
        .await?;
    use base64::Engine;
    let result = serde_json::json!({"simulation_only":true,"signed":false,"broadcast":false,
        "wire":base64::engine::general_purpose::STANDARD.encode(wire),
        "pumpfun_version":format!("{version:?}"),"route":if direct {"direct"} else {"stock"},
        "side":format!("{:?}", params.trade_type),"wallet":owner.to_string(),
        "input_mint":params.input_mint.to_string(),"output_mint":params.output_mint.to_string(),
        "input_amount":params.input_amount,"wire_bytes":bytes,"solana_message_version":"V1",
        "instructions":instructions.iter().map(|ix| serde_json::json!({"program_id":ix.program_id.to_string(),
            "accounts":ix.accounts.iter().map(|a| a.pubkey.to_string()).collect::<Vec<_>>(),"data":ix.data})).collect::<Vec<_>>(),
        "response":response});
    finish_report(&result, report)?;
    println!(
        "Simulation succeeded: slot={}, CU={:?}. Nothing signed or broadcast.",
        response.context.slot, response.value.units_consumed
    );
    Ok(())
}

fn finish_report(result: &serde_json::Value, report: Option<&str>) -> Result<()> {
    if let Some(path) = report {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    let error =
        result.pointer("/response/value/err").context("Missing explicit simulation result")?;
    ensure!(error.is_null(), "Simulation failed; inspect response.value.err and logs above");
    ensure!(
        result.pointer("/response/context/slot").and_then(serde_json::Value::as_u64).is_some(),
        "Missing simulation slot"
    );
    Ok(())
}

pub async fn run(args: Vec<String>, simulate: bool) -> Result<()> {
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "Pump.fun INNER curve stock route (not PumpSwap/LaunchLab).
cargo run --example pumpfun_stock_route -- [--sell] [--asset SOL|WSOL|USDC] [--version v2|v3]
  --version defaults to v3; partial fills are disabled for V3.
Simulation: cargo run --example pumpfun_simulate -- --version v1|v2|v3
  --mint MINT --amount RAW --keypair PATH [--sell] [--pool CPMM_POOL]
  No pool: direct curve trade; with pool: stock multi-hop (V2/V3 only).
  --curve-min-out RAW is required for stock routes and V3 direct trades.
  V1/V2 native direct trades derive the minimum from --slippage-bps.
  --report NEW_PATH saves logs, CU and result; never overwrites.
  Requires SOLANA_RPC_URL, a funded wallet, and input tokens for a sell.
  Keypair is used for account derivation only; no signing or broadcasting.
  Default: synthetic offline accounts; demonstrates construction, not execution.
  --live --mint MINT --pool CPMM_POOL --amount RAW --curve-min-out RAW
  --live requires SOLANA_RPC_URL. The CPMM must pair the funding asset with
  the curve's actual quote mint. --curve-min-out is a fresh caller quote:
  target-token units on buys, stock-quote units on sells. Never copy old fills.
  --slippage-bps BPS sets the conversion hop's tolerance (default 100).
The stock-route entry only constructs; the simulate entry calls simulateTransaction
after construction. Default stock-route accounts are synthetic."
        );
        return Ok(());
    }
    let mut i = 0;
    let mut seen = std::collections::HashSet::new();
    while i < args.len() {
        ensure!(seen.insert(args[i].as_str()), "Duplicate argument {}", args[i]);
        match args[i].as_str() {
            "--live" | "--sell" => i += 1,
            "--mint" | "--pool" | "--amount" | "--curve-min-out" | "--asset" | "--slippage-bps"
            | "--version" | "--keypair" | "--report" => {
                ensure!(
                    args.get(i + 1).is_some_and(|v| !v.starts_with("--")),
                    "Missing value for {}",
                    args[i]
                );
                i += 2;
            }
            value => return Err(anyhow!("Unknown argument {value}; use --help")),
        }
    }
    let value = |flag: &str| {
        args.iter().position(|arg| arg == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    };
    let live = simulate || args.iter().any(|arg| arg == "--live");
    let version = match value("--version").unwrap_or("v3") {
        "v3" => PumpFunVersion::V3,
        "v2" => PumpFunVersion::V2,
        "v1" | "legacy" => PumpFunVersion::Legacy,
        other => return Err(anyhow!("Unsupported version {other}; use v1, v2 or v3")),
    };
    let v3 = version == PumpFunVersion::V3;
    let direct = simulate && value("--pool").is_none();
    ensure!(
        !direct || value("--asset").is_none(),
        "Direct trades use the curve's quote; --asset is for stock routes"
    );
    ensure!(direct || version != PumpFunVersion::Legacy, "V1 does not support stock quote routes");
    ensure!(
        simulate || (value("--keypair").is_none() && value("--report").is_none()),
        "keypair/report require pumpfun_simulate"
    );
    let payer = if simulate {
        Arc::new(
            read_keypair_file(value("--keypair").context("Simulation requires --keypair PATH")?)
                .map_err(|e| anyhow!("Cannot read simulation keypair: {e}"))?,
        )
    } else {
        Arc::new(Keypair::new())
    };
    if let Some(path) = value("--report") {
        ensure!(!std::path::Path::new(path).exists(), "Report exists; choose a new path");
    }
    let buy = !args.iter().any(|arg| arg == "--sell");
    let asset = match value("--asset").unwrap_or("SOL") {
        "SOL" => SOL_TOKEN_ACCOUNT,
        "WSOL" => WSOL_TOKEN_ACCOUNT,
        "USDC" => USDC_TOKEN_ACCOUNT,
        other => return Err(anyhow!("Unsupported asset {other}")),
    };
    let funding = if asset == SOL_TOKEN_ACCOUNT { WSOL_TOKEN_ACCOUNT } else { asset };
    let amount: u64 = value("--amount").unwrap_or("1000000").parse()?;
    let curve_min: u64 = value("--curve-min-out").unwrap_or("500000").parse()?;
    let slippage: u64 = value("--slippage-bps").unwrap_or("100").parse()?;
    ensure!(
        amount > 0 && curve_min > 0 && slippage < 10000,
        "Amounts must be nonzero; slippage must be below 10000 bps"
    );

    let (mint, curve, pool, quote_program, base_fee, quote_fee) = if live {
        ensure!(
            value("--amount").is_some() && (direct && !v3 || value("--curve-min-out").is_some()),
            "Live construction requires explicit amount; V3 and stock routes also require curve-min-out"
        );
        let mint: Pubkey = value("--mint").context("--live requires --mint")?.parse()?;
        let rpc =
            SolanaRpcClient::new(std::env::var("SOLANA_RPC_URL").context("Set SOLANA_RPC_URL")?);
        // Cold bootstrap only. A production subscriber supplies these snapshots.
        let curve = PumpFunParams::from_mint_by_rpc(&rpc, &mint).await?;
        let pool = if direct {
            None
        } else {
            Some(
                RaydiumCpmmParams::from_pool_address_by_rpc(
                    &rpc,
                    &value("--pool").context("Stock route requires --pool")?.parse()?,
                )
                .await?,
            )
        };
        let (_, base_fee) = TokenTransferFee::from_mint_by_rpc(&rpc, &mint).await?;
        let (owner, quote_fee) =
            TokenTransferFee::from_mint_by_rpc(&rpc, &curve.bonding_curve.effective_quote_mint())
                .await?;
        (mint, curve, pool, owner, base_fee, quote_fee)
    } else {
        ensure!(
            value("--mint").is_none() && value("--pool").is_none(),
            "Use --live to load real mint/pool accounts"
        );
        eprintln!("SYNTHETIC offline construction: these pool/curve addresses are not deployed. No mainnet execution claim.");
        let mint = Pubkey::new_unique();
        let stock = Pubkey::new_unique();
        let creator = Pubkey::new_unique();
        let curve = PumpFunParams {
            bonding_curve: Arc::new(BondingCurveAccount {
                account: get_bonding_curve_pda(&mint).context("curve PDA")?,
                quote_mint: stock,
                creator,
                virtual_token_reserves: 1_000_000_000_000,
                virtual_sol_reserves: 1_000_000_000,
                real_token_reserves: 800_000_000_000,
                ..Default::default()
            }),
            associated_bonding_curve: Pubkey::default(),
            observed_trade_creator: Some(creator),
            creator_vault: get_creator_vault_pda(&creator).context("creator PDA")?,
            fee_sharing_creator_vault_if_active: None,
            token_program: TOKEN_PROGRAM_2022,
            close_token_account_when_sell: Some(false),
            fee_recipient:
                sol_trade_sdk::instruction::utils::pumpfun::global_constants::FEE_RECIPIENT,
            quote_mint: stock,
        };
        let pool = RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            funding,
            stock,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            TOKEN_PROGRAM,
            TOKEN_PROGRAM_2022,
            Pubkey::new_unique(),
            1_000_000_000_000,
            1_000_000_000_000,
        );
        (
            mint,
            curve,
            Some(pool),
            TOKEN_PROGRAM_2022,
            TokenTransferFee::default(),
            TokenTransferFee::default(),
        )
    };
    ensure!(!curve.bonding_curve.complete, "Curve has graduated; supply a current active mint");
    let stock = curve.bonding_curve.effective_quote_mint();
    let (protocol_params, input, output, preview) = if direct {
        ensure!(
            value("--asset").is_none(),
            "Direct trades use the curve's quote; --asset is for stock routes"
        );
        ensure!(
            base_fee.basis_points == 0 && quote_fee.basis_points == 0,
            "Direct simulation currently requires zero transfer fees"
        );
        if version == PumpFunVersion::Legacy {
            ensure!(stock == WSOL_TOKEN_ACCOUNT, "V1 direct trades require a native SOL curve");
        }
        if !v3 && stock == WSOL_TOKEN_ACCOUNT {
            ensure!(
                value("--curve-min-out").is_none(),
                "V1/V2 native direct limits come from slippage-bps; curve-min-out would be ignored"
            );
        }
        let quote = if stock == WSOL_TOKEN_ACCOUNT { SOL_TOKEN_ACCOUNT } else { stock };
        let protocol = if version == PumpFunVersion::V2 && stock != WSOL_TOKEN_ACCOUNT {
            ensure!(
                value("--curve-min-out").is_some(),
                "V2 token-quote direct trades require curve-min-out"
            );
            let hop = PumpFunQuoteHop::exact_in(
                PumpFunQuoteVenue::PumpFun {
                    mint,
                    curve,
                    quote_token_program: quote_program,
                    base_transfer_fee: base_fee,
                    quote_transfer_fee: quote_fee,
                },
                if buy { quote } else { mint },
                if buy { mint } else { quote },
                curve_min,
            );
            DexParamEnum::PumpFunQuoteRoute(if buy {
                PumpFunQuoteRoute::buy(vec![hop])
            } else {
                PumpFunQuoteRoute::sell(vec![hop])
            })
        } else if v3 {
            DexParamEnum::PumpFunV3(PumpFunV3Params {
                curve,
                quote_token_program: quote_program,
                minimum_output_amount: curve_min,
                base_transfer_fee: base_fee,
                quote_transfer_fee: quote_fee,
            })
        } else {
            DexParamEnum::PumpFunVersioned { params: curve, version }
        };
        (protocol, if buy { quote } else { mint }, if buy { mint } else { quote }, None)
    } else {
        let pool = pool.context("Stock route requires a conversion pool")?;
        ensure!(
            (pool.base_mint == funding && pool.quote_mint == stock)
                || (pool.quote_mint == funding && pool.base_mint == stock),
            "CPMM must connect the requested funding mint and curve quote mint"
        );
        let conversion_input = if buy { amount } else { curve_min };
        let input_mint = if buy { funding } else { stock };
        let conversion_min = compute_swap_amount_for_pool(
            &pool,
            input_mint == pool.base_mint,
            conversion_input,
            slippage,
        )?
        .min_amount_out;
        let curve_venue = if v3 {
            PumpFunQuoteVenue::PumpFunV3 {
                mint,
                params: PumpFunV3Params {
                    curve,
                    quote_token_program: quote_program,
                    minimum_output_amount: curve_min,
                    base_transfer_fee: base_fee,
                    quote_transfer_fee: quote_fee,
                },
            }
        } else {
            PumpFunQuoteVenue::PumpFun {
                mint,
                curve,
                quote_token_program: quote_program,
                base_transfer_fee: base_fee,
                quote_transfer_fee: quote_fee,
            }
        };
        let bridge_venue = PumpFunQuoteVenue::RaydiumCpmm(pool);
        // Spend guaranteed output only. Surplus stock tokens remain in the wallet.
        let route = if buy {
            PumpFunQuoteRoute::buy(vec![
                PumpFunQuoteHop::exact_in(bridge_venue, asset, stock, conversion_min),
                PumpFunQuoteHop::exact_in(curve_venue, stock, mint, curve_min),
            ])
        } else {
            PumpFunQuoteRoute::sell(vec![
                PumpFunQuoteHop::exact_in(curve_venue, mint, stock, curve_min),
                PumpFunQuoteHop::exact_in(bridge_venue, stock, asset, conversion_min),
            ])
        };
        let input = if buy { asset } else { mint };
        let output = if buy { mint } else { asset };
        let preview = route.preview(input, output, amount, slippage, buy)?;
        (DexParamEnum::PumpFunQuoteRoute(route), input, output, Some(preview))
    };
    let params = SwapParams {
        rpc: None,
        payer,
        trade_type: if buy { TradeType::Buy } else { TradeType::Sell },
        input_mint: input,
        output_mint: output,
        input_token_program: None,
        output_token_program: None,
        input_amount: Some(amount),
        slippage_basis_points: Some(slippage),
        protocol_params,
        address_lookup_table_accounts: vec![],
        recent_blockhash: None,
        wait_tx_confirmed: false,
        open_seed_optimize: false,
        swqos_clients: Arc::new(vec![]),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: !buy && asset == SOL_TOKEN_ACCOUNT,
        fixed_output_amount: None,
        gas_fee_strategy: GasFeeStrategy::new(),
        simulate: false,
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 0,
        effective_core_ids: Arc::new(vec![]),
        check_min_tip: false,
        transaction_version: TradeTransactionVersion::V1,
        grpc_recv_us: None,
        use_exact_sol_amount: Some(true),
    };
    let instructions = if buy {
        PumpFunInstructionBuilder.build_buy_instructions(&params).await?
    } else {
        PumpFunInstructionBuilder.build_sell_instructions(&params).await?
    };
    use sol_trade_sdk::instruction::utils::pumpfun::{
        accounts, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR,
        SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
    };
    let curve_ix = instructions
        .iter()
        .find(|ix| ix.program_id == accounts::PUMPFUN)
        .context("Missing Pump.fun instruction")?;
    let expected = match (version, buy) {
        (PumpFunVersion::Legacy, true) => BUY_EXACT_SOL_IN_DISCRIMINATOR,
        (PumpFunVersion::Legacy, false) => SELL_DISCRIMINATOR,
        (PumpFunVersion::V2, true) => BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
        (PumpFunVersion::V2, false) => SELL_V2_DISCRIMINATOR,
        (PumpFunVersion::V3, true) => [225, 247, 80, 30, 213, 179, 132, 136],
        (PumpFunVersion::V3, false) => [28, 146, 222, 119, 38, 196, 105, 213],
    };
    ensure!(
        curve_ix.data.get(..8) == Some(expected.as_slice()),
        "Constructed instruction does not match requested version/direction"
    );
    println!(
        "Pump.fun {:?} {}: input_raw={}, minimum_output_raw={}",
        version,
        if buy { "buy" } else { "sell" },
        u64::from_le_bytes(curve_ix.data[8..16].try_into()?),
        u64::from_le_bytes(curve_ix.data[16..24].try_into()?)
    );
    if let Some(preview) = &preview {
        for (index, hop) in preview.hops.iter().enumerate() {
            println!(
                "hop {}: {} -> {}, input={}, minimum_output={}",
                index + 1,
                hop.input_mint,
                hop.output_mint,
                hop.input_amount,
                hop.minimum_amount_out
            );
        }
        println!(
        "{} instructions; minimum final credit={}; rpc=None during build; nothing signed or sent",
        instructions.len(),
        preview.minimum_output_credit
    );
    }
    if simulate {
        simulate_instructions(&params, instructions, version, direct, value("--report")).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_failures_preserve_evidence_and_cannot_report_success() {
        let path = std::env::temp_dir().join(format!("pump-sim-{}.json", Pubkey::new_unique()));
        let failure = serde_json::json!({"response":{"context":{"slot":123},"value":{
            "err":{"InstructionError":[0,{"Custom":6002}]},"logs":["slippage failure"],"unitsConsumed":1000}}});
        assert!(finish_report(&failure, path.to_str()).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            failure
        );
        let success = serde_json::json!({"response":{"context":{"slot":124},"value":{"err":null}}});
        assert!(finish_report(&success, path.to_str()).is_err()); // Never overwrite failed evidence.
        assert!(finish_report(&success, None).is_ok());
        assert!(finish_report(&serde_json::json!({}), None).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn offline_routes_and_cli_guardrails() {
        for version in ["v2", "v3"] {
            for asset in ["SOL", "WSOL", "USDC"] {
                for sell in [false, true] {
                    let mut args =
                        vec!["--version".into(), version.into(), "--asset".into(), asset.into()];
                    if sell {
                        args.push("--sell".into());
                    }
                    run(args, false).await.unwrap();
                }
            }
        }
        for args in [
            vec!["--version", "v1", "--pool", "unused"],
            vec!["--version", "v3", "--version", "v2"],
            vec!["--amount"],
            vec!["--unknown"],
            vec![],
        ] {
            assert!(run(args.into_iter().map(str::to_owned).collect(), true).await.is_err());
        }
    }
}
