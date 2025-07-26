use anyhow::{anyhow, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use spl_token::instruction::close_account;

use crate::{
    constants::pumpswap::{accounts, BUY_DISCRIMINATOR, SELL_DISCRIMINATOR},
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::common::utils::{
        calculate_with_slippage_buy, calculate_with_slippage_sell, get_token_balance,
    },
    trading::core::{
        params::{BuyParams, PumpSwapParams, SellParams},
        traits::InstructionBuilder,
    },
    trading::pumpswap::common::{
        coin_creator_vault_ata, coin_creator_vault_authority, find_pool, get_buy_token_amount,
        get_sell_sol_amount,
    },
};

/// PumpSwap协议的指令构建器
/// PumpSwap protocol instruction builder
pub struct PumpSwapInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpSwapInstructionBuilder {
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>> {
        // 获取PumpSwap特定参数
        // Get PumpSwap specific parameters
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        // 根据是否提供了账户信息来构建指令
        // Build instructions based on whether account information is provided
        match (&protocol_params.pool,) {
            (Some(pool),) => {
                self.build_buy_instructions_with_accounts(
                    params,
                    *pool,
                    protocol_params.auto_handle_wsol,
                )
                .await
            }
            _ => self.build_buy_instructions_auto_discover(params).await,
        }
    }

    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>> {
        // 获取PumpSwap特定参数
        // Get PumpSwap specific parameters
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        // 根据是否提供了账户信息来构建指令
        // Build instructions based on whether account information is provided
        match (&protocol_params.pool,) {
            (Some(pool),) => {
                self.build_sell_instructions_with_accounts(params, *pool)
                    .await
            }
            _ => self.build_sell_instructions_auto_discover(params).await,
        }
    }
}

impl PumpSwapInstructionBuilder {
    /// 自动发现池和账户信息并构建买入指令
    /// Automatically discover pool and account information and build buy instructions
    async fn build_buy_instructions_auto_discover(
        &self,
        params: &BuyParams,
    ) -> Result<Vec<Instruction>> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        // 查找池
        // Find pool
        let pool = find_pool(rpc.as_ref(), &params.mint).await?;

        self.build_buy_instructions_with_accounts(params, pool, true)
            .await
    }

    /// 自动发现池和账户信息并构建卖出指令
    /// Automatically discover pool and account information and build sell instructions
    async fn build_sell_instructions_auto_discover(
        &self,
        params: &SellParams,
    ) -> Result<Vec<Instruction>> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();

        // 查找池
        // Find pool
        let pool = find_pool(rpc.as_ref(), &params.mint).await?;

        self.build_sell_instructions_with_accounts(params, pool)
            .await
    }

    /// 使用提供的账户信息构建买入指令
    /// Build buy instructions using provided account information
    async fn build_buy_instructions_with_accounts(
        &self,
        params: &BuyParams,
        pool: Pubkey,
        auto_handle_wsol: bool,
    ) -> Result<Vec<Instruction>> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        // 计算预期的代币数量
        // Calculate expected token amount
        let token_amount = get_buy_token_amount(rpc.as_ref(), &pool, params.sol_amount).await?;

        // 计算滑点后的最大SOL数量
        // Calculate maximum SOL amount after slippage
        let max_sol_amount = calculate_with_slippage_buy(
            params.sol_amount,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );

        // 创建用户代币账户
        // Create user token account
        let user_base_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &params.mint,
        );
        let user_quote_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &accounts::WSOL_TOKEN_ACCOUNT,
        );

        // 获取池的代币账户
        // Get pool token account
        let pool_base_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &params.mint,
                &accounts::TOKEN_PROGRAM,
            );

        let pool_quote_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &accounts::WSOL_TOKEN_ACCOUNT,
                &accounts::TOKEN_PROGRAM,
            );

        let mut instructions = vec![];

        if auto_handle_wsol {
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
                    &user_quote_token_account,
                    max_sol_amount,
                ),
            );

            // 同步wSOL余额
            // Sync wSOL balance
            instructions.push(
                spl_token::instruction::sync_native(
                    &accounts::TOKEN_PROGRAM,
                    &user_quote_token_account,
                )
                .unwrap(),
            );
        }

        // 创建用户的基础代币账户
        // Create user base token account
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &params.mint,
            &accounts::TOKEN_PROGRAM,
        ));

        let coin_creator_vault_ata = coin_creator_vault_ata(params.creator);
        let coin_creator_vault_authority = coin_creator_vault_authority(params.creator);

        // 创建买入指令
        // Create buy instruction
        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::GLOBAL_ACCOUNT, false), // global (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(params.mint, false), // mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::WSOL_TOKEN_ACCOUNT, false), // WSOL_TOKEN_ACCOUNT (readonly)
            solana_sdk::instruction::AccountMeta::new(user_base_token_account, false), // user_base_token_account
            solana_sdk::instruction::AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            solana_sdk::instruction::AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            solana_sdk::instruction::AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::FEE_RECIPIENT, false), // fee_recipient (readonly)
            solana_sdk::instruction::AccountMeta::new(accounts::FEE_RECIPIENT_ATA, false), // fee_recipient_ata
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::SYSTEM_PROGRAM, false), // System Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(
                accounts::ASSOCIATED_TOKEN_PROGRAM,
                false,
            ), // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::EVENT_AUTHORITY, false), // event_authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_PROGRAM, false), // PUMP_AMM_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new(coin_creator_vault_ata, false), // coin_creator_vault_ata
            solana_sdk::instruction::AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly)
        ];

        // 创建指令数据
        // Create instruction data
        let mut data = vec![];
        data.extend_from_slice(&BUY_DISCRIMINATOR);
        data.extend_from_slice(&token_amount.to_le_bytes());
        data.extend_from_slice(&max_sol_amount.to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data,
        });

        if auto_handle_wsol {
            // 关闭wSOL ATA账户，回收租金
            // Close wSOL ATA account to reclaim rent
            instructions.push(
                spl_token::instruction::close_account(
                    &accounts::TOKEN_PROGRAM,
                    &user_quote_token_account,
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
    /// Build sell instructions using provided account information
    async fn build_sell_instructions_with_accounts(
        &self,
        params: &SellParams,
        pool: Pubkey,
    ) -> Result<Vec<Instruction>> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();

        // 获取代币余额
        // Get token balance
        let mut amount = params.token_amount;
        if params.token_amount.is_none() {
            let balance_u64 =
                get_token_balance(rpc.as_ref(), &params.payer.pubkey(), &params.mint).await?;
            amount = Some(balance_u64);
        }
        let amount = amount.unwrap_or(0);

        if amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        // 计算预期的SOL数量
        // Calculate expected SOL amount
        let sol_amount = get_sell_sol_amount(rpc.as_ref(), &pool, amount).await?;

        // 计算滑点后的最小SOL数量
        // Calculate minimum SOL amount after slippage
        let min_sol_amount = calculate_with_slippage_sell(
            sol_amount,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );

        let coin_creator_vault_ata = coin_creator_vault_ata(params.creator);
        let coin_creator_vault_authority = coin_creator_vault_authority(params.creator);

        let user_base_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &params.mint,
        );
        let user_quote_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &accounts::WSOL_TOKEN_ACCOUNT,
        );
        let pool_base_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &params.mint,
                &accounts::TOKEN_PROGRAM,
            );
        let pool_quote_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &accounts::WSOL_TOKEN_ACCOUNT,
                &accounts::TOKEN_PROGRAM,
            );

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
                &accounts::TOKEN_PROGRAM,
            ),
        );

        // 创建用户的代币账户
        // Create user token account
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &params.mint,
            &accounts::TOKEN_PROGRAM,
        ));

        // 创建卖出指令
        // Create sell instruction
        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::GLOBAL_ACCOUNT, false), // global (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(params.mint, false), // mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::WSOL_TOKEN_ACCOUNT, false), // WSOL_TOKEN_ACCOUNT (readonly)
            solana_sdk::instruction::AccountMeta::new(user_base_token_account, false), // user_base_token_account
            solana_sdk::instruction::AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            solana_sdk::instruction::AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            solana_sdk::instruction::AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::FEE_RECIPIENT, false), // fee_recipient (readonly)
            solana_sdk::instruction::AccountMeta::new(accounts::FEE_RECIPIENT_ATA, false), // fee_recipient_ata
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::SYSTEM_PROGRAM, false), // System Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(
                accounts::ASSOCIATED_TOKEN_PROGRAM,
                false,
            ), // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::EVENT_AUTHORITY, false), // event_authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_PROGRAM, false), // PUMP_AMM_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new(coin_creator_vault_ata, false), // coin_creator_vault_ata
            solana_sdk::instruction::AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly)
        ];

        // 创建指令数据
        // Create instruction data
        let mut data = vec![];
        data.extend_from_slice(&SELL_DISCRIMINATOR);
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&min_sol_amount.to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data,
        });

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        if protocol_params.auto_handle_wsol {
            instructions.push(
                close_account(
                    &accounts::TOKEN_PROGRAM,
                    &user_quote_token_account,
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
