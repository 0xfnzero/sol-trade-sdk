use crate::common::bonding_curve::BondingCurveAccount;
use crate::common::nonce_cache::DurableNonceInfo;
use crate::common::spl_associated_token_account::get_associated_token_address_with_program_id;
use crate::common::{GasFeeStrategy, SolanaRpcClient};
use crate::constants::TOKEN_PROGRAM;
use crate::instruction::utils::pumpfun::global_constants::MAYHEM_FEE_RECIPIENT;
use crate::instruction::utils::pumpswap::accounts::MAYHEM_FEE_RECIPIENT as MAYHEM_FEE_RECIPIENT_SWAP;
use crate::swqos::{SwqosClient, TradeType};
use crate::trading::common::get_multi_token_balances;
use crate::trading::MiddlewareManager;
use solana_hash::Hash;
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

/// DEX 参数枚举 - 零开销抽象替代 Box<dyn ProtocolParams>
#[derive(Clone)]
pub enum DexParamEnum {
    PumpFun(PumpFunParams),
    PumpSwap(PumpSwapParams),
    Bonk(BonkParams),
    RaydiumCpmm(RaydiumCpmmParams),
    RaydiumAmmV4(RaydiumAmmV4Params),
    MeteoraDammV2(MeteoraDammV2Params),
}

impl DexParamEnum {
    /// 获取内部参数的 Any 引用，用于向后兼容的类型检查
    #[inline]
    pub fn as_any(&self) -> &dyn std::any::Any {
        match self {
            DexParamEnum::PumpFun(p) => p,
            DexParamEnum::PumpSwap(p) => p,
            DexParamEnum::Bonk(p) => p,
            DexParamEnum::RaydiumCpmm(p) => p,
            DexParamEnum::RaydiumAmmV4(p) => p,
            DexParamEnum::MeteoraDammV2(p) => p,
        }
    }
}

/// Swap parameters
#[derive(Clone)]
pub struct SwapParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub trade_type: TradeType,
    pub input_mint: Pubkey,
    pub input_token_program: Option<Pubkey>,
    pub output_mint: Pubkey,
    pub output_token_program: Option<Pubkey>,
    pub input_amount: Option<u64>,
    pub slippage_basis_points: Option<u64>,
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
    pub recent_blockhash: Option<Hash>,
    pub wait_transaction_confirmed: bool,
    pub protocol_params: DexParamEnum,
    pub open_seed_optimize: bool,
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    pub durable_nonce: Option<DurableNonceInfo>,
    pub with_tip: bool,
    pub create_input_mint_ata: bool,
    pub close_input_mint_ata: bool,
    pub create_output_mint_ata: bool,
    pub close_output_mint_ata: bool,
    pub fixed_output_amount: Option<u64>,
    pub gas_fee_strategy: GasFeeStrategy,
    pub simulate: bool,
    /// Use exact SOL amount instructions (buy_exact_sol_in for PumpFun, buy_exact_quote_in for PumpSwap).
    /// When Some(true) or None (default), the exact SOL/quote amount is spent and slippage is applied to output tokens.
    /// When Some(false), uses regular buy instruction where slippage is applied to SOL/quote input.
    /// This option only applies to PumpFun and PumpSwap DEXes; it is ignored for other DEXes.
    pub use_exact_sol_amount: Option<bool>,
}

impl std::fmt::Debug for SwapParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwapParams: ...")
    }
}

/// PumpFun protocol specific parameters
/// Configuration parameters specific to PumpFun trading protocol
#[derive(Clone)]
pub struct PumpFunParams {
    pub bonding_curve: Arc<BondingCurveAccount>,
    pub associated_bonding_curve: Pubkey,
    pub creator_vault: Pubkey,
    pub token_program: Pubkey,
    /// Whether to close token account when selling, only effective during sell operations
    pub close_token_account_when_sell: Option<bool>,
}

impl PumpFunParams {
    pub fn immediate_sell(
        creator_vault: Pubkey,
        token_program: Pubkey,
        close_token_account_when_sell: bool,
    ) -> Self {
        Self {
            bonding_curve: Arc::new(BondingCurveAccount { ..Default::default() }),
            associated_bonding_curve: Pubkey::default(),
            creator_vault: creator_vault,
            token_program: token_program,
            close_token_account_when_sell: Some(close_token_account_when_sell),
        }
    }

    pub fn from_dev_trade(
        mint: Pubkey,
        token_amount: u64,
        max_sol_cost: u64,
        creator: Pubkey,
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        creator_vault: Pubkey,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT;
        let bonding_curve_account = BondingCurveAccount::from_dev_trade(
            bonding_curve,
            &mint,
            token_amount,
            max_sol_cost,
            creator,
            is_mayhem_mode,
        );
        Self {
            bonding_curve: Arc::new(bonding_curve_account),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
        }
    }

    pub fn from_trade(
        bonding_curve: Pubkey,
        associated_bonding_curve: Pubkey,
        mint: Pubkey,
        creator: Pubkey,
        creator_vault: Pubkey,
        virtual_token_reserves: u64,
        virtual_sol_reserves: u64,
        real_token_reserves: u64,
        real_sol_reserves: u64,
        close_token_account_when_sell: Option<bool>,
        fee_recipient: Pubkey,
        token_program: Pubkey,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT;
        let bonding_curve = BondingCurveAccount::from_trade(
            bonding_curve,
            mint,
            creator,
            virtual_token_reserves,
            virtual_sol_reserves,
            real_token_reserves,
            real_sol_reserves,
            is_mayhem_mode,
        );
        Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
            token_program: token_program,
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let account =
            crate::instruction::utils::pumpfun::fetch_bonding_curve_account(rpc, mint).await?;
        let mint_account = rpc.get_account(&mint).await?;
        let bonding_curve = BondingCurveAccount {
            discriminator: 0,
            account: account.1,
            virtual_token_reserves: account.0.virtual_token_reserves,
            virtual_sol_reserves: account.0.virtual_sol_reserves,
            real_token_reserves: account.0.real_token_reserves,
            real_sol_reserves: account.0.real_sol_reserves,
            token_total_supply: account.0.token_total_supply,
            complete: account.0.complete,
            creator: account.0.creator,
            is_mayhem_mode: account.0.is_mayhem_mode,
        };
        let associated_bonding_curve = get_associated_token_address_with_program_id(
            &bonding_curve.account,
            mint,
            &mint_account.owner,
        );
        let creator_vault =
            crate::instruction::utils::pumpfun::get_creator_vault_pda(&bonding_curve.creator);
        Ok(Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault.unwrap(),
            close_token_account_when_sell: None,
            token_program: mint_account.owner,
        })
    }
}

/// PumpSwap Protocol Specific Parameters
///
/// Parameters for configuring PumpSwap trading protocol, including liquidity pool information,
/// token configuration, and transaction amounts.
///
/// **Performance Note**: If these parameters are not provided, the system will attempt to
/// retrieve the relevant information from RPC, which will increase transaction time.
/// For optimal performance, it is recommended to provide all necessary parameters in advance.
#[derive(Clone)]
pub struct PumpSwapParams {
    /// Liquidity pool address
    pub pool: Pubkey,
    /// Base token mint address
    /// The mint account address of the base token in the trading pair
    pub base_mint: Pubkey,
    /// Quote token mint address
    /// The mint account address of the quote token in the trading pair, usually SOL or USDC
    pub quote_mint: Pubkey,
    /// Pool base token account
    pub pool_base_token_account: Pubkey,
    /// Pool quote token account
    pub pool_quote_token_account: Pubkey,
    /// Base token reserves in the pool
    pub pool_base_token_reserves: u64,
    /// Quote token reserves in the pool
    pub pool_quote_token_reserves: u64,
    /// Coin creator vault ATA
    pub coin_creator_vault_ata: Pubkey,
    /// Coin creator vault authority
    pub coin_creator_vault_authority: Pubkey,
    /// Token program ID
    pub base_token_program: Pubkey,
    /// Quote token program ID
    pub quote_token_program: Pubkey,
    /// Whether the pool is in mayhem mode
    pub is_mayhem_mode: bool,
}

impl PumpSwapParams {
    pub fn new(
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_account: Pubkey,
        pool_quote_token_account: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        coin_creator_vault_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
        fee_recipient: Pubkey,
    ) -> Self {
        let is_mayhem_mode = fee_recipient == MAYHEM_FEE_RECIPIENT_SWAP;
        Self {
            pool,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            base_token_program,
            quote_token_program,
            is_mayhem_mode,
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        if let Ok((pool_address, _)) =
            crate::instruction::utils::pumpswap::find_by_base_mint(rpc, mint).await
        {
            Self::from_pool_address_by_rpc(rpc, &pool_address).await
        } else if let Ok((pool_address, _)) =
            crate::instruction::utils::pumpswap::find_by_quote_mint(rpc, mint).await
        {
            Self::from_pool_address_by_rpc(rpc, &pool_address).await
        } else {
            return Err(anyhow::anyhow!("No pool found for mint"));
        }
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data = crate::instruction::utils::pumpswap::fetch_pool(rpc, pool_address).await?;
        let (pool_base_token_reserves, pool_quote_token_reserves) =
            crate::instruction::utils::pumpswap::get_token_balances(&pool_data, rpc).await?;
        let creator = pool_data.coin_creator;
        let coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(
            creator,
            pool_data.quote_mint,
        );
        let coin_creator_vault_authority =
            crate::instruction::utils::pumpswap::coin_creator_vault_authority(creator);

        let base_token_program_ata = get_associated_token_address_with_program_id(
            &pool_address,
            &pool_data.base_mint,
            &crate::constants::TOKEN_PROGRAM,
        );
        let quote_token_program_ata = get_associated_token_address_with_program_id(
            &pool_address,
            &pool_data.quote_mint,
            &crate::constants::TOKEN_PROGRAM,
        );

        Ok(Self {
            pool: pool_address.clone(),
            base_mint: pool_data.base_mint,
            quote_mint: pool_data.quote_mint,
            pool_base_token_account: pool_data.pool_base_token_account,
            pool_quote_token_account: pool_data.pool_quote_token_account,
            pool_base_token_reserves: pool_base_token_reserves,
            pool_quote_token_reserves: pool_quote_token_reserves,
            coin_creator_vault_ata: coin_creator_vault_ata,
            coin_creator_vault_authority: coin_creator_vault_authority,
            base_token_program: if pool_data.pool_base_token_account == base_token_program_ata {
                crate::constants::TOKEN_PROGRAM
            } else {
                crate::constants::TOKEN_PROGRAM_2022
            },
            quote_token_program: if pool_data.pool_quote_token_account == quote_token_program_ata {
                crate::constants::TOKEN_PROGRAM
            } else {
                crate::constants::TOKEN_PROGRAM_2022
            },
            is_mayhem_mode: pool_data.is_mayhem_mode,
        })
    }
}

/// Bonk protocol specific parameters
/// Configuration parameters specific to Bonk trading protocol
#[derive(Clone, Default)]
pub struct BonkParams {
    pub virtual_base: u128,
    pub virtual_quote: u128,
    pub real_base: u128,
    pub real_quote: u128,
    pub pool_state: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    /// Token program ID
    pub mint_token_program: Pubkey,
    pub platform_config: Pubkey,
    pub platform_associated_account: Pubkey,
    pub creator_associated_account: Pubkey,
    pub global_config: Pubkey,
}

impl BonkParams {
    pub fn immediate_sell(
        mint_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        Self {
            mint_token_program,
            platform_config,
            platform_associated_account,
            creator_associated_account,
            global_config,
            ..Default::default()
        }
    }
    pub fn from_trade(
        virtual_base: u64,
        virtual_quote: u64,
        real_base_after: u64,
        real_quote_after: u64,
        pool_state: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        Self {
            virtual_base: virtual_base as u128,
            virtual_quote: virtual_quote as u128,
            real_base: real_base_after as u128,
            real_quote: real_quote_after as u128,
            pool_state: pool_state,
            base_vault: base_vault,
            quote_vault: quote_vault,
            mint_token_program: base_token_program,
            platform_config: platform_config,
            platform_associated_account: platform_associated_account,
            creator_associated_account: creator_associated_account,
            global_config: global_config,
        }
    }

    pub fn from_dev_trade(
        is_exact_in: bool,
        amount_in: u64,
        amount_out: u64,
        pool_state: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
        global_config: Pubkey,
    ) -> Self {
        const DEFAULT_VIRTUAL_BASE: u128 = 1073025605596382;
        const DEFAULT_VIRTUAL_QUOTE: u128 = 30000852951;
        let _amount_in = if is_exact_in {
            amount_in
        } else {
            crate::instruction::utils::bonk::get_amount_in(
                amount_out,
                crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
                crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
                crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
                DEFAULT_VIRTUAL_BASE,
                DEFAULT_VIRTUAL_QUOTE,
                0,
                0,
                0,
            )
        };
        let real_quote = crate::instruction::utils::bonk::get_amount_in_net(
            amount_in,
            crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
            crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
            crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
        ) as u128;
        let _amount_out = if is_exact_in {
            crate::instruction::utils::bonk::get_amount_out(
                amount_in,
                crate::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE,
                crate::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE,
                crate::instruction::utils::bonk::accounts::SHARE_FEE_RATE,
                DEFAULT_VIRTUAL_BASE,
                DEFAULT_VIRTUAL_QUOTE,
                0,
                0,
                0,
            ) as u128
        } else {
            amount_out as u128
        };
        let real_base = _amount_out;
        Self {
            virtual_base: DEFAULT_VIRTUAL_BASE,
            virtual_quote: DEFAULT_VIRTUAL_QUOTE,
            real_base: real_base,
            real_quote: real_quote,
            pool_state: pool_state,
            base_vault: base_vault,
            quote_vault: quote_vault,
            mint_token_program: base_token_program,
            platform_config: platform_config,
            platform_associated_account: platform_associated_account,
            creator_associated_account: creator_associated_account,
            global_config: global_config,
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
        usd1_pool: bool,
    ) -> Result<Self, anyhow::Error> {
        let pool_address = crate::instruction::utils::bonk::get_pool_pda(
            mint,
            if usd1_pool {
                &crate::constants::USD1_TOKEN_ACCOUNT
            } else {
                &crate::constants::WSOL_TOKEN_ACCOUNT
            },
        )
        .unwrap();
        let pool_data =
            crate::instruction::utils::bonk::fetch_pool_state(rpc, &pool_address).await?;
        let token_account = rpc.get_account(&pool_data.base_mint).await?;
        let platform_associated_account =
            crate::instruction::utils::bonk::get_platform_associated_account(
                &pool_data.platform_config,
            );
        let creator_associated_account =
            crate::instruction::utils::bonk::get_creator_associated_account(&pool_data.creator);
        let platform_associated_account = platform_associated_account.unwrap();
        let creator_associated_account = creator_associated_account.unwrap();
        Ok(Self {
            virtual_base: pool_data.virtual_base as u128,
            virtual_quote: pool_data.virtual_quote as u128,
            real_base: pool_data.real_base as u128,
            real_quote: pool_data.real_quote as u128,
            pool_state: pool_address,
            base_vault: pool_data.base_vault,
            quote_vault: pool_data.quote_vault,
            mint_token_program: token_account.owner,
            platform_config: pool_data.platform_config,
            platform_associated_account,
            creator_associated_account,
            global_config: pool_data.global_config,
        })
    }
}

/// RaydiumCpmm protocol specific parameters
/// Configuration parameters specific to Raydium CPMM trading protocol
#[derive(Clone)]
pub struct RaydiumCpmmParams {
    /// Pool address
    pub pool_state: Pubkey,
    /// Amm config address
    pub amm_config: Pubkey,
    /// Base token mint address
    pub base_mint: Pubkey,
    /// Quote token mint address
    pub quote_mint: Pubkey,
    /// Base token reserve amount in the pool
    pub base_reserve: u64,
    /// Quote token reserve amount in the pool
    pub quote_reserve: u64,
    /// Base token vault address
    pub base_vault: Pubkey,
    /// Quote token vault address
    pub quote_vault: Pubkey,
    /// Base token program ID
    pub base_token_program: Pubkey,
    /// Quote token program ID
    pub quote_token_program: Pubkey,
    /// Observation state account
    pub observation_state: Pubkey,
}

impl RaydiumCpmmParams {
    pub fn from_trade(
        pool_state: Pubkey,
        amm_config: Pubkey,
        input_token_mint: Pubkey,
        output_token_mint: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        input_token_program: Pubkey,
        output_token_program: Pubkey,
        observation_state: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Self {
        Self {
            pool_state: pool_state,
            amm_config: amm_config,
            base_mint: input_token_mint,
            quote_mint: output_token_mint,
            base_reserve: base_reserve,
            quote_reserve: quote_reserve,
            base_vault: input_vault,
            quote_vault: output_vault,
            base_token_program: input_token_program,
            quote_token_program: output_token_program,
            observation_state: observation_state,
        }
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool =
            crate::instruction::utils::raydium_cpmm::fetch_pool_state(rpc, pool_address).await?;
        let (token0_balance, token1_balance) =
            crate::instruction::utils::raydium_cpmm::get_pool_token_balances(
                rpc,
                pool_address,
                &pool.token0_mint,
                &pool.token1_mint,
            )
            .await?;
        Ok(Self {
            pool_state: pool_address.clone(),
            amm_config: pool.amm_config,
            base_mint: pool.token0_mint,
            quote_mint: pool.token1_mint,
            base_reserve: token0_balance,
            quote_reserve: token1_balance,
            base_vault: pool.token0_vault,
            quote_vault: pool.token1_vault,
            base_token_program: pool.token0_program,
            quote_token_program: pool.token1_program,
            observation_state: pool.observation_key,
        })
    }
}

/// RaydiumCpmm protocol specific parameters
/// Configuration parameters specific to Raydium CPMM trading protocol
#[derive(Clone)]
pub struct RaydiumAmmV4Params {
    /// AMM pool address
    pub amm: Pubkey,
    /// Base token (coin) mint address
    pub coin_mint: Pubkey,
    /// Quote token (pc) mint address  
    pub pc_mint: Pubkey,
    /// Pool's coin token account address
    pub token_coin: Pubkey,
    /// Pool's pc token account address
    pub token_pc: Pubkey,
    /// Current coin reserve amount in the pool
    pub coin_reserve: u64,
    /// Current pc reserve amount in the pool
    pub pc_reserve: u64,
}

impl RaydiumAmmV4Params {
    pub fn new(
        amm: Pubkey,
        coin_mint: Pubkey,
        pc_mint: Pubkey,
        token_coin: Pubkey,
        token_pc: Pubkey,
        coin_reserve: u64,
        pc_reserve: u64,
    ) -> Self {
        Self { amm, coin_mint, pc_mint, token_coin, token_pc, coin_reserve, pc_reserve }
    }
    pub async fn from_amm_address_by_rpc(
        rpc: &SolanaRpcClient,
        amm: Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let amm_info = crate::instruction::utils::raydium_amm_v4::fetch_amm_info(rpc, amm).await?;
        let (coin_reserve, pc_reserve) =
            get_multi_token_balances(rpc, &amm_info.token_coin, &amm_info.token_pc).await?;
        Ok(Self {
            amm,
            coin_mint: amm_info.coin_mint,
            pc_mint: amm_info.pc_mint,
            token_coin: amm_info.token_coin,
            token_pc: amm_info.token_pc,
            coin_reserve,
            pc_reserve,
        })
    }
}

/// MeteoraDammV2 protocol specific parameters
/// Configuration parameters specific to Meteora Damm V2 trading protocol
#[derive(Clone)]
pub struct MeteoraDammV2Params {
    pub pool: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_program: Pubkey,
    pub token_b_program: Pubkey,
}

impl MeteoraDammV2Params {
    pub fn new(
        pool: Pubkey,
        token_a_vault: Pubkey,
        token_b_vault: Pubkey,
        token_a_mint: Pubkey,
        token_b_mint: Pubkey,
        token_a_program: Pubkey,
        token_b_program: Pubkey,
    ) -> Self {
        Self {
            pool,
            token_a_vault,
            token_b_vault,
            token_a_mint,
            token_b_mint,
            token_a_program,
            token_b_program,
        }
    }

    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        pool_address: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_data =
            crate::instruction::utils::meteora_damm_v2::fetch_pool(rpc, pool_address).await?;
        Ok(Self {
            pool: pool_address.clone(),
            token_a_vault: pool_data.token_a_vault,
            token_b_vault: pool_data.token_b_vault,
            token_a_mint: pool_data.token_a_mint,
            token_b_mint: pool_data.token_b_mint,
            token_a_program: TOKEN_PROGRAM,
            token_b_program: TOKEN_PROGRAM,
        })
    }
}
