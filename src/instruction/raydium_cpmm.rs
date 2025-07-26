use anyhow::{anyhow, Result};
use solana_sdk::{instruction::Instruction, signer::Signer};
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use spl_token::instruction::close_account;

use crate::{
    constants::raydium_cpmm::{accounts, SWAP_BASE_IN_DISCRIMINATOR},
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::common::utils::get_token_balance,
    trading::core::{
        params::{BuyParams, RaydiumCpmmParams, SellParams},
        traits::InstructionBuilder,
    },
    trading::raydium_cpmm::{
        common::{get_observation_state_pda, get_pool_pda, get_vault_pda},
        // pool::Pool,
    },
};

/// RaydiumCpmm协议的指令构建器
/// RaydiumCpmm protocol instruction builder
pub struct RaydiumCpmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumCpmmInstructionBuilder {
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>> {
        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        self.build_buy_instructions_with_accounts(params).await
    }

    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>> {
        self.build_sell_instructions_with_accounts(params).await
    }
}

impl RaydiumCpmmInstructionBuilder {
    /// 使用提供的账户信息构建买入指令
    /// Build buy instructions using provided account information
    async fn build_buy_instructions_with_accounts(
        &self,
        params: &BuyParams,
    ) -> Result<Vec<Instruction>> {
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        let pool_state = if protocol_params.pool_state.is_some() {
            protocol_params.pool_state.unwrap()
        } else {
            let mint_token_in_pool_state_index = protocol_params
                .mint_token_in_pool_state_index
                .unwrap_or(1);
            get_pool_pda(
                &accounts::AMM_CONFIG,
                if mint_token_in_pool_state_index == 1 {
                    &accounts::WSOL_TOKEN_ACCOUNT
                } else {
                    &params.mint
                },
                if mint_token_in_pool_state_index == 1 {
                    &params.mint
                } else {
                    &accounts::WSOL_TOKEN_ACCOUNT
                },
            )
            .unwrap()
        };

        let wsol_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &accounts::WSOL_TOKEN_ACCOUNT,
        );
        let mint_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &params.mint,
        );

        // 获取池的代币账户
        // Get pool token accounts
        let wsol_vault_account = get_vault_pda(&pool_state, &accounts::WSOL_TOKEN_ACCOUNT).unwrap();
        let mint_vault_account = get_vault_pda(&pool_state, &params.mint).unwrap();

        let observation_state_account = get_observation_state_pda(&pool_state).unwrap();

        let amount_in: u64 = params.sol_amount;
        let mut minimum_amount_out: u64 = if protocol_params.minimum_amount_out.is_some() {
            protocol_params.minimum_amount_out.unwrap()
        } else {
            println!("minimum_amount_out not provided, using default value 0");
            // 未提供minimum_amount_out，使用默认值0
            0
        };
        if minimum_amount_out != 0 {
            let slippage_basis_points: u64 = if params.slippage_basis_points.is_some() {
                params.slippage_basis_points.unwrap()
            } else {
                DEFAULT_SLIPPAGE
            } as u64;
            minimum_amount_out = minimum_amount_out * (10000 - slippage_basis_points) / 10000;
            println!("slippage_basis_points: {}", slippage_basis_points);
        }
        println!("minimum_amount_out: {}", minimum_amount_out);

        let mut instructions = vec![];

        if protocol_params.auto_handle_wsol {
            // 插入wsol
            // Insert wSOL
            instructions.push(
                // 创建wSOL ATA账户，如果不存在
                // Create wSOL ATA account if it doesn't exist
                create_associated_token_account_idempotent(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &accounts::WSOL_TOKEN_ACCOUNT,
                    &accounts::TOKEN_PROGRAM,
                ),
            );
            instructions.push(
                // 将SOL转入wSOL ATA账户
                // Transfer SOL to wSOL ATA account
                solana_sdk::system_instruction::transfer(
                    &params.payer.pubkey(),
                    &wsol_token_account,
                    amount_in,
                ),
            );

            // 同步wSOL余额
            // Sync wSOL balance
            instructions.push(
                spl_token::instruction::sync_native(&accounts::TOKEN_PROGRAM, &wsol_token_account)
                    .unwrap(),
            );
        }

        // 创建用户的基础代币账户
        // Create user base token account
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &params.mint,
            &protocol_params
                .mint_token_program
                .unwrap_or(accounts::TOKEN_PROGRAM),
        ));

        // 创建买入指令
        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AUTHORITY, false), // Authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_CONFIG, false), // Amm Config (readonly)
            solana_sdk::instruction::AccountMeta::new(pool_state, false), // Pool State
            solana_sdk::instruction::AccountMeta::new(wsol_token_account, false), // Input Token Account
            solana_sdk::instruction::AccountMeta::new(mint_token_account, false), // Output Token Account
            solana_sdk::instruction::AccountMeta::new(wsol_vault_account, false), // Input Vault Account
            solana_sdk::instruction::AccountMeta::new(mint_vault_account, false), // Output Vault Account
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // Input Token Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(
                protocol_params
                    .mint_token_program
                    .unwrap_or(accounts::TOKEN_PROGRAM),
                false,
            ), // Output Token Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::WSOL_TOKEN_ACCOUNT, false), // Input token mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(params.mint, false), // Output token mint (readonly)
            solana_sdk::instruction::AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // 创建指令数据
        let mut data = vec![];
        data.extend_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::RAYDIUM_CPMM,
            accounts,
            data,
        });

        if protocol_params.auto_handle_wsol {
            // 关闭wSOL ATA账户，回收租金
            // Close wSOL ATA account to reclaim rent
            instructions.push(
                spl_token::instruction::close_account(
                    &accounts::TOKEN_PROGRAM,
                    &wsol_token_account,
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &[],
                )
                .unwrap(),
            );
        }

        Ok(instructions)
    }

    /// 使用提供的账户信息构建卖出指令
    async fn build_sell_instructions_with_accounts(
        &self,
        params: &SellParams,
    ) -> Result<Vec<Instruction>> {
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();

        // 获取代币余额
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

        let mut minimum_amount_out: u64 = if protocol_params.minimum_amount_out.is_some() {
            protocol_params.minimum_amount_out.unwrap()
        } else {
            println!("minimum_amount_out not provided, using default value 0");
            // 未提供minimum_amount_out，使用默认值0
            0
        };
        if minimum_amount_out != 0 {
            let slippage_basis_points: u64 = if params.slippage_basis_points.is_some() {
                params.slippage_basis_points.unwrap()
            } else {
                DEFAULT_SLIPPAGE
            } as u64;
            minimum_amount_out = minimum_amount_out * (10000 - slippage_basis_points) / 10000;
            println!("slippage_basis_points: {}", slippage_basis_points);
        }
        println!("minimum_amount_out: {}", minimum_amount_out);

        let pool_state = if protocol_params.pool_state.is_some() {
            protocol_params.pool_state.unwrap()
        } else {
            let mint_token_in_pool_state_index = protocol_params
                .mint_token_in_pool_state_index
                .unwrap_or(1);
            get_pool_pda(
                &accounts::AMM_CONFIG,
                if mint_token_in_pool_state_index == 1 {
                    &accounts::WSOL_TOKEN_ACCOUNT
                } else {
                    &params.mint
                },
                if mint_token_in_pool_state_index == 1 {
                    &params.mint
                } else {
                    &accounts::WSOL_TOKEN_ACCOUNT
                },
            )
            .unwrap()
        };

        let wsol_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &accounts::WSOL_TOKEN_ACCOUNT,
        );
        let mint_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &params.mint,
        );

        // 获取池的代币账户
        // Get pool token accounts
        let wsol_vault_account = get_vault_pda(&pool_state, &accounts::WSOL_TOKEN_ACCOUNT).unwrap();
        let mint_vault_account = get_vault_pda(&pool_state, &params.mint).unwrap();

        let observation_state_account = get_observation_state_pda(&pool_state).unwrap();

        let mut instructions = vec![];

        // 插入wsol
        // Insert wSOL
        instructions.push(
            // 创建wSOL ATA账户，如果不存在
            // Create wSOL ATA account if it doesn't exist
            create_associated_token_account_idempotent(
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &accounts::WSOL_TOKEN_ACCOUNT,
                &protocol_params.mint_token_program.unwrap(),
            ),
        );

        // 创建卖出指令
        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AUTHORITY, false), // Authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_CONFIG, false), // Amm Config (readonly)
            solana_sdk::instruction::AccountMeta::new(pool_state, false), // Pool State
            solana_sdk::instruction::AccountMeta::new(mint_token_account, false), // Input Token Account
            solana_sdk::instruction::AccountMeta::new(wsol_token_account, false), // Output Token Account
            solana_sdk::instruction::AccountMeta::new(mint_vault_account, false), // Input Vault Account
            solana_sdk::instruction::AccountMeta::new(wsol_vault_account, false), // Output Vault Account
            solana_sdk::instruction::AccountMeta::new_readonly(
                protocol_params
                    .mint_token_program
                    .unwrap_or(accounts::TOKEN_PROGRAM),
                false,
            ), // Input Token Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // Output Token Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(params.mint, false), // Input token mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::WSOL_TOKEN_ACCOUNT, false), // Output token mint (readonly)
            solana_sdk::instruction::AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // 创建指令数据
        let mut data = vec![];
        data.extend_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::RAYDIUM_CPMM,
            accounts,
            data,
        });

        if protocol_params.auto_handle_wsol {
            instructions.push(
                close_account(
                    &accounts::TOKEN_PROGRAM,
                    &wsol_token_account,
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &[&params.payer.pubkey()],
                )
                .unwrap(),
            );
        }

        Ok(instructions)
    }
}
