use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::bonk::{
            accounts, get_pool_pda, get_vault_pda, BUY_EXECT_IN_DISCRIMINATOR,
            BUY_EXECT_OUT_DISCRIMINATOR, SELL_EXECT_IN_DISCRIMINATOR, SELL_EXECT_OUT_DISCRIMINATOR,
        },
    },
    trading::core::{
        params::{BonkParams, SwapParams},
        traits::InstructionBuilder,
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
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<BonkParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Bonk"))?;

        let usd1_pool = protocol_params.global_config == accounts::USD1_GLOBAL_CONFIG;

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            if usd1_pool {
                get_pool_pda(&params.output_mint, &crate::constants::USD1_TOKEN_ACCOUNT).unwrap()
            } else {
                get_pool_pda(&params.output_mint, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
            }
        } else {
            protocol_params.pool_state
        };

        let global_config = if usd1_pool {
            accounts::USD1_GLOBAL_CONFIG_META
        } else {
            accounts::GLOBAL_CONFIG_META
        };

        let quote_mint = if usd1_pool {
            crate::constants::USD1_TOKEN_ACCOUNT
        } else {
            crate::constants::WSOL_TOKEN_ACCOUNT
        };
        let quote_token_mint = if usd1_pool {
            crate::constants::USD1_TOKEN_ACCOUNT_META
        } else {
            crate::constants::WSOL_TOKEN_ACCOUNT_META
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let share_fee_rate: u64 = 0;
        let minimum_amount_out: u64 = match params.fixed_output_amount {
            Some(fixed_amount) => fixed_amount,
            None => get_buy_token_amount_from_sol_amount(
                amount_in,
                protocol_params.virtual_base,
                protocol_params.virtual_quote,
                protocol_params.real_base,
                protocol_params.real_quote,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE) as u128,
            ),
        };

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                &protocol_params.mint_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        let base_vault_account = if protocol_params.base_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &params.output_mint).unwrap()
        } else {
            protocol_params.base_vault
        };
        let quote_vault_account = if protocol_params.quote_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &quote_mint).unwrap()
        } else {
            protocol_params.quote_vault
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_input_mint_ata {
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
                amount_in,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &params.output_mint,
                    &protocol_params.mint_token_program,
                    params.open_seed_optimize,
                ),
            );
        }

        let mut data = [0u8; 32];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&BUY_EXECT_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_out.to_le_bytes());
            data[16..24].copy_from_slice(&amount_in.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&BUY_EXECT_IN_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }
        data[24..32].copy_from_slice(&share_fee_rate.to_le_bytes());

        let accounts: [AccountMeta; 15] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            global_config,                                 // Global Config (readonly)
            AccountMeta::new_readonly(protocol_params.platform_config, false), // Platform Config (readonly)
            AccountMeta::new(pool_state, false),                               // Pool State
            AccountMeta::new(user_base_token_account, false),                  // User Base Token
            AccountMeta::new(user_quote_token_account, false),                 // User Quote Token
            AccountMeta::new(base_vault_account, false),                       // Base Vault
            AccountMeta::new(quote_vault_account, false),                      // Quote Vault
            AccountMeta::new_readonly(params.output_mint, false), // Base Token Mint (readonly)
            quote_token_mint,                                     // Quote Token Mint (readonly)
            AccountMeta::new_readonly(protocol_params.mint_token_program, false), // Base Token Program (readonly)
            crate::constants::TOKEN_PROGRAM_META, // Quote Token Program (readonly)
            accounts::EVENT_AUTHORITY_META,       // Event Authority (readonly)
            accounts::BONK_META,                  // Program (readonly)
        ];

        instructions.push(Instruction::new_with_bytes(accounts::BONK, &data, accounts.to_vec()));

        if params.close_input_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &quote_mint);
        }

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let amount = params
            .input_amount
            .filter(|&a| a > 0)
            .ok_or_else(|| anyhow!("Bonk sell requires input_amount (token amount to sell); fetch balance via RPC before calling build_sell"))?;

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<BonkParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Bonk"))?;

        let usd1_pool = protocol_params.global_config == accounts::USD1_GLOBAL_CONFIG;

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            if usd1_pool {
                get_pool_pda(&params.input_mint, &crate::constants::USD1_TOKEN_ACCOUNT).unwrap()
            } else {
                get_pool_pda(&params.input_mint, &crate::constants::WSOL_TOKEN_ACCOUNT).unwrap()
            }
        } else {
            protocol_params.pool_state
        };

        let global_config = if usd1_pool {
            accounts::USD1_GLOBAL_CONFIG_META
        } else {
            accounts::GLOBAL_CONFIG_META
        };

        let quote_mint = if usd1_pool {
            crate::constants::USD1_TOKEN_ACCOUNT
        } else {
            crate::constants::WSOL_TOKEN_ACCOUNT
        };
        let quote_token_mint = if usd1_pool {
            crate::constants::USD1_TOKEN_ACCOUNT_META
        } else {
            crate::constants::WSOL_TOKEN_ACCOUNT_META
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let share_fee_rate: u64 = 0;
        let minimum_amount_out: u64 = match params.fixed_output_amount {
            Some(fixed_amount) => fixed_amount,
            None => get_sell_sol_amount_from_token_amount(
                amount,
                protocol_params.virtual_base,
                protocol_params.virtual_quote,
                protocol_params.real_base,
                protocol_params.real_quote,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE) as u128,
            ),
        };

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                &protocol_params.mint_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        let base_vault_account = if protocol_params.base_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &params.input_mint).unwrap()
        } else {
            protocol_params.base_vault
        };
        let quote_vault_account = if protocol_params.quote_vault == Pubkey::default() {
            get_vault_pda(&pool_state, &quote_mint).unwrap()
        } else {
            protocol_params.quote_vault
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(4);

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        }

        let mut data = [0u8; 32];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&SELL_EXECT_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_out.to_le_bytes());
            data[16..24].copy_from_slice(&amount.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&SELL_EXECT_IN_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount.to_le_bytes());
            data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }
        data[24..32].copy_from_slice(&share_fee_rate.to_le_bytes());

        let accounts: [AccountMeta; 15] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            global_config,                                 // Global Config (readonly)
            AccountMeta::new_readonly(protocol_params.platform_config, false), // Platform Config (readonly)
            AccountMeta::new(pool_state, false),                               // Pool State
            AccountMeta::new(user_base_token_account, false),                  // User Base Token
            AccountMeta::new(user_quote_token_account, false),                 // User Quote Token
            AccountMeta::new(base_vault_account, false),                       // Base Vault
            AccountMeta::new(quote_vault_account, false),                      // Quote Vault
            AccountMeta::new_readonly(params.input_mint, false), // Base Token Mint (readonly)
            quote_token_mint,                                    // Quote Token Mint (readonly)
            AccountMeta::new_readonly(protocol_params.mint_token_program, false), // Base Token Program (readonly)
            crate::constants::TOKEN_PROGRAM_META, // Quote Token Program (readonly)
            accounts::EVENT_AUTHORITY_META,       // Event Authority (readonly)
            accounts::BONK_META,                  // Program (readonly)
        ];

        instructions.push(Instruction::new_with_bytes(accounts::BONK, &data, accounts.to_vec()));

        if params.close_output_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &quote_mint);
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &protocol_params.mint_token_program,
                &user_base_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }

        Ok(instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{common::GasFeeStrategy, swqos::TradeType, trading::core::params::DexParamEnum};
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn bonk_params() -> BonkParams {
        BonkParams {
            mint_token_program: crate::constants::TOKEN_PROGRAM,
            platform_config: pk(8),
            platform_associated_account: pk(9),
            creator_associated_account: pk(10),
            global_config: accounts::GLOBAL_CONFIG,
            ..Default::default()
        }
    }

    fn swap_params(trade_type: TradeType) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type,
            input_mint: pk(3),
            input_token_program: None,
            output_mint: pk(3),
            output_token_program: None,
            input_amount: Some(100_000),
            slippage_basis_points: Some(100),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::Bonk(bonk_params()),
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: false,
            close_output_mint_ata: false,
            fixed_output_amount: Some(42),
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
        }
    }

    #[tokio::test]
    async fn bonk_buy_uses_exact_out_when_fixed_output_is_set() {
        let instructions = BonkInstructionBuilder
            .build_buy_instructions(&swap_params(TradeType::Buy))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 15);
        assert_eq!(ix.accounts[14].pubkey, accounts::BONK);
        assert_eq!(&ix.data[..8], BUY_EXECT_OUT_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 100_000);
    }

    #[tokio::test]
    async fn bonk_sell_uses_exact_out_when_fixed_output_is_set() {
        let instructions = BonkInstructionBuilder
            .build_sell_instructions(&swap_params(TradeType::Sell))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 15);
        assert_eq!(ix.accounts[14].pubkey, accounts::BONK);
        assert_eq!(&ix.data[..8], SELL_EXECT_OUT_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 100_000);
    }

    #[tokio::test]
    async fn bonk_usd1_buy_create_input_builds_usd1_ata_not_wsol_wrap() {
        let mut params = swap_params(TradeType::Buy);
        if let DexParamEnum::Bonk(protocol_params) = &mut params.protocol_params {
            protocol_params.global_config = accounts::USD1_GLOBAL_CONFIG;
        }
        params.create_input_mint_ata = true;
        params.open_seed_optimize = false;

        let instructions = BonkInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let create_ix = instructions.first().unwrap();

        assert_eq!(create_ix.program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(create_ix.accounts[3].pubkey, crate::constants::USD1_TOKEN_ACCOUNT);
    }
}
