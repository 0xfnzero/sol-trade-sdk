use crate::{
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::meteora_damm_v2::{
            accounts, get_event_authority_pda, SWAP2_DISCRIMINATOR, SWAP_MODE_EXACT_IN,
            SWAP_MODE_EXACT_OUT, SWAP_MODE_PARTIAL_FILL,
        },
    },
    trading::core::{
        params::{MeteoraDammV2Params, SwapParams},
        traits::InstructionBuilder,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct MeteoraDammV2InstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for MeteoraDammV2InstructionBuilder {
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
            .downcast_ref::<MeteoraDammV2Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDammV2"))?;

        let is_wsol = protocol_params.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let is_usdc = protocol_params.token_a_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.token_b_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let requested_input = if params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT {
            crate::constants::WSOL_TOKEN_ACCOUNT
        } else {
            params.input_mint
        };
        let requested_output = if params.output_mint == crate::constants::SOL_TOKEN_ACCOUNT {
            crate::constants::WSOL_TOKEN_ACCOUNT
        } else {
            params.output_mint
        };
        let is_a_in = requested_input == protocol_params.token_a_mint;
        if !((is_a_in && requested_output == protocol_params.token_b_mint)
            || (requested_input == protocol_params.token_b_mint
                && requested_output == protocol_params.token_a_mint))
        {
            return Err(anyhow!("Input/output mints must match opposite DAMM v2 pool sides"));
        }
        let input_mint =
            if is_a_in { protocol_params.token_a_mint } else { protocol_params.token_b_mint };
        let input_token_program =
            if is_a_in { protocol_params.token_a_program } else { protocol_params.token_b_program };
        let output_mint =
            if is_a_in { protocol_params.token_b_mint } else { protocol_params.token_a_mint };
        let output_token_program =
            if is_a_in { protocol_params.token_b_program } else { protocol_params.token_a_program };
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let (amount_0, amount_1) = match protocol_params.swap_mode {
            SWAP_MODE_EXACT_OUT => {
                let amount_out = params.fixed_output_amount.ok_or_else(|| {
                    anyhow!("fixed_output_amount must be set for MeteoraDammV2 exact-out swap2")
                })?;
                (amount_out, amount_in)
            }
            SWAP_MODE_EXACT_IN | SWAP_MODE_PARTIAL_FILL => {
                let minimum_amount_out = params.fixed_output_amount.ok_or_else(|| {
                    anyhow!("fixed_output_amount must be set for MeteoraDammV2 swap2 min output")
                })?;
                (amount_in, minimum_amount_out)
            }
            mode => return Err(anyhow!("Unsupported MeteoraDammV2 swap_mode {}", mode)),
        };

        let input_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &input_mint,
                &input_token_program,
                params.open_seed_optimize,
            );
        let output_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_input_mint_ata {
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &input_mint,
                &input_token_program,
                amount_in,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        //
        // cp-amm swap2 (pinocchio path) always expects a fixed referral slot.
        // When there is no referral, Anchor/optional encoding uses the program
        // id as the None placeholder; omitting it shifts event_authority and
        // triggers ConstraintSeeds (2006).
        let mut account_metas =
            Vec::with_capacity(14 + usize::from(protocol_params.include_rate_limiter_sysvar));
        account_metas.extend([
            accounts::AUTHORITY_META,                      // Pool Authority (readonly)
            AccountMeta::new(protocol_params.pool, false), // Pool
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(protocol_params.token_a_vault, false), // Token A Vault
            AccountMeta::new(protocol_params.token_b_vault, false), // Token B Vault
            AccountMeta::new_readonly(protocol_params.token_a_mint, false), // Token A Mint (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_mint, false), // Token B Mint (readonly)
            AccountMeta::new_readonly(params.payer.pubkey(), true), // User Transfer Authority (IDL: signer only)
            AccountMeta::new_readonly(protocol_params.token_a_program, false), // Token Program (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_program, false), // Token Program (readonly)
            match protocol_params.referral_token_account {
                Some(referral) => AccountMeta::new(referral, false),
                None => accounts::METEORA_DAMM_V2_META,
            },
            AccountMeta::new_readonly(get_event_authority_pda(), false), // Event Authority (readonly)
            accounts::METEORA_DAMM_V2_META,                              // Program (readonly)
        ]);
        if protocol_params.include_rate_limiter_sysvar {
            account_metas.push(accounts::SYSVAR_INSTRUCTIONS_META);
        }
        // Create instruction data
        let mut data = [0u8; 25];
        data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_0.to_le_bytes());
        data[16..24].copy_from_slice(&amount_1.to_le_bytes());
        data[24] = protocol_params.swap_mode;

        instructions.push(Instruction::new_with_bytes(
            accounts::METEORA_DAMM_V2,
            &data,
            account_metas,
        ));

        if params.close_input_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &input_mint);
        }

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<MeteoraDammV2Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDammV2"))?;

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let is_wsol = protocol_params.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let is_usdc = protocol_params.token_b_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.token_a_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let requested_input = if params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT {
            crate::constants::WSOL_TOKEN_ACCOUNT
        } else {
            params.input_mint
        };
        let requested_output = if params.output_mint == crate::constants::SOL_TOKEN_ACCOUNT {
            crate::constants::WSOL_TOKEN_ACCOUNT
        } else {
            params.output_mint
        };
        let is_a_in = requested_input == protocol_params.token_a_mint;
        if !((is_a_in && requested_output == protocol_params.token_b_mint)
            || (requested_input == protocol_params.token_b_mint
                && requested_output == protocol_params.token_a_mint))
        {
            return Err(anyhow!("Input/output mints must match opposite DAMM v2 pool sides"));
        }
        let input_mint =
            if is_a_in { protocol_params.token_a_mint } else { protocol_params.token_b_mint };
        let input_token_program =
            if is_a_in { protocol_params.token_a_program } else { protocol_params.token_b_program };
        let output_mint =
            if is_a_in { protocol_params.token_b_mint } else { protocol_params.token_a_mint };
        let output_token_program =
            if is_a_in { protocol_params.token_b_program } else { protocol_params.token_a_program };
        let amount_in = params.input_amount.unwrap_or(0);
        let (amount_0, amount_1) = match protocol_params.swap_mode {
            SWAP_MODE_EXACT_OUT => {
                let amount_out = params.fixed_output_amount.ok_or_else(|| {
                    anyhow!("fixed_output_amount must be set for MeteoraDammV2 exact-out swap2")
                })?;
                (amount_out, amount_in)
            }
            SWAP_MODE_EXACT_IN | SWAP_MODE_PARTIAL_FILL => {
                let minimum_amount_out = params.fixed_output_amount.ok_or_else(|| {
                    anyhow!("fixed_output_amount must be set for MeteoraDammV2 swap2 min output")
                })?;
                (amount_in, minimum_amount_out)
            }
            mode => return Err(anyhow!("Unsupported MeteoraDammV2 swap_mode {}", mode)),
        };

        let input_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &input_mint,
                &input_token_program,
                params.open_seed_optimize,
            );
        let output_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(4);

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );
        }

        // Same fixed referral slot rule as buy (see comment there).
        let mut account_metas =
            Vec::with_capacity(14 + usize::from(protocol_params.include_rate_limiter_sysvar));
        account_metas.extend([
            accounts::AUTHORITY_META,                      // Pool Authority (readonly)
            AccountMeta::new(protocol_params.pool, false), // Pool
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(protocol_params.token_a_vault, false), // Token A Vault
            AccountMeta::new(protocol_params.token_b_vault, false), // Token B Vault
            AccountMeta::new_readonly(protocol_params.token_a_mint, false), // Token A Mint (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_mint, false), // Token B Mint (readonly)
            AccountMeta::new_readonly(params.payer.pubkey(), true), // User Transfer Authority (IDL: signer only)
            AccountMeta::new_readonly(protocol_params.token_a_program, false), // Token Program (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_program, false), // Token Program (readonly)
            match protocol_params.referral_token_account {
                Some(referral) => AccountMeta::new(referral, false),
                None => accounts::METEORA_DAMM_V2_META,
            },
            AccountMeta::new_readonly(get_event_authority_pda(), false), // Event Authority (readonly)
            accounts::METEORA_DAMM_V2_META,                              // Program (readonly)
        ]);
        if protocol_params.include_rate_limiter_sysvar {
            account_metas.push(accounts::SYSVAR_INSTRUCTIONS_META);
        }
        // Create instruction data
        let mut data = [0u8; 25];
        data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_0.to_le_bytes());
        data[16..24].copy_from_slice(&amount_1.to_le_bytes());
        data[24] = protocol_params.swap_mode;

        instructions.push(Instruction::new_with_bytes(
            accounts::METEORA_DAMM_V2,
            &data,
            account_metas,
        ));

        if params.close_output_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &output_mint);
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &input_token_program,
                &input_token_account,
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
    use crate::{
        common::GasFeeStrategy,
        swqos::TradeType,
        trading::core::params::{DexParamEnum, SwapParams},
    };
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn meteora_params(referral: Option<Pubkey>) -> MeteoraDammV2Params {
        let params = MeteoraDammV2Params::new(
            pk(1),
            pk(2),
            pk(3),
            crate::constants::WSOL_TOKEN_ACCOUNT,
            pk(4),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
        );
        match referral {
            Some(account) => params.with_referral_token_account(account),
            None => params,
        }
    }

    fn swap_params(protocol_params: MeteoraDammV2Params) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type: TradeType::Buy,
            input_mint: crate::constants::WSOL_TOKEN_ACCOUNT,
            input_token_program: None,
            output_mint: pk(4),
            output_token_program: None,
            input_amount: Some(100_000),
            slippage_basis_points: Some(100),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::MeteoraDammV2(protocol_params),
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: false,
            close_output_mint_ata: false,
            fixed_output_amount: Some(1),
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            transaction_version: crate::common::TradeTransactionVersion::V0,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
        }
    }

    #[tokio::test]
    async fn sol_usdc_direction_follows_explicit_mints_on_both_pool_orderings() {
        use crate::common::fast_fn::get_associated_token_address_with_program_id_fast;
        for (a, b) in [
            (crate::constants::WSOL_TOKEN_ACCOUNT, crate::constants::USDC_TOKEN_ACCOUNT),
            (crate::constants::USDC_TOKEN_ACCOUNT, crate::constants::WSOL_TOKEN_ACCOUNT),
        ] {
            let pool = MeteoraDammV2Params::new(
                pk(1),
                pk(2),
                pk(3),
                a,
                b,
                crate::constants::TOKEN_PROGRAM,
                crate::constants::TOKEN_PROGRAM,
            );
            for sell in [false, true] {
                let mut params = swap_params(pool.clone());
                params.open_seed_optimize = false;
                params.input_mint = if sell {
                    crate::constants::USDC_TOKEN_ACCOUNT
                } else {
                    crate::constants::WSOL_TOKEN_ACCOUNT
                };
                params.output_mint = if sell {
                    crate::constants::WSOL_TOKEN_ACCOUNT
                } else {
                    crate::constants::USDC_TOKEN_ACCOUNT
                };
                let builder = MeteoraDammV2InstructionBuilder;
                let ixs = if sell {
                    builder.build_sell_instructions(&params).await
                } else {
                    builder.build_buy_instructions(&params).await
                }
                .unwrap();
                let ix = ixs.last().unwrap();
                assert_eq!(
                    ix.accounts[2].pubkey,
                    get_associated_token_address_with_program_id_fast(
                        &params.payer.pubkey(),
                        &params.input_mint,
                        &crate::constants::TOKEN_PROGRAM
                    )
                );
                assert_eq!(
                    ix.accounts[3].pubkey,
                    get_associated_token_address_with_program_id_fast(
                        &params.payer.pubkey(),
                        &params.output_mint,
                        &crate::constants::TOKEN_PROGRAM
                    )
                );
                params.output_mint = params.input_mint;
                assert!(builder.build_buy_instructions(&params).await.is_err());
                assert!(builder.build_sell_instructions(&params).await.is_err());
            }
        }
    }

    #[tokio::test]
    async fn meteora_uses_program_id_placeholder_when_referral_absent() {
        let instructions = MeteoraDammV2InstructionBuilder
            .build_buy_instructions(&swap_params(meteora_params(None)))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 14);
        assert_eq!(ix.accounts[11].pubkey, accounts::METEORA_DAMM_V2);
        assert!(!ix.accounts[11].is_writable);
        assert_eq!(ix.accounts[12].pubkey, get_event_authority_pda());
        assert_eq!(ix.accounts[13].pubkey, accounts::METEORA_DAMM_V2);
        assert_eq!(&ix.data[..8], SWAP2_DISCRIMINATOR);
        assert_eq!(ix.data[24], SWAP_MODE_PARTIAL_FILL);
    }

    #[tokio::test]
    async fn meteora_includes_writable_referral_account_when_set() {
        let referral = pk(9);
        let instructions = MeteoraDammV2InstructionBuilder
            .build_buy_instructions(&swap_params(meteora_params(Some(referral))))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 14);
        assert_eq!(ix.accounts[11].pubkey, referral);
        assert!(ix.accounts[11].is_writable);
        assert_eq!(ix.accounts[12].pubkey, get_event_authority_pda());
        assert_eq!(ix.accounts[13].pubkey, accounts::METEORA_DAMM_V2);
    }

    #[tokio::test]
    async fn meteora_includes_sysvar_only_when_rate_limiter_is_set() {
        let protocol_params = meteora_params(None).with_rate_limiter_sysvar(true);
        let instructions = MeteoraDammV2InstructionBuilder
            .build_buy_instructions(&swap_params(protocol_params))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 15);
        assert_eq!(ix.accounts[11].pubkey, accounts::METEORA_DAMM_V2);
        assert_eq!(ix.accounts[12].pubkey, get_event_authority_pda());
        assert_eq!(ix.accounts[13].pubkey, accounts::METEORA_DAMM_V2);
        assert_eq!(ix.accounts[14].pubkey, accounts::SYSVAR_INSTRUCTIONS);
    }

    #[tokio::test]
    async fn meteora_swap2_exact_out_uses_amount_out_then_max_input() {
        let protocol_params = meteora_params(None).with_swap_mode(SWAP_MODE_EXACT_OUT);
        let instructions = MeteoraDammV2InstructionBuilder
            .build_buy_instructions(&swap_params(protocol_params))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], SWAP2_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 100_000);
        assert_eq!(ix.data[24], SWAP_MODE_EXACT_OUT);
    }

    #[tokio::test]
    async fn meteora_sol_buy_uses_pool_wsol_mint_for_user_input_account() {
        let mut params = swap_params(meteora_params(None));
        params.input_mint = crate::constants::SOL_TOKEN_ACCOUNT;

        let instructions =
            MeteoraDammV2InstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();
        let expected_wsol_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let wrong_sol_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::SOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        assert_eq!(ix.accounts[2].pubkey, expected_wsol_ata);
        assert_ne!(ix.accounts[2].pubkey, wrong_sol_ata);
    }

    #[tokio::test]
    async fn meteora_buy_create_output_account_uses_pool_output_token_program() {
        let protocol_params = MeteoraDammV2Params::new(
            pk(1),
            pk(2),
            pk(3),
            crate::constants::WSOL_TOKEN_ACCOUNT,
            pk(4),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM_2022,
        );
        let mut params = swap_params(protocol_params);
        params.create_output_mint_ata = true;

        let instructions =
            MeteoraDammV2InstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let expected_output_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &pk(4),
                &crate::constants::TOKEN_PROGRAM_2022,
                params.open_seed_optimize,
            );
        let wrong_output_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &pk(4),
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let swap_ix = instructions.last().unwrap();

        assert_eq!(instructions[0].program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(instructions[0].accounts[5].pubkey, crate::constants::TOKEN_PROGRAM_2022);
        assert_eq!(swap_ix.accounts[3].pubkey, expected_output_ata);
        assert_ne!(swap_ix.accounts[3].pubkey, wrong_output_ata);
        assert_eq!(swap_ix.accounts[10].pubkey, crate::constants::TOKEN_PROGRAM_2022);
    }
}
