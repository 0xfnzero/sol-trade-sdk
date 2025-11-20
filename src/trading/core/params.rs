use super::traits::ProtocolParams;
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

/// Swap parameters
#[derive(Clone)]
pub struct SwapParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub trade_type: TradeType,
    pub input_mint: Pubkey,
    pub input_token_program: Pubkey,
    pub output_mint: Pubkey,
    pub output_token_program: Pubkey,
    pub input_amount: Option<u64>,
    pub slippage_basis_points: Option<u64>,
    pub address_lookup_table_account: Option<AddressLookupTableAccount>,
    pub recent_blockhash: Option<Hash>,
    pub data_size_limit: u32,
    pub wait_transaction_confirmed: bool,
    pub protocol_params: Box<dyn ProtocolParams>,
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
}

impl std::fmt::Debug for SwapParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwapParams: {:?}", self)
    }
}

/// PumpFun protocol specific parameters
/// Configuration parameters specific to PumpFun trading protocol
#[derive(Clone)]
pub struct PumpFunParams {
    pub bonding_curve: Arc<BondingCurveAccount>,
    pub associated_bonding_curve: Pubkey,
    pub creator_vault: Pubkey,
    /// Whether to close token account when selling, only effective during sell operations
    pub close_token_account_when_sell: Option<bool>,
}

impl PumpFunParams {
    pub fn immediate_sell(creator_vault: Pubkey, close_token_account_when_sell: bool) -> Self {
        Self {
            bonding_curve: Arc::new(BondingCurveAccount { ..Default::default() }),
            associated_bonding_curve: Pubkey::default(),
            creator_vault: creator_vault,
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
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let account =
            crate::instruction::utils::pumpfun::fetch_bonding_curve_account(rpc, mint).await?;
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
            &crate::constants::TOKEN_PROGRAM,
        );
        let creator_vault =
            crate::instruction::utils::pumpfun::get_creator_vault_pda(&bonding_curve.creator);
        Ok(Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: associated_bonding_curve,
            creator_vault: creator_vault.unwrap(),
            close_token_account_when_sell: None,
        })
    }
}

impl ProtocolParams for PumpFunParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
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

impl ProtocolParams for PumpSwapParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
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

impl ProtocolParams for BonkParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
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

impl ProtocolParams for RaydiumCpmmParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}

/// RaydiumCpmm protocol specific parameters
/// Configuration parameters specific to Raydium CPMM trading protocol
#[derive(Clone)]
pub struct RaydiumAmmV4Params {
    /// AMM pool address
    pub amm: Pubkey,
    /// Base token (coin) mint address
    pub base_mint: Pubkey,
    /// Quote token (pc) mint address  
    pub quote_mint: Pubkey,
    /// Pool's coin token account address
    pub base_vault: Pubkey,
    /// Pool's pc token account address
    pub quote_vault: Pubkey,
    /// Current coin reserve amount in the pool
    pub base_reserve: u64,
    /// Current pc reserve amount in the pool
    pub quote_reserve: u64,
}

impl RaydiumAmmV4Params {
    pub fn new(
        amm: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Self {
        Self { amm, base_mint, quote_mint, base_vault, quote_vault, base_reserve, quote_reserve }
    }
    pub async fn from_amm_address_by_rpc(
        rpc: &SolanaRpcClient,
        amm: Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let amm_info = crate::instruction::utils::raydium_amm_v4::fetch_amm_info(rpc, amm).await?;
        let (base_reserve, quote_reserve) =
            get_multi_token_balances(rpc, &amm_info.base_vault, &amm_info.quote_vault).await?;
        Ok(Self {
            amm,
            base_mint: amm_info.base_mint,
            quote_mint: amm_info.quote_mint,
            base_vault: amm_info.base_vault,
            quote_vault: amm_info.quote_vault,
            base_reserve,
            quote_reserve,
        })
    }
}

impl ProtocolParams for RaydiumAmmV4Params {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
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

impl ProtocolParams for MeteoraDammV2Params {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct MeteoraDlmmParams {
    pub lb_pair: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub bin_array: Vec<Pubkey>,
}

impl MeteoraDlmmParams {
    pub fn new(
        lb_pair: Pubkey,
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        token_x_mint: Pubkey,
        token_y_mint: Pubkey,
        oracle: Pubkey,
        token_x_program: Pubkey,
        token_y_program: Pubkey,
        bin_array: Vec<Pubkey>,
    ) -> Self {
        Self {
            lb_pair,
            reserve_x,
            reserve_y,
            token_x_mint,
            token_y_mint,
            oracle,
            token_x_program,
            token_y_program,
            bin_array,
        }
    }
}

impl ProtocolParams for MeteoraDlmmParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct OrcaParams {
    pub pool: Pubkey,
    pub token_mint_a: Pubkey,
    pub token_mint_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub tick_array0: Pubkey,
    pub tick_array1: Pubkey,
    pub tick_array2: Pubkey,
    pub oracle: Pubkey,
    pub amount_specified_is_input: bool,
    pub a_to_b: bool,
}

impl OrcaParams {
    pub fn new(
        pool: Pubkey,
        token_mint_a: Pubkey,
        token_mint_b: Pubkey,
        vault_a: Pubkey,
        vault_b: Pubkey,
        tick_array0: Pubkey,
        tick_array1: Pubkey,
        tick_array2: Pubkey,
        oracle: Pubkey,
        amount_specified_is_input: bool,
        a_to_b: bool,
    ) -> Self {
        Self {
            pool,
            token_mint_a,
            token_mint_b,
            vault_a,
            vault_b,
            tick_array0,
            tick_array1,
            tick_array2,
            oracle,
            amount_specified_is_input,
            a_to_b,
        }
    }
}

impl ProtocolParams for OrcaParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct RaydiumClmmParams {
    pub amm_config: Pubkey,
    pub pool: Pubkey,
    pub input_token_vault: Pubkey,
    pub output_token_vault: Pubkey,
    pub observation_key: Pubkey,
    pub tick_arrays: Vec<Pubkey>,
    pub other_amount_threshold: u64,
    pub sqrt_price_limit_x64: u128,
    pub is_base_input: bool,
}

impl RaydiumClmmParams {
    pub fn new(
        amm_config: Pubkey,
        pool: Pubkey,
        input_token_vault: Pubkey,
        output_token_vault: Pubkey,
        observation_key: Pubkey,
        tick_arrays: Vec<Pubkey>,
        other_amount_threshold: u64,
        sqrt_price_limit_x64: u128,
        is_base_input: bool,
    ) -> Self {
        Self {
            amm_config,
            pool,
            input_token_vault,
            output_token_vault,
            observation_key,
            tick_arrays,
            other_amount_threshold,
            sqrt_price_limit_x64,
            is_base_input,
        }
    }
}

impl ProtocolParams for RaydiumClmmParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}