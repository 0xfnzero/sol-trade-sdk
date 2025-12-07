use clap::Parser;
use sol_trade_sdk::{
    common::{
        fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
        spl_associated_token_account::{
            create_associated_token_account_idempotent, get_associated_token_address,
        },
        spl_token::{self, close_account},
        AnyResult, TradeConfig,
    },
    constants::WSOL_TOKEN_ACCOUNT,
    swqos::SwqosConfig,
    trading::{
        core::params::{
            BonkParams, PumpFunParams, PumpSwapParams, RaydiumAmmV4Params, RaydiumCpmmParams, DexParamEnum,
        },
        factory::DexType,
    },
    SolanaTrade, TradeBuyParams, TradeSellParams, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    message::{AccountMeta, Instruction},
    native_token::sol_str_to_lamports,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use solana_system_interface::instruction::transfer;
use std::sync::{Arc, LazyLock};
use std::{
    io::{self, Write},
    str::FromStr,
};
// 设置 payer
static PAYER: LazyLock<Keypair> = LazyLock::new(|| Keypair::new());
// 设置 rpc url
static RPC_URL: &str = "https://api.mainnet-beta.solana.com";

static DEXS: &[&str] = &["pumpfun", "pumpswap", "bonk", "raydium_v4", "raydium_cpmm"];

#[derive(Parser)]
#[command(name = "sol-trade-cli")]
#[command(about = "SOL Trade CLI - A command line interface for trading tokens on Solana")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Parser)]
enum Command {
    /// Buy tokens with SOL
    Buy {
        /// Token mint address
        mint: String,
        /// DEX to use (pumpfun, pumpswap, bonk, raydium_v4, raydium_cpmm)
        dex: String,
        /// Amount of SOL to spend
        #[arg(short, long)]
        amount: f64,
        /// Slippage tolerance (optional)
        #[arg(short, long)]
        slippage: Option<u64>,
        /// AMM address for Raydium V4 (required for raydium_v4)
        #[arg(long)]
        amm: Option<String>,
        /// Pool address for Raydium CPMM (required for raydium_cpmm)
        #[arg(long)]
        pool: Option<String>,
    },
    /// Sell tokens for SOL
    Sell {
        /// Token mint address
        mint: String,
        /// DEX to use (pumpfun, pumpswap, bonk, raydium_v4, raydium_cpmm)
        dex: String,
        /// Amount of tokens to sell (sell all if not specified)
        #[arg(short, long)]
        amount: Option<f64>,
        /// Slippage tolerance (optional)
        #[arg(short, long)]
        slippage: Option<u64>,
        /// AMM address for Raydium V4 (required for raydium_v4)
        #[arg(long)]
        amm: Option<String>,
        /// Pool address for Raydium CPMM (required for raydium_cpmm)
        #[arg(long)]
        pool: Option<String>,
    },
    /// Wrap SOL to WSOL
    WrapSol {
        /// Amount of SOL to wrap
        #[arg(short, long)]
        amount: f64,
    },
    /// Close WSOL account and recover SOL
    CloseWsol,
    /// Check wallet status and balances
    Wallet,
    /// Start interactive mode
    Interactive,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(command) => {
            // Handle direct command line usage
            handle_command(command).await?
        }
        None => {
            // No command provided, run interactive mode
            println!("🚀 SOL Trade CLI - Interactive Mode");
            println!("═══════════════════════════════════\n");
            run_interactive_mode().await?
        }
    }

    Ok(())
}

async fn handle_command(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Buy { mint, dex, amount, slippage, amm, pool } => {
            validate_dex_params(&dex, &amm, &pool)?;
            match dex.as_str() {
                "raydium_v4" => {
                    if let Some(amm_addr) = amm {
                        handle_buy_rv4(&mint, &amm_addr, amount, slippage).await?
                    }
                }
                "raydium_cpmm" => {
                    if let Some(pool_addr) = pool {
                        handle_buy_rcpmm(&mint, &pool_addr, amount, slippage).await?
                    }
                }
                _ => handle_buy(&mint, &dex, amount, slippage).await?,
            }
        }
        Command::Sell { mint, dex, amount, slippage, amm, pool } => {
            validate_dex_params(&dex, &amm, &pool)?;
            match dex.as_str() {
                "raydium_v4" => {
                    if let Some(amm_addr) = amm {
                        handle_sell_rv4(&mint, &amm_addr, amount, slippage).await?
                    }
                }
                "raydium_cpmm" => {
                    if let Some(pool_addr) = pool {
                        handle_sell_rcpmm(&mint, &pool_addr, amount, slippage).await?
                    }
                }
                _ => handle_sell(&mint, &dex, amount, slippage).await?,
            }
        }
        Command::WrapSol { amount } => handle_wrap_sol(amount).await?,
        Command::CloseWsol => handle_close_wsol().await?,
        Command::Wallet => handle_wallet().await?,
        Command::Interactive => {
            println!("🚀 SOL Trade CLI - Interactive Mode");
            println!("═══════════════════════════════════\n");
            run_interactive_mode().await?
        }
    }
    Ok(())
}

fn validate_dex_params(
    dex: &str,
    amm: &Option<String>,
    pool: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !DEXS.contains(&dex) {
        return Err(format!("Invalid DEX: '{}'. Supported DEXs: {}", dex, DEXS.join(", ")).into());
    }

    match dex {
        "raydium_v4" => {
            if amm.is_none() {
                return Err("AMM address is required for Raydium V4. Use --amm flag.".into());
            }
        }
        "raydium_cpmm" => {
            if pool.is_none() {
                return Err("Pool address is required for Raydium CPMM. Use --pool flag.".into());
            }
        }
        _ => {}
    }
    Ok(())
}

async fn show_startup_info() {
    println!("\n📋 STARTUP INFORMATION");
    println!("══════════════════════════════════════");

    println!("🌐 RPC URL: {}", RPC_URL);

    // Try to initialize client to show wallet info
    match initialize_real_client().await {
        Ok(client) => {
            println!("👛 Wallet Address: {}", client.get_payer_pubkey());

            // Try to get balance
            match client.get_payer_sol_balance().await {
                Ok(balance) => {
                    println!("💰 SOL Balance: {:.6} SOL", balance as f64 / 1_000_000_000.0);
                }
                Err(_) => {
                    println!("💰 SOL Balance: Unable to fetch (network issue)");
                }
            }
        }
        Err(_) => {
            // Generate a temporary keypair to show the format
            let temp_keypair = solana_sdk::signature::Keypair::new();
            println!(
                "👛 Wallet Address: {} (temporary - set SOLANA_RPC_URL for real wallet)",
                temp_keypair.pubkey()
            );
            println!("💰 SOL Balance: Unable to fetch (no valid RPC connection)");
        }
    }

    println!("⚡ Network: Mainnet-beta");
    println!("══════════════════════════════════════");

    // Show help
    show_help();
    println!("══════════════════════════════════════\n");
}

async fn run_interactive_mode() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Interactive Mode Started");

    // Show startup information
    show_startup_info().await;

    println!("Type 'help' for available commands, 'quit' to exit\n");

    loop {
        print!("sol-trade> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        match input {
            "quit" | "exit" | "q" => {
                println!("👋 Goodbye!");
                break;
            }
            "help" | "h" => {
                show_help();
            }
            "wallet" | "status" => {
                handle_wallet().await?;
            }
            "close_wsol" | "close-wsol" => {
                handle_close_wsol().await?;
            }
            _ => {
                if input.starts_with("raydium_cpmm_buy ") {
                    handle_interactive_raydium_cpmm_buy(&input[16..]).await?;
                } else if input.starts_with("raydium_cpmm_sell ") {
                    handle_interactive_raydium_cpmm_sell(&input[17..]).await?;
                } else if input.starts_with("raydium_v4_buy ") {
                    handle_interactive_raydium_v4_buy(&input[14..]).await?;
                } else if input.starts_with("raydium_v4_sell ") {
                    handle_interactive_raydium_v4_sell(&input[15..]).await?;
                } else if input.starts_with("buy ") {
                    handle_interactive_buy(&input[4..]).await?;
                } else if input.starts_with("sell ") {
                    handle_interactive_sell(&input[5..]).await?;
                } else if input.starts_with("wrap_sol ") || input.starts_with("wrap-sol ") {
                    let amount_str =
                        if input.starts_with("wrap_sol ") { &input[9..] } else { &input[9..] };
                    if let Ok(amount) = amount_str.parse::<f64>() {
                        handle_wrap_sol(amount).await?;
                    } else {
                        println!("❌ Invalid amount. Usage: wrap_sol <amount>");
                    }
                } else {
                    println!(
                        "❌ Unknown command: '{}'. Type 'help' for available commands.",
                        input
                    );
                }
            }
        }
        println!(); // Add blank line after each command
    }

    Ok(())
}

fn show_help() {
    println!("📚 Available Commands:");
    println!("  buy <mint> <dex> <sol_amount> [slippage]    - Buy tokens with SOL");
    println!("  sell <mint> <dex> [token_amount] [slippage] - Sell tokens (all if no amount)");
    println!("  raydium_v4_buy <mint> <amm> <sol_amount> [slippage] - Buy tokens with SOL from Raydium V4");
    println!(
        "  raydium_v4_sell <mint> <amm> [token_amount] [slippage] - Sell tokens from Raydium V4"
    );
    println!("  raydium_cpmm_buy <mint> <pool_address> <sol_amount> [slippage] - Buy tokens with SOL from Raydium CPMM");
    println!("  raydium_cpmm_sell <mint> <pool_address> [token_amount] [slippage] - Sell tokens from Raydium CPMM");
    println!("  wrap_sol <amount>                           - Wrap SOL to WSOL");
    println!("  close_wsol                                  - Close WSOL account");
    println!("  wallet                                      - Check wallet status");
    println!("  help                                        - Show this help");
    println!("  quit                                        - Exit interactive mode");
    println!();
    println!("🏛️ Supported DEXs: {}", DEXS.join(", "));
    println!();
    println!("📝 Examples:");
    println!("  buy xxxxxxx pumpfun 1.0");
    println!("  wrap_sol 2.5");
}

async fn handle_interactive_buy(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 3 {
        println!("❌ Usage: buy <mint> <dex> <sol_amount> [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let dex = parts[1];
    let sol_amount = match parts[2].parse::<f64>() {
        Ok(amount) => amount,
        Err(_) => {
            println!("❌ Invalid SOL amount: {}", parts[2]);
            return Ok(());
        }
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_buy(mint, dex, sol_amount, slippage).await
}

async fn handle_interactive_sell(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        println!("❌ Usage: sell <mint> <dex> [token_amount] [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let dex = parts[1];
    let token_amount = if parts.len() > 2 {
        match parts[2].parse::<f64>() {
            Ok(amount) => Some(amount),
            Err(_) => {
                println!("❌ Invalid token amount: {}", parts[2]);
                return Ok(());
            }
        }
    } else {
        None
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_sell(mint, dex, token_amount, slippage).await
}

async fn handle_interactive_raydium_v4_buy(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 3 {
        println!("❌ Usage: raydium_v4_buy <mint> <amm> <sol_amount> [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let amm = parts[1];
    let sol_amount = match parts[2].parse::<f64>() {
        Ok(amount) => amount,
        Err(_) => {
            println!("❌ Invalid SOL amount: {}", parts[2]);
            return Ok(());
        }
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_buy_rv4(mint, amm, sol_amount, slippage).await
}

async fn handle_interactive_raydium_v4_sell(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        println!("❌ Usage: raydium_v4_sell <mint> <amm> [token_amount] [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let amm = parts[1];
    let token_amount = if parts.len() > 2 {
        match parts[2].parse::<f64>() {
            Ok(amount) => Some(amount),
            Err(_) => {
                println!("❌ Invalid token amount: {}", parts[2]);
                return Ok(());
            }
        }
    } else {
        None
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_sell_rv4(mint, amm, token_amount, slippage).await
}

async fn handle_interactive_raydium_cpmm_buy(args: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 3 {
        println!("❌ Usage: raydium_cpmm_buy <mint> <pool_address> <sol_amount> [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let pool_address = parts[1];
    let sol_amount = match parts[2].parse::<f64>() {
        Ok(amount) => amount,
        Err(_) => {
            println!("❌ Invalid SOL amount: {}", parts[2]);
            return Ok(());
        }
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_buy_rcpmm(mint, pool_address, sol_amount, slippage).await
}

async fn handle_interactive_raydium_cpmm_sell(
    args: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        println!("❌ Usage: raydium_cpmm_sell <mint> <pool_address> [token_amount] [slippage]");
        return Ok(());
    }

    let mint = parts[0];
    let pool_address = parts[1];
    let token_amount = if parts.len() > 2 {
        match parts[2].parse::<f64>() {
            Ok(amount) => Some(amount),
            Err(_) => {
                println!("❌ Invalid token amount: {}", parts[2]);
                return Ok(());
            }
        }
    } else {
        None
    };
    let slippage = if parts.len() > 3 { parts[3].parse::<u64>().ok() } else { None };

    handle_sell_rcpmm(mint, pool_address, token_amount, slippage).await
}

async fn check_mint_ata(
    client: &SolanaTrade,
    mint: &str,
) -> Result<(bool, bool, Pubkey, f64, u8), Box<dyn std::error::Error>> {
    let mut create_mint_ata = true;
    let mut use_seed = false;
    let mut decimals = 0;
    let mut amount_f64: f64 = 0.0;

    let mint_pubkey = Pubkey::from_str(mint).unwrap();

    if let Ok(mint_info) = client.rpc.get_account(&mint_pubkey).await {
        let owner_pubkey = mint_info.owner.clone();
        let mint_ata = get_associated_token_address_with_program_id_fast_use_seed(
            &client.get_payer_pubkey(),
            &mint_pubkey,
            &owner_pubkey,
            false,
        );
        match client.rpc.get_token_account_balance(&mint_ata).await {
            Ok(balance) => {
                let amount = balance.ui_amount.unwrap_or(0.0);
                decimals = balance.decimals;
                amount_f64 = amount as f64 * 10_f64.powi(decimals as i32);

                create_mint_ata = false;
                use_seed = false;
            }
            Err(_) => {}
        }
        if !create_mint_ata {
            return Ok((create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals));
        }

        let mint_ata = get_associated_token_address_with_program_id_fast_use_seed(
            &client.get_payer_pubkey(),
            &mint_pubkey,
            &owner_pubkey,
            true,
        );
        match client.rpc.get_token_account_balance(&mint_ata).await {
            Ok(_) => {
                create_mint_ata = false;
                use_seed = true;
            }
            Err(_) => {}
        }
        return Ok((create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals));
    }
    return Err("Mint account not found".to_string().into());
}

// Buy and sell functions - currently in demo mode since trading logic is complex
async fn handle_buy(
    mint: &str,
    dex: &str,
    sol_amount: f64,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate DEX parameter
    if !DEXS.contains(&dex) {
        println!("❌ Invalid DEX: '{}'. Supported DEXs: {}", dex, DEXS.join(", "));
        return Ok(());
    }

    let client = initialize_real_client().await?;

    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;

    match dex {
        "pumpfun" => {
            handle_buy_pumpfun(mint, sol_amount, slippage, create_mint_ata, use_seed, owner_pubkey)
                .await?;
        }
        "pumpswap" => {
            handle_buy_pumpswap(
                mint,
                sol_amount,
                slippage,
                create_mint_ata,
                use_seed,
                owner_pubkey,
            )
            .await?;
        }
        "bonk" => {
            handle_buy_bonk(mint, sol_amount, slippage, create_mint_ata, use_seed, owner_pubkey)
                .await?;
        }
        _ => {}
    }
    Ok(())
}
async fn handle_buy_rv4(
    mint: &str,
    amm: &str,
    sol_amount: f64,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;
    handle_buy_raydium_v4(mint, amm, sol_amount, slippage, create_mint_ata, use_seed, owner_pubkey)
        .await?;
    Ok(())
}

async fn handle_buy_rcpmm(
    mint: &str,
    pool_address: &str,
    sol_amount: f64,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;
    handle_buy_raydium_cpmm(
        mint,
        pool_address,
        sol_amount,
        slippage,
        create_mint_ata,
        use_seed,
        owner_pubkey,
    )
    .await?;
    Ok(())
}

async fn handle_buy_pumpfun(
    mint: &str,
    sol_amount: f64,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔥 BUY PUMPFUN COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   SOL Amount: {} SOL", sol_amount);
    if slippage.is_some() {
        println!("   Slippage: {}", slippage.unwrap());
    }
    let client = initialize_real_client().await?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = PumpFunParams::from_mint_by_rpc(&client.rpc, &mint_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let sol_lamports = sol_str_to_lamports(sol_amount.to_string().as_str()).unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let buy_params = TradeBuyParams {
        dex_type: DexType::PumpFun,
        input_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: sol_lamports,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::PumpFun(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: false,
        close_input_token_ata: false,
        create_mint_ata: create_mint_ata,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.buy(buy_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully bought tokens from PumpFun!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to buy tokens from PumpFun: {}", e);
        }
    }

    Ok(())
}

async fn handle_buy_pumpswap(
    mint: &str,
    sol_amount: f64,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    _owner_pubkey: Pubkey,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    println!("🔥 BUY PUMPSWAP COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   SOL Amount: {} SOL", sol_amount);
    if slippage.is_some() {
        println!("   Slippage: {}%", slippage.unwrap());
    }
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = PumpSwapParams::from_mint_by_rpc(&client.rpc, &mint_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let sol_lamports = sol_str_to_lamports(sol_amount.to_string().as_str()).unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let buy_params = TradeBuyParams {
        dex_type: DexType::PumpSwap,
        input_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: sol_lamports,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::PumpSwap(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: false,
        create_mint_ata: create_mint_ata,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.buy(buy_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully bought tokens from PumpSwap!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to buy tokens from PumpSwap: {}", e);
        }
    }
    Ok(())
}

async fn handle_buy_bonk(
    mint: &str,
    sol_amount: f64,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    println!("🔥 BUY BONK COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   SOL Amount: {} SOL", sol_amount);
    if slippage.is_some() {
        println!("   Slippage: {}%", slippage.unwrap());
    }
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = BonkParams::from_mint_by_rpc(&client.rpc, &mint_pubkey, false).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let sol_lamports = sol_str_to_lamports(sol_amount.to_string().as_str()).unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let buy_params = TradeBuyParams {
        dex_type: DexType::Bonk,
        input_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: sol_lamports,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::Bonk(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: false,
        create_mint_ata: create_mint_ata,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.buy(buy_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully bought tokens from Bonk!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to buy tokens from Bonk: {}", e);
        }
    }
    Ok(())
}

async fn handle_buy_raydium_v4(
    mint: &str,
    amm: &str,
    sol_amount: f64,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    println!("🔥 BUY RAYDIUM V4 COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   AMM: {}", amm);
    println!("   SOL Amount: {} SOL", sol_amount);
    if slippage.is_some() {
        println!("   Slippage: {}%", slippage.unwrap());
    }

    let mint_pubkey = Pubkey::from_str(mint)?;
    let amm_pubkey = Pubkey::from_str(amm)?;
    let param = RaydiumAmmV4Params::from_amm_address_by_rpc(&client.rpc, amm_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let sol_lamports = sol_str_to_lamports(sol_amount.to_string().as_str()).unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let buy_params = TradeBuyParams {
        dex_type: DexType::RaydiumAmmV4,
        input_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: sol_lamports,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::RaydiumAmmV4(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: false,
        create_mint_ata: create_mint_ata,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.buy(buy_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully bought tokens from Raydium V4!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to buy tokens from Raydium V4: {}", e);
        }
    }
    Ok(())
}

async fn handle_buy_raydium_cpmm(
    mint: &str,
    pool_address: &str,
    sol_amount: f64,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    println!("🔥 BUY RAYDIUM CPMM COMMAND");
    println!("   Pool Address: {}", pool_address);
    println!("   Token Mint: {}", mint);
    println!("   SOL Amount: {} SOL", sol_amount);
    if slippage.is_some() {
        println!("   Slippage: {}%", slippage.unwrap());
    }

    let mint_pubkey = Pubkey::from_str(mint)?;
    let pool_pubkey = Pubkey::from_str(pool_address)?;
    let param = RaydiumCpmmParams::from_pool_address_by_rpc(&client.rpc, &pool_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let sol_lamports = sol_str_to_lamports(sol_amount.to_string().as_str()).unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let buy_params = TradeBuyParams {
        dex_type: DexType::RaydiumCpmm,
        input_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: sol_lamports,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::RaydiumCpmm(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: false,
        create_mint_ata: create_mint_ata,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.buy(buy_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully bought tokens from Raydium CPMM!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to buy tokens from Raydium CPMM: {}", e);
        }
    }
    Ok(())
}

async fn handle_sell(
    mint: &str,
    dex: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate DEX parameter
    if !DEXS.contains(&dex) {
        println!("❌ Invalid DEX: '{}'. Supported DEXs: {}", dex, DEXS.join(", "));
        return Ok(());
    }

    let client = initialize_real_client().await?;
    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;

    match dex {
        "pumpfun" => {
            handle_sell_pumpfun(
                mint,
                token_amount,
                slippage,
                create_mint_ata,
                use_seed,
                owner_pubkey,
                amount_f64,
                decimals,
            )
            .await?;
        }
        "pumpswap" => {
            handle_sell_pumpswap(
                mint,
                token_amount,
                slippage,
                create_mint_ata,
                use_seed,
                owner_pubkey,
                amount_f64,
                decimals,
            )
            .await?;
        }
        "bonk" => {
            handle_sell_bonk(
                mint,
                token_amount,
                slippage,
                create_mint_ata,
                use_seed,
                owner_pubkey,
                amount_f64,
                decimals,
            )
            .await?;
        }
        _ => {}
    }

    Ok(())
}

async fn handle_sell_rv4(
    mint: &str,
    amm: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;
    handle_sell_raydium_v4(
        amm,
        mint,
        token_amount,
        slippage,
        create_mint_ata,
        use_seed,
        owner_pubkey,
        amount_f64,
        decimals,
    )
    .await?;
    Ok(())
}

async fn handle_sell_rcpmm(
    mint: &str,
    pool_address: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = initialize_real_client().await?;
    let (create_mint_ata, use_seed, owner_pubkey, amount_f64, decimals) =
        check_mint_ata(&client, mint).await?;
    handle_sell_raydium_cpmm(
        mint,
        pool_address,
        token_amount,
        slippage,
        create_mint_ata,
        use_seed,
        owner_pubkey,
        amount_f64,
        decimals,
    )
    .await?;
    Ok(())
}

async fn handle_sell_pumpfun(
    mint: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
    amount_f64: f64,
    decimals: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let amount = if token_amount.is_some() { token_amount.unwrap() } else { amount_f64 };

    println!("🔥 SELL PUMPFUN COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   Token Amount: {} ", amount);
    if slippage.is_some() {
        println!("   Slippage: {}%", slippage.unwrap());
    }

    let client = initialize_real_client().await?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = PumpFunParams::from_mint_by_rpc(&client.rpc, &mint_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let sell_params = TradeSellParams {
        dex_type: DexType::PumpFun,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount as u64,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::PumpFun(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };

    match client.sell(sell_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully sold tokens from PumpFun!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to sell tokens from PumpFun: {}", e);
        }
    }

    Ok(())
}

async fn handle_sell_pumpswap(
    mint: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
    amount_f64: f64,
    decimals: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let amount = if token_amount.is_some() { token_amount.unwrap() } else { amount_f64 };
    println!("🔥 SELL PUMPSWAP COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   Token Amount: {}", amount);
    if slippage.is_some() {
        println!("   Slippage: {}", slippage.unwrap());
    }
    let client = initialize_real_client().await?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = PumpSwapParams::from_mint_by_rpc(&client.rpc, &mint_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let sell_params = TradeSellParams {
        dex_type: DexType::PumpSwap,
        output_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: amount as u64,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::PumpSwap(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.sell(sell_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully sold tokens from PumpSwap!");
            println!("   ✅ Transaction Signature: {}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to sell tokens from PumpSwap: {}", e);
        }
    }

    Ok(())
}

async fn handle_sell_bonk(
    mint: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
    amount_f64: f64,
    decimals: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let amount = if token_amount.is_some() { token_amount.unwrap() } else { amount_f64 };
    println!("🔥 SELL PUMPSWAP COMMAND");
    println!("   Token Mint: {}", mint);
    println!("   Token Amount: {}", amount);
    if slippage.is_some() {
        println!("   Slippage: {}", slippage.unwrap());
    }
    let client = initialize_real_client().await?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = BonkParams::from_mint_by_rpc(&client.rpc, &mint_pubkey, false).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let sell_params = TradeSellParams {
        dex_type: DexType::Bonk,
        output_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: amount as u64,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::Bonk(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.sell(sell_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully sold tokens from Bonk!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to sell tokens from Bonk: {}", e);
        }
    }

    Ok(())
}

async fn handle_sell_raydium_v4(
    amm: &str,
    mint: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
    amount_f64: f64,
    decimals: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let amount = if token_amount.is_some() { token_amount.unwrap() } else { amount_f64 };
    println!("🔥 SELL RAYDIUM V4 COMMAND");
    println!("   AMM: {}", amm);
    println!("   Token Mint: {}", mint);
    println!("   Token Amount: {}", amount);
    if slippage.is_some() {
        println!("   Slippage: {}", slippage.unwrap());
    }
    let client = initialize_real_client().await?;
    let amm_pubkey = Pubkey::from_str(amm)?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = RaydiumAmmV4Params::from_amm_address_by_rpc(&client.rpc, amm_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let sell_params = TradeSellParams {
        dex_type: DexType::RaydiumAmmV4,
        output_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: amount as u64,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::RaydiumAmmV4(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.sell(sell_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully sold tokens from Raydium V4!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to sell tokens from Raydium V4: {}", e);
        }
    }

    Ok(())
}

async fn handle_sell_raydium_cpmm(
    mint: &str,
    pool_address: &str,
    token_amount: Option<f64>,
    slippage: Option<u64>,
    create_mint_ata: bool,
    use_seed: bool,
    owner_pubkey: Pubkey,
    amount_f64: f64,
    decimals: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let amount = if token_amount.is_some() { token_amount.unwrap() } else { amount_f64 };
    println!("🔥 SELL RAYDIUM CPMM COMMAND");
    println!("   Pool Address: {}", pool_address);
    println!("   Token Mint: {}", mint);
    println!("   Token Amount: {}", amount);
    if slippage.is_some() {
        println!("   Slippage: {}", slippage.unwrap());
    }
    let client = initialize_real_client().await?;
    let pool_pubkey = Pubkey::from_str(pool_address)?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let param = RaydiumCpmmParams::from_pool_address_by_rpc(&client.rpc, &pool_pubkey).await?;
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    let sell_params = TradeSellParams {
        dex_type: DexType::RaydiumCpmm,
        output_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: amount as u64,
        slippage_basis_points: slippage,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::RaydiumCpmm(param),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    match client.sell(sell_params).await {
        Ok((_, signature, _)) => {
            println!("   ✅ Successfully sold tokens from Raydium CPMM!");
            println!("   ✅ Transaction Signature: {:?}", signature);
        }
        Err(e) => {
            println!("   ❌ Failed to sell tokens from Raydium CPMM: {}", e);
        }
    }

    Ok(())
}

async fn handle_wrap_sol(amount: f64) -> Result<(), Box<dyn std::error::Error>> {
    println!("📦 WRAP SOL COMMAND");
    println!("   Amount: {} SOL → {} WSOL", amount, amount);

    match initialize_real_client().await {
        Ok(client) => {
            let amount_lamports = (amount * 1_000_000_000.0) as u64;

            match wrap_sol_real(&client, amount_lamports).await {
                Ok(_) => {
                    println!("   ✅ Successfully wrapped {} SOL to WSOL!", amount);
                }
                Err(e) => {
                    println!("   ❌ Failed to wrap SOL: {}", e);
                }
            }
        }
        Err(_) => {
            println!("   ⚠️ Cannot connect to Solana network");
        }
    }

    Ok(())
}

async fn handle_close_wsol() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 CLOSE WSOL COMMAND");
    println!("   Action: Closing WSOL account to recover SOL");

    match initialize_real_client().await {
        Ok(client) => {
            // Check WSOL balance first
            let wsol_mint = WSOL_TOKEN_ACCOUNT;
            match client.get_payer_token_balance(&wsol_mint).await {
                Ok(wsol_balance) => {
                    if wsol_balance == 0 {
                        println!("   ⚠️ No WSOL balance found to recover");
                        return Ok(());
                    }

                    println!(
                        "   💰 Found {:.6} WSOL to recover",
                        wsol_balance as f64 / 1_000_000_000.0
                    );

                    match close_wsol_real(&client).await {
                        Ok(_) => {
                            println!(
                                "   ✅ Successfully closed WSOL account and recovered {:.6} SOL",
                                wsol_balance as f64 / 1_000_000_000.0
                            );
                        }
                        Err(e) => {
                            println!("   ❌ Failed to close WSOL account: {}", e);
                        }
                    }
                }
                Err(_) => {
                    println!("   ⚠️ No WSOL account found");
                }
            }
        }
        Err(_) => {
            println!("   ⚠️ Cannot connect to Solana network");
        }
    }

    Ok(())
}

async fn handle_wallet() -> Result<(), Box<dyn std::error::Error>> {
    println!("👛 WALLET STATUS");

    match initialize_real_client().await {
        Ok(client) => match get_wallet_real(&client).await {
            Ok(_) => {}
            Err(e) => {
                println!("   ❌ Failed to get wallet status: {}", e);
            }
        },
        Err(_) => {
            println!("   ⚠️ Cannot connect to Solana network");
        }
    }

    Ok(())
}

// Real implementation functions
async fn initialize_real_client() -> AnyResult<SolanaTrade> {
    // You need to update this with a real RPC URL
    println!("🚀 Initializing SolanaTrade client...");
    let payer = Arc::new(Keypair::try_from(&PAYER.to_bytes()[..]).unwrap());
    let rpc_url = RPC_URL.to_string();
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let solana_trade = SolanaTrade::new(payer, trade_config).await;
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

async fn wrap_sol_real(
    client: &SolanaTrade,
    amount_lamports: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use solana_sdk::transaction::Transaction;

    // Check balance first
    let balance = client.get_payer_sol_balance().await?;
    if balance < amount_lamports + 1_000_000 {
        // Need extra for transaction fees
        return Err(format!(
            "Insufficient balance. Have {:.3} SOL, need {:.3} SOL (including fees)",
            balance as f64 / 1_000_000_000.0,
            (amount_lamports + 1_000_000) as f64 / 1_000_000_000.0
        )
        .into());
    }

    // Create WSOL account and wrap SOL
    let wsol_mint = "So11111111111111111111111111111111111111112".parse().unwrap();
    let instructions = vec![
        // Create associated token account for WSOL
        create_associated_token_account_idempotent(
            &client.get_payer_pubkey(),
            &client.get_payer_pubkey(),
            &wsol_mint,
            &spl_token::ID,
        ),
        // Transfer SOL to WSOL account
        transfer(
            &client.get_payer_pubkey(),
            &get_associated_token_address(&client.get_payer_pubkey(), &wsol_mint),
            amount_lamports,
        ),
        // Sync native (convert SOL to WSOL tokens)
        Instruction {
            program_id: sol_trade_sdk::constants::TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(
                get_associated_token_address(&client.get_payer_pubkey(), &wsol_mint),
                false,
            )],
            data: vec![17],
        },
    ];

    // Build and send transaction
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let transaction = Transaction::new_signed_with_payer(
        &instructions,
        Some(&client.get_payer_pubkey()),
        &[client.get_payer()],
        recent_blockhash,
    );

    let signature = client.rpc.send_and_confirm_transaction(&transaction).await?;
    println!("   📝 Transaction Signature: {}", signature);
    Ok(())
}

async fn close_wsol_real(client: &SolanaTrade) -> Result<(), Box<dyn std::error::Error>> {
    use solana_sdk::transaction::Transaction;

    let wsol_mint = "So11111111111111111111111111111111111111112".parse().unwrap();
    let wsol_account = get_associated_token_address(&client.get_payer_pubkey(), &wsol_mint);

    // Check if WSOL account exists
    let account_info = client.rpc.get_account(&wsol_account).await;
    if account_info.is_err() {
        return Err("WSOL account not found".into());
    }

    // Close WSOL account instruction
    let close_instruction = close_account(
        &spl_token::ID,
        &wsol_account,
        &client.get_payer_pubkey(),
        &client.get_payer_pubkey(),
        &[],
    )?;

    // Build and send transaction
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let transaction = Transaction::new_signed_with_payer(
        &[close_instruction],
        Some(&client.get_payer_pubkey()),
        &[client.get_payer()],
        recent_blockhash,
    );

    let signature = client.rpc.send_and_confirm_transaction(&transaction).await?;
    println!("   📝 Transaction Signature: {}", signature);
    Ok(())
}

async fn get_wallet_real(client: &SolanaTrade) -> Result<(), Box<dyn std::error::Error>> {
    // Get SOL balance
    let sol_balance = client.get_payer_sol_balance().await?;

    // Get WSOL balance
    let wsol_mint = "So11111111111111111111111111111111111111112".parse().unwrap();
    let wsol_balance = client.get_payer_token_balance(&wsol_mint).await.unwrap_or(0);

    // Get wallet address
    let wallet_address = client.get_payer_pubkey();

    println!("\n   📊 REAL WALLET OVERVIEW:");
    println!("   ═══════════════════════════════════");
    println!("   💰 SOL Balance:    {:.6} SOL", sol_balance as f64 / 1_000_000_000.0);
    println!("   🪙  WSOL Balance:   {:.6} WSOL", wsol_balance as f64 / 1_000_000_000.0);
    println!("   🏛️  Wallet Address: {}", wallet_address);
    println!("   ⚡ Network:        Mainnet-beta");

    Ok(())
}
