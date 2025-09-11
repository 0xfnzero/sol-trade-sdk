use super::traits::ProtocolParams;
use crate::common::bonding_curve::BondingCurveAccount;
use crate::common::{PriorityFee, SolanaRpcClient};
use crate::solana_streamer_sdk::streaming::event_parser::common::EventType;
use crate::solana_streamer_sdk::streaming::event_parser::protocols::bonk::BonkTradeEvent;
use crate::swqos::SwqosClient;
use crate::trading::common::get_multi_token_balances;
use crate::trading::MiddlewareManager;
use solana_hash::Hash;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::PumpFunTradeEvent;
use solana_streamer_sdk::streaming::event_parser::protocols::pumpswap::{
    PumpSwapBuyEvent, PumpSwapSellEvent,
};
use solana_streamer_sdk::streaming::event_parser::protocols::raydium_amm_v4::types::AmmInfo;
use solana_streamer_sdk::streaming::event_parser::protocols::raydium_cpmm::RaydiumCpmmSwapEvent;
use std::sync::Arc;
/// Buy parameters
#[derive(Clone)]
pub struct BuyParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub slippage_basis_points: Option<u64>,
    pub priority_fee: Arc<PriorityFee>,
    pub lookup_table_key: Option<Pubkey>,
    pub recent_blockhash: Hash,
    pub data_size_limit: u32,
    pub wait_transaction_confirmed: bool,
    pub protocol_params: Box<dyn ProtocolParams>,
    pub open_seed_optimize: bool,
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    pub create_wsol_ata: bool,
    pub close_wsol_ata: bool,
    pub create_mint_ata: bool,
}

/// Sell parameters
#[derive(Clone)]
pub struct SellParams {
    pub rpc: Option<Arc<SolanaRpcClient>>,
    pub payer: Arc<Keypair>,
    pub mint: Pubkey,
    pub token_amount: Option<u64>,
    pub slippage_basis_points: Option<u64>,
    pub priority_fee: Arc<PriorityFee>,
    pub lookup_table_key: Option<Pubkey>,
    pub recent_blockhash: Hash,
    pub wait_transaction_confirmed: bool,
    pub with_tip: bool,
    pub protocol_params: Box<dyn ProtocolParams>,
    pub open_seed_optimize: bool,
    pub swqos_clients: Vec<Arc<SwqosClient>>,
    pub middleware_manager: Option<Arc<MiddlewareManager>>,
    pub create_wsol_ata: bool,
    pub close_wsol_ata: bool,
}

impl std::fmt::Debug for BuyParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BuyParams: {:?}", self)
    }
}

impl std::fmt::Debug for SellParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SellParams: {:?}", self)
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
        event: &PumpFunTradeEvent,
        close_token_account_when_sell: Option<bool>,
    ) -> Self {
        let bonding_curve = BondingCurveAccount::from_dev_trade(
            &event.mint,
            event.token_amount,
            event.max_sol_cost,
            event.creator,
        );
        Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: event.associated_bonding_curve,
            creator_vault: event.creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
        }
    }

    pub fn from_trade(
        event: &PumpFunTradeEvent,
        close_token_account_when_sell: Option<bool>,
    ) -> Self {
        let bonding_curve = BondingCurveAccount::from_trade(event);
        Self {
            bonding_curve: Arc::new(bonding_curve),
            associated_bonding_curve: event.associated_bonding_curve,
            creator_vault: event.creator_vault,
            close_token_account_when_sell: close_token_account_when_sell,
        }
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
}

impl PumpSwapParams {
    pub fn from_buy_trade(event: &PumpSwapBuyEvent) -> Self {
        Self {
            pool: event.pool,
            base_mint: event.base_mint,
            quote_mint: event.quote_mint,
            pool_base_token_account: event.pool_base_token_account,
            pool_quote_token_account: event.pool_quote_token_account,
            pool_base_token_reserves: event.pool_base_token_reserves,
            pool_quote_token_reserves: event.pool_quote_token_reserves,
            coin_creator_vault_ata: event.coin_creator_vault_ata,
            coin_creator_vault_authority: event.coin_creator_vault_authority,
            base_token_program: event.base_token_program,
            quote_token_program: event.quote_token_program,
        }
    }

    pub fn from_sell_trade(event: &PumpSwapSellEvent) -> Self {
        Self {
            pool: event.pool,
            base_mint: event.base_mint,
            quote_mint: event.quote_mint,
            pool_base_token_account: event.pool_base_token_account,
            pool_quote_token_account: event.pool_quote_token_account,
            pool_base_token_reserves: event.pool_base_token_reserves,
            pool_quote_token_reserves: event.pool_quote_token_reserves,
            coin_creator_vault_ata: event.coin_creator_vault_ata,
            coin_creator_vault_authority: event.coin_creator_vault_authority,
            base_token_program: event.base_token_program,
            quote_token_program: event.quote_token_program,
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

        let base_token_program_ata =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool_address,
                &pool_data.base_mint,
                &crate::constants::TOKEN_PROGRAM,
            );
        let quote_token_program_ata =
            spl_associated_token_account::get_associated_token_address_with_program_id(
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
    /// Specifies the program used by the token, usually spl_token::ID or spl_token_2022::ID
    pub mint_token_program: Pubkey,
    pub platform_config: Pubkey,
    pub platform_associated_account: Pubkey,
    pub creator_associated_account: Pubkey,
}

impl BonkParams {
    pub fn immediate_sell(
        mint_token_program: Pubkey,
        platform_config: Pubkey,
        platform_associated_account: Pubkey,
        creator_associated_account: Pubkey,
    ) -> Self {
        Self {
            mint_token_program,
            platform_config,
            platform_associated_account,
            creator_associated_account,
            ..Default::default()
        }
    }
    pub fn from_trade(trade_info: BonkTradeEvent) -> Self {
        Self {
            virtual_base: trade_info.virtual_base as u128,
            virtual_quote: trade_info.virtual_quote as u128,
            real_base: trade_info.real_base_after as u128,
            real_quote: trade_info.real_quote_after as u128,
            pool_state: trade_info.pool_state,
            base_vault: trade_info.base_vault,
            quote_vault: trade_info.quote_vault,
            mint_token_program: trade_info.base_token_program,
            platform_config: trade_info.platform_config,
            platform_associated_account: trade_info.platform_associated_account,
            creator_associated_account: trade_info.creator_associated_account,
        }
    }

    pub fn from_dev_trade(trade_info: BonkTradeEvent) -> Self {
        const DEFAULT_VIRTUAL_BASE: u128 = 1073025605596382;
        const DEFAULT_VIRTUAL_QUOTE: u128 = 30000852951;
        let amount_in = if trade_info.metadata.event_type == EventType::BonkBuyExactIn {
            trade_info.amount_in
        } else {
            crate::instruction::utils::bonk::get_amount_in(
                trade_info.amount_out,
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
        let amount_out = if trade_info.metadata.event_type == EventType::BonkBuyExactIn {
            crate::instruction::utils::bonk::get_amount_out(
                trade_info.amount_in,
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
            trade_info.amount_out as u128
        };
        let real_base = amount_out;
        Self {
            virtual_base: DEFAULT_VIRTUAL_BASE,
            virtual_quote: DEFAULT_VIRTUAL_QUOTE,
            real_base: real_base,
            real_quote: real_quote,
            pool_state: trade_info.pool_state,
            base_vault: trade_info.base_vault,
            quote_vault: trade_info.quote_vault,
            mint_token_program: trade_info.base_token_program,
            platform_config: trade_info.platform_config,
            platform_associated_account: trade_info.platform_associated_account,
            creator_associated_account: trade_info.creator_associated_account,
        }
    }

    pub async fn from_mint_by_rpc(
        rpc: &SolanaRpcClient,
        mint: &Pubkey,
    ) -> Result<Self, anyhow::Error> {
        let pool_address = crate::instruction::utils::bonk::get_pool_pda(
            mint,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
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
    /// Base token program ID (usually spl_token::ID or spl_token_2022::ID)
    pub base_token_program: Pubkey,
    /// Quote token program ID (usually spl_token::ID or spl_token_2022::ID)
    pub quote_token_program: Pubkey,
    /// Observation state account
    pub observation_state: Pubkey,
}

impl RaydiumCpmmParams {
    pub fn from_trade(
        trade_info: RaydiumCpmmSwapEvent,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Self {
        Self {
            pool_state: trade_info.pool_state,
            amm_config: trade_info.amm_config,
            base_mint: trade_info.input_token_mint,
            quote_mint: trade_info.output_token_mint,
            base_reserve: base_reserve,
            quote_reserve: quote_reserve,
            base_vault: trade_info.input_vault,
            quote_vault: trade_info.output_vault,
            base_token_program: trade_info.input_token_program,
            quote_token_program: trade_info.output_token_program,
            observation_state: trade_info.observation_state,
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
    pub fn from_amm_info_and_reserves(
        amm: Pubkey,
        amm_info: AmmInfo,
        coin_reserve: u64,
        pc_reserve: u64,
    ) -> Self {
        Self {
            amm,
            coin_mint: amm_info.coin_mint,
            pc_mint: amm_info.pc_mint,
            token_coin: amm_info.token_coin,
            token_pc: amm_info.token_pc,
            coin_reserve,
            pc_reserve,
        }
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

impl ProtocolParams for RaydiumAmmV4Params {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}
