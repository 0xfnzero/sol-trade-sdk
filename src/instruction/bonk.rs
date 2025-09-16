use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::bonk::{
        accounts, get_pool_pda, get_vault_pda, BUY_EXECT_IN_DISCRIMINATOR,
        SELL_EXECT_IN_DISCRIMINATOR,
    },
    trading::{
        common::utils::get_token_balance,
        core::{
            params::{BonkParams, InternalBuyParams, InternalSellParams},
            traits::InstructionBuilder,
        },
    },
    utils::calc::bonk::{
        get_buy_token_amount_from_sol_amount, get_sell_sol_amount_from_token_amount,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for Bonk protocol
pub struct BonkInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for BonkInstructionBuilder {
    async fn build_buy_instructions(&self, params: &InternalBuyParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<BonkParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Bonk"))?;

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(&params.mint, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
        } else {
            protocol_params.pool_state
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.sol_amount;
        let share_fee_rate: u64 = 0;
        let minimum_amount_out: u64 = get_buy_token_amount_from_sol_amount(
            amount_in,
            protocol_params.virtual_base,
            protocol_params.virtual_quote,
            protocol_params.real_base,
            protocol_params.real_quote,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE) as u128,
        );

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &protocol_params.mint_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        let base_vault_account = if protocol_params.base_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &params.mint).unwrap()
        } else {
            protocol_params.base_vault
        };
        let quote_vault_account = if protocol_params.quote_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
        } else {
            protocol_params.quote_vault
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_wsol_ata {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), amount_in));
        }

        if params.create_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &params.mint,
                    &protocol_params.mint_token_program,
                    params.open_seed_optimize,
                ),
            );
        }

        let mut data = [0u8; 32];
        data[..8].copy_from_slice(&BUY_EXECT_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        data[24..32].copy_from_slice(&share_fee_rate.to_le_bytes());

        let accounts: [AccountMeta; 18] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            accounts::GLOBAL_CONFIG_META,                  // Global Config (readonly)
            AccountMeta::new_readonly(protocol_params.platform_config, false), // Platform Config (readonly)
            AccountMeta::new(pool_state, false),                               // Pool State
            AccountMeta::new(user_base_token_account, false),                  // User Base Token
            AccountMeta::new(user_quote_token_account, false),                 // User Quote Token
            AccountMeta::new(base_vault_account, false),                       // Base Vault
            AccountMeta::new(quote_vault_account, false),                      // Quote Vault
            AccountMeta::new_readonly(params.mint, false), // Base Token Mint (readonly)
            crate::constants::WSOL_TOKEN_ACCOUNT_META,     // Quote Token Mint (readonly)
            AccountMeta::new_readonly(protocol_params.mint_token_program, false), // Base Token Program (readonly)
            crate::constants::TOKEN_PROGRAM_META, // Quote Token Program (readonly)
            accounts::EVENT_AUTHORITY_META,       // Event Authority (readonly)
            accounts::BONK_META,                  // Program (readonly)
            crate::constants::SYSTEM_PROGRAM_META, // System Program (readonly)
            AccountMeta::new(protocol_params.platform_associated_account, false), // Platform Associated Account
            AccountMeta::new(protocol_params.creator_associated_account, false), // Creator Associated Account
        ];

        instructions.push(Instruction::new_with_bytes(accounts::BONK, &data, accounts.to_vec()));

        if params.close_wsol_ata {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &InternalSellParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<BonkParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Bonk"))?;

        let rpc = params.rpc.as_ref().unwrap().clone();

        let mut amount = params.token_amount;
        if params.token_amount.is_none() || params.token_amount.unwrap_or(0) == 0 {
            let balance_u64 =
                get_token_balance(rpc.as_ref(), &params.payer.pubkey(), &params.mint).await?;
            amount = Some(balance_u64);
        }
        let amount = amount.unwrap_or(0);

        if amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(&params.mint, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
        } else {
            protocol_params.pool_state
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let share_fee_rate: u64 = 0;
        let minimum_amount_out: u64 = get_sell_sol_amount_from_token_amount(
            amount,
            protocol_params.virtual_base,
            protocol_params.virtual_quote,
            protocol_params.real_base,
            protocol_params.real_quote,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE) as u128,
        );

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &protocol_params.mint_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        let base_vault_account = if protocol_params.base_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &params.mint).unwrap()
        } else {
            protocol_params.base_vault
        };
        let quote_vault_account = if protocol_params.quote_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
        } else {
            protocol_params.quote_vault
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(3);

        if params.create_wsol_ata {
            instructions.extend(crate::trading::common::create_wsol_ata(&params.payer.pubkey()));
        }

        let mut data = [0u8; 32];
        data[..8].copy_from_slice(&SELL_EXECT_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        data[24..32].copy_from_slice(&share_fee_rate.to_le_bytes());

        let accounts: [AccountMeta; 18] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            accounts::GLOBAL_CONFIG_META,                  // Global Config (readonly)
            AccountMeta::new_readonly(protocol_params.platform_config, false), // Platform Config (readonly)
            AccountMeta::new(pool_state, false),                               // Pool State
            AccountMeta::new(user_base_token_account, false),                  // User Base Token
            AccountMeta::new(user_quote_token_account, false),                 // User Quote Token
            AccountMeta::new(base_vault_account, false),                       // Base Vault
            AccountMeta::new(quote_vault_account, false),                      // Quote Vault
            AccountMeta::new_readonly(params.mint, false), // Base Token Mint (readonly)
            crate::constants::WSOL_TOKEN_ACCOUNT_META,     // Quote Token Mint (readonly)
            AccountMeta::new_readonly(protocol_params.mint_token_program, false), // Base Token Program (readonly)
            crate::constants::TOKEN_PROGRAM_META, // Quote Token Program (readonly)
            accounts::EVENT_AUTHORITY_META,       // Event Authority (readonly)
            accounts::BONK_META,                  // Program (readonly)
            crate::constants::SYSTEM_PROGRAM_META, // System Program (readonly)
            AccountMeta::new(protocol_params.platform_associated_account, false), // Platform Associated Account
            AccountMeta::new(protocol_params.creator_associated_account, false), // Creator Associated Account
        ];

        instructions.push(Instruction::new_with_bytes(accounts::BONK, &data, accounts.to_vec()));

        if params.close_wsol_ata {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }
}
