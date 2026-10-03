//! Meteora Dynamic Bonding Curve swaps: `swap2` for a plain pool,
//! `swap2_with_transfer_hook` for a pool whose base token has a transfer hook.

use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::meteora_dbc::{
            accounts, resolve_transfer_hook_accounts, SWAP2_DISCRIMINATOR,
            SWAP2_WITH_TRANSFER_HOOK_DISCRIMINATOR, SWAP_MODE_EXACT_IN, SWAP_MODE_EXACT_OUT,
            SWAP_MODE_PARTIAL_FILL, TRANSFER_HOOK_BASE_ACCOUNTS,
        },
    },
    trading::core::{
        params::{DbcTransferHook, MeteoraDbcParams, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::common::calculate_min_amount_out,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for Meteora Dynamic Bonding Curve pools.
pub struct MeteoraDbcInstructionBuilder;

fn protocol_params(params: &SwapParams) -> Result<&MeteoraDbcParams> {
    params
        .protocol_params
        .as_any()
        .downcast_ref::<MeteoraDbcParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDbc"))
}

fn user_token_account(params: &SwapParams, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
        &params.payer.pubkey(),
        mint,
        token_program,
        params.open_seed_optimize,
    )
}

/// `swap2`'s `(amount_0, amount_1)`: the input and its minimum output, or for
/// an exact-out swap the output and the most it may cost. The minimum output
/// is `fixed_output_amount`, else the quote on the pool's curve less slippage.
fn swap_amounts(
    params: &SwapParams,
    pool: &MeteoraDbcParams,
    is_buy: bool,
    amount_in: u64,
) -> Result<(u64, u64)> {
    match pool.swap_mode {
        SWAP_MODE_EXACT_OUT => {
            let amount_out = params.fixed_output_amount.ok_or_else(|| {
                anyhow!("fixed_output_amount must be set for a MeteoraDbc exact-out swap")
            })?;
            Ok((amount_out, amount_in))
        }
        SWAP_MODE_EXACT_IN | SWAP_MODE_PARTIAL_FILL => {
            if let Some(minimum_amount_out) = params.fixed_output_amount {
                return Ok((amount_in, minimum_amount_out));
            }
            let curve = pool.quote.as_ref().ok_or_else(|| {
                anyhow!("MeteoraDbc swap needs the pool's curve or fixed_output_amount")
            })?;
            let quote = curve.quote_exact_in(is_buy, amount_in)?;
            if quote.amount_out == 0 {
                return Err(anyhow!("MeteoraDbc swap of {amount_in} buys nothing"));
            }
            let slippage = params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE);
            Ok((amount_in, calculate_min_amount_out(quote.amount_out, slippage)))
        }
        mode => Err(anyhow!("Unsupported MeteoraDbc swap_mode {mode}")),
    }
}

/// The base token's transfer-hook accounts for a swap moving it between the
/// pool's vault and `user_base_account`.
fn transfer_hook_accounts(
    pool: &MeteoraDbcParams,
    payer: &Pubkey,
    user_base_account: &Pubkey,
    is_buy: bool,
) -> Result<Option<Vec<AccountMeta>>> {
    match &pool.transfer_hook {
        None => Ok(None),
        Some(DbcTransferHook::Accounts(hook_accounts)) => Ok(Some(hook_accounts.clone())),
        Some(DbcTransferHook::Metas { program, metas }) => {
            // A buy is the pool authority's transfer out of the vault; a sale
            // the payer's transfer into it.
            let (source, destination, authority) = if is_buy {
                (&pool.base_vault, user_base_account, &accounts::POOL_AUTHORITY)
            } else {
                (user_base_account, &pool.base_vault, payer)
            };
            resolve_transfer_hook_accounts(
                program,
                &pool.base_mint,
                metas,
                source,
                destination,
                authority,
            )
            .map(Some)
        }
    }
}

fn swap_instruction(
    pool: &MeteoraDbcParams,
    payer: &Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    amounts: (u64, u64),
    hook_accounts: Option<Vec<AccountMeta>>,
) -> Result<Instruction> {
    let hook_count = hook_accounts.as_ref().map_or(0, Vec::len);
    let mut account_metas =
        Vec::with_capacity(15 + usize::from(pool.include_instructions_sysvar) + hook_count);
    account_metas.extend([
        accounts::POOL_AUTHORITY_META,
        AccountMeta::new_readonly(pool.config, false),
        AccountMeta::new(pool.pool, false),
        AccountMeta::new(input_token_account, false),
        AccountMeta::new(output_token_account, false),
        AccountMeta::new(pool.base_vault, false),
        AccountMeta::new(pool.quote_vault, false),
        // The swap that completes a transfer-hook pool's curve revokes the hook.
        AccountMeta {
            pubkey: pool.base_mint,
            is_signer: false,
            is_writable: hook_accounts.is_some(),
        },
        AccountMeta::new_readonly(pool.quote_mint, false),
        AccountMeta::new_readonly(*payer, true),
        AccountMeta::new_readonly(pool.base_token_program, false),
        AccountMeta::new_readonly(pool.quote_token_program, false),
        // Without a referral the slot holds the program id.
        match pool.referral_token_account {
            Some(referral) => AccountMeta::new(referral, false),
            None => accounts::METEORA_DBC_META,
        },
        accounts::EVENT_AUTHORITY_META,
        accounts::METEORA_DBC_META,
    ]);
    // Remaining accounts: the instructions sysvar, then the hook accounts.
    if pool.include_instructions_sysvar {
        account_metas.push(accounts::SYSVAR_INSTRUCTIONS_META);
    }

    let mut data = Vec::with_capacity(31);
    data.extend_from_slice(match hook_accounts {
        Some(_) => &SWAP2_WITH_TRANSFER_HOOK_DISCRIMINATOR,
        None => &SWAP2_DISCRIMINATOR,
    });
    data.extend_from_slice(&amounts.0.to_le_bytes());
    data.extend_from_slice(&amounts.1.to_le_bytes());
    data.push(pool.swap_mode);
    if let Some(hook_accounts) = hook_accounts {
        // `TransferHookAccountsInfo`: the slices of the remaining accounts.
        let count = u8::try_from(hook_accounts.len())
            .map_err(|_| anyhow!("MeteoraDbc transfer hook takes too many accounts"))?;
        if count == 0 {
            data.extend_from_slice(&0u32.to_le_bytes());
        } else {
            data.extend_from_slice(&1u32.to_le_bytes());
            data.extend_from_slice(&[TRANSFER_HOOK_BASE_ACCOUNTS, count]);
        }
        account_metas.extend(hook_accounts);
    }

    Ok(Instruction { program_id: accounts::METEORA_DBC, accounts: account_metas, data })
}

#[async_trait::async_trait]
impl InstructionBuilder for MeteoraDbcInstructionBuilder {
    /// Buys the pool's base token with its quote.
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        let amount_in = params.input_amount.unwrap_or(0);
        if amount_in == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        let pool = protocol_params(params)?;
        if params.output_mint != pool.base_mint {
            return Err(anyhow!(
                "MeteoraDbc pool {} sells {}, not {}",
                pool.pool,
                pool.base_mint,
                params.output_mint
            ));
        }
        let payer = params.payer.pubkey();
        let amounts = swap_amounts(params, pool, true, amount_in)?;
        let quote_account = user_token_account(params, &pool.quote_mint, &pool.quote_token_program);
        let base_account = user_token_account(params, &pool.base_mint, &pool.base_token_program);
        let hook_accounts = transfer_hook_accounts(pool, &payer, &base_account, true)?;

        let mut instructions = Vec::with_capacity(6);
        if params.create_input_mint_ata {
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &payer,
                &pool.quote_mint,
                &pool.quote_token_program,
                amount_in,
                params.open_seed_optimize,
            );
        }
        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &payer,
                &pool.base_mint,
                &pool.base_token_program,
                params.open_seed_optimize,
            );
        }
        instructions.push(swap_instruction(
            pool,
            &payer,
            quote_account,
            base_account,
            amounts,
            hook_accounts,
        )?);
        if params.close_input_mint_ata {
            // Also returns what a partial fill left in the wrapped account.
            push_close_wsol_if_needed(&mut instructions, &payer, &pool.quote_mint);
        }
        Ok(instructions)
    }

    /// Sells the pool's base token for its quote.
    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        let amount_in = params
            .input_amount
            .filter(|&amount| amount > 0)
            .ok_or_else(|| anyhow!("Token amount is not set"))?;
        let pool = protocol_params(params)?;
        if params.input_mint != pool.base_mint {
            return Err(anyhow!(
                "MeteoraDbc pool {} buys {}, not {}",
                pool.pool,
                pool.base_mint,
                params.input_mint
            ));
        }
        let payer = params.payer.pubkey();
        let amounts = swap_amounts(params, pool, false, amount_in)?;
        let quote_account = user_token_account(params, &pool.quote_mint, &pool.quote_token_program);
        let base_account = user_token_account(params, &pool.base_mint, &pool.base_token_program);
        let hook_accounts = transfer_hook_accounts(pool, &payer, &base_account, false)?;

        let mut instructions = Vec::with_capacity(4);
        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &payer,
                &pool.quote_mint,
                &pool.quote_token_program,
                params.open_seed_optimize,
            );
        }
        instructions.push(swap_instruction(
            pool,
            &payer,
            base_account,
            quote_account,
            amounts,
            hook_accounts,
        )?);
        if params.close_output_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &payer, &pool.quote_mint);
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &pool.base_token_program,
                &base_account,
                &payer,
                &payer,
                &[&payer],
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
        instruction::utils::meteora_dbc::ExtraAccountMeta,
        instruction::utils::meteora_dbc_types::{DbcBaseFee, DbcConfig, DbcCurvePoint},
        swqos::TradeType,
        trading::core::params::{DbcQuoteState, DexParamEnum},
    };
    use solana_sdk::{pubkey, signature::Keypair};
    use std::sync::Arc;

    const USDC: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

    /// Mainnet pool 2Rz8zRLAqMtXKBGsxb8DwYN1Ed13TDwLtxNUrEUHtBJY as transaction
    /// ysZEH25dfiiZMm94… traded it: a transfer-hook pool quoted in USDC.
    fn hook_pool() -> MeteoraDbcParams {
        MeteoraDbcParams::new(
            pubkey!("2Rz8zRLAqMtXKBGsxb8DwYN1Ed13TDwLtxNUrEUHtBJY"),
            pubkey!("CchPHVPXdshYVhUd3ExZDd9vJQgW3NK5jhesoeewLZp8"),
            pubkey!("mAo7GAjCZ2LCW5yNoQW31Ce9kLttUMCjyjD2kP9ever"),
            USDC,
            pubkey!("3CapsPu1TXoao9PASYQ25geQL2Va2X5eDb2Hxs41bWad"),
            pubkey!("7EJZt8h4wfSoqDV4A2vtS4VSHkEq3Zcpv8X3B99KrWbQ"),
            crate::constants::TOKEN_PROGRAM_2022,
            crate::constants::TOKEN_PROGRAM,
        )
        .with_transfer_hook(DbcTransferHook::Accounts(vec![
            AccountMeta::new_readonly(
                pubkey!("887b3SjuJv9t9wP6fd7Fe7dqFhksC8c39a2PJRa3cGec"),
                false,
            ),
            AccountMeta::new_readonly(
                pubkey!("FGZZEin9TMPnyNMPvdXtgNRPD41f8gV6sByTnDVRXiUR"),
                false,
            ),
        ]))
    }

    fn curve(sqrt_price: u128) -> DbcQuoteState {
        DbcQuoteState {
            config: Arc::new(DbcConfig {
                base_fee: DbcBaseFee { cliff_fee_numerator: 20_000_000, ..Default::default() },
                migration_sqrt_price: 1_750_011_800_614_054_764,
                sqrt_start_price: 583_337_266_871_351_588,
                curve: vec![DbcCurvePoint {
                    sqrt_price: 1_837_512_390_644_757_503,
                    liquidity: 2_916_686_334_356_757_942_357_946_112_045,
                }],
                ..Default::default()
            }),
            sqrt_price,
            fee_numerator: 20_000_000,
            rate_limited_buys: false,
        }
    }

    fn swap_params(
        trade_type: TradeType,
        input_mint: Pubkey,
        output_mint: Pubkey,
        input_amount: u64,
        pool: MeteoraDbcParams,
    ) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type,
            input_mint,
            input_token_program: None,
            output_mint,
            output_token_program: None,
            input_amount: Some(input_amount),
            slippage_basis_points: Some(1_000),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::MeteoraDbc(pool),
            open_seed_optimize: false,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: false,
            close_output_mint_ata: false,
            fixed_output_amount: None,
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
            precheck: None,
        }
    }

    fn ata(owner: &Pubkey, mint: &Pubkey, program: &Pubkey) -> Pubkey {
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            owner, mint, program, false,
        )
    }

    #[tokio::test]
    async fn hook_pool_buy_matches_the_mainnet_instruction() {
        let pool = hook_pool().with_quote(curve(1_730_409_438_693_799_042));
        let base_mint = pool.base_mint;
        let params = swap_params(TradeType::Buy, USDC, base_mint, 56_387_707, pool.clone());
        let payer = params.payer.pubkey();
        let instructions =
            MeteoraDbcInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        assert_eq!(instructions.len(), 1);
        let ix = &instructions[0];

        assert_eq!(ix.program_id, accounts::METEORA_DBC);
        // The discriminator, amounts and mode of `swap2_with_transfer_hook`,
        // then one slice of two base-hook accounts, as the mainnet buy sent.
        assert_eq!(&ix.data[..8], &[0xb7, 0x5d, 0x99, 0x28, 0x18, 0xe6, 0xc2, 0x97]);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 56_387_707);
        // 6_256_581_995 quoted, less 10%.
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 5_630_923_795);
        assert_eq!(&ix.data[24..], &[1, 1, 0, 0, 0, 0, 2]);

        let keys: Vec<Pubkey> = ix.accounts.iter().map(|account| account.pubkey).collect();
        assert_eq!(
            keys,
            vec![
                accounts::POOL_AUTHORITY,
                pool.config,
                pool.pool,
                ata(&payer, &USDC, &crate::constants::TOKEN_PROGRAM),
                ata(&payer, &base_mint, &crate::constants::TOKEN_PROGRAM_2022),
                pool.base_vault,
                pool.quote_vault,
                base_mint,
                USDC,
                payer,
                crate::constants::TOKEN_PROGRAM_2022,
                crate::constants::TOKEN_PROGRAM,
                accounts::METEORA_DBC,
                accounts::EVENT_AUTHORITY,
                accounts::METEORA_DBC,
                pubkey!("887b3SjuJv9t9wP6fd7Fe7dqFhksC8c39a2PJRa3cGec"),
                pubkey!("FGZZEin9TMPnyNMPvdXtgNRPD41f8gV6sByTnDVRXiUR"),
            ]
        );
        let writable: Vec<usize> = ix
            .accounts
            .iter()
            .enumerate()
            .filter_map(|(index, account)| account.is_writable.then_some(index))
            .collect();
        // Pool, both user accounts, both vaults and, for a hook pool, the mint.
        assert_eq!(writable, vec![2, 3, 4, 5, 6, 7]);
        let signers: Vec<usize> = ix
            .accounts
            .iter()
            .enumerate()
            .filter_map(|(index, account)| account.is_signer.then_some(index))
            .collect();
        assert_eq!(signers, vec![9]);
    }

    #[tokio::test]
    async fn plain_pool_swaps_use_swap2_and_wrap_sol() {
        let mut pool = hook_pool().with_quote(curve(1_730_409_438_693_799_042));
        pool.transfer_hook = None;
        pool.quote_mint = crate::constants::WSOL_TOKEN_ACCOUNT;
        pool.base_token_program = crate::constants::TOKEN_PROGRAM;
        let base_mint = pool.base_mint;

        let mut buy = swap_params(
            TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            base_mint,
            56_387_707,
            pool.clone(),
        );
        buy.create_input_mint_ata = true;
        buy.close_input_mint_ata = true;
        buy.create_output_mint_ata = true;
        let instructions = MeteoraDbcInstructionBuilder.build_buy_instructions(&buy).await.unwrap();
        let swap = instructions
            .iter()
            .position(|ix| ix.program_id == accounts::METEORA_DBC)
            .expect("swap instruction");
        // Wrap and create accounts first, close the wrapped account last.
        assert!(swap > 0 && swap < instructions.len() - 1);
        let ix = &instructions[swap];
        assert_eq!(&ix.data[..8], &SWAP2_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 25);
        assert_eq!(ix.data[24], SWAP_MODE_PARTIAL_FILL);
        assert_eq!(ix.accounts.len(), 15);
        assert!(!ix.accounts[7].is_writable);

        let sell = swap_params(
            TradeType::Sell,
            base_mint,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            6_256_581_995,
            pool.clone(),
        );
        let payer = sell.payer.pubkey();
        let instructions =
            MeteoraDbcInstructionBuilder.build_sell_instructions(&sell).await.unwrap();
        let ix = &instructions[0];
        // A sale's input is the base account, its output the quote account.
        assert_eq!(
            ix.accounts[3].pubkey,
            ata(&payer, &base_mint, &crate::constants::TOKEN_PROGRAM)
        );
        assert_eq!(
            ix.accounts[4].pubkey,
            ata(&payer, &crate::constants::WSOL_TOKEN_ACCOUNT, &crate::constants::TOKEN_PROGRAM)
        );
        let minimum_out = u64::from_le_bytes(ix.data[16..24].try_into().unwrap());
        let quote = pool.quote.unwrap().quote_exact_in(false, 6_256_581_995).unwrap();
        assert_eq!(minimum_out, calculate_min_amount_out(quote.amount_out, 1_000));
    }

    #[tokio::test]
    async fn sysvar_and_referral_take_their_slots() {
        let pool = hook_pool()
            .with_quote(curve(1_730_409_438_693_799_042))
            .with_instructions_sysvar(true)
            .with_referral_token_account(Pubkey::new_from_array([9; 32]));
        let params = swap_params(TradeType::Buy, USDC, pool.base_mint, 1_000_000, pool);
        let instructions =
            MeteoraDbcInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = &instructions[0];
        assert_eq!(ix.accounts.len(), 18);
        assert_eq!(ix.accounts[12].pubkey, Pubkey::new_from_array([9; 32]));
        assert!(ix.accounts[12].is_writable);
        // The sysvar comes ahead of the hook accounts.
        assert_eq!(ix.accounts[15].pubkey, accounts::SYSVAR_INSTRUCTIONS);
        assert_eq!(ix.accounts[16].pubkey, pubkey!("887b3SjuJv9t9wP6fd7Fe7dqFhksC8c39a2PJRa3cGec"));
    }

    #[tokio::test]
    async fn hook_metas_resolve_for_the_swaps_own_accounts() {
        // PDA(hook, ["cfg", mint]), writable.
        let mut config = [0u8; 32];
        config[..7].copy_from_slice(&[1, 3, b'c', b'f', b'g', 3, 1]);
        let hook = pubkey!("C3vEdPepTPRrJdQ4nQ3ZmhdXCmpKdGRKVUqxduHZWbdR");
        let mut pool = hook_pool().with_quote(curve(1_730_409_438_693_799_042));
        pool.base_mint = pubkey!("HAoowFkDyWfuetaV7DBdmU4aesB5jn8jL7uRpnSWengW");
        pool.transfer_hook = Some(DbcTransferHook::Metas {
            program: hook,
            metas: vec![ExtraAccountMeta {
                discriminator: 1,
                address_config: config,
                is_signer: false,
                is_writable: true,
            }],
        });
        let params = swap_params(TradeType::Sell, pool.base_mint, USDC, 1_000_000, pool);
        let instructions =
            MeteoraDbcInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let ix = &instructions[0];
        assert_eq!(ix.accounts.len(), 18);
        assert_eq!(
            ix.accounts[15],
            AccountMeta::new(pubkey!("GjucFNkLjTEEMmyxfaR73273Fohb32CHuLDY5A2D4u62"), false)
        );
        assert_eq!(ix.accounts[16], AccountMeta::new_readonly(hook, false));
        assert_eq!(&ix.data[25..], &[1, 0, 0, 0, 0, 3]);
    }

    #[tokio::test]
    async fn minimum_output_needs_the_curve_or_a_fixed_amount() {
        let pool = hook_pool();
        let mut params = swap_params(TradeType::Buy, USDC, pool.base_mint, 1_000_000, pool.clone());
        assert!(MeteoraDbcInstructionBuilder.build_buy_instructions(&params).await.is_err());

        params.fixed_output_amount = Some(77);
        let instructions =
            MeteoraDbcInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        assert_eq!(u64::from_le_bytes(instructions[0].data[16..24].try_into().unwrap()), 77);

        // Exact out: the output first, then the most it may cost.
        params.protocol_params = DexParamEnum::MeteoraDbc(pool.with_swap_mode(SWAP_MODE_EXACT_OUT));
        let instructions =
            MeteoraDbcInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let data = &instructions[0].data;
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 77);
        assert_eq!(u64::from_le_bytes(data[16..24].try_into().unwrap()), 1_000_000);
        assert_eq!(data[24], SWAP_MODE_EXACT_OUT);
    }

    #[tokio::test]
    async fn swap_of_another_mint_is_refused() {
        let pool = hook_pool().with_quote(curve(1_730_409_438_693_799_042));
        let other = Pubkey::new_from_array([3; 32]);
        let buy = swap_params(TradeType::Buy, USDC, other, 1_000_000, pool.clone());
        assert!(MeteoraDbcInstructionBuilder.build_buy_instructions(&buy).await.is_err());
        let sell = swap_params(TradeType::Sell, other, USDC, 1_000_000, pool);
        assert!(MeteoraDbcInstructionBuilder.build_sell_instructions(&sell).await.is_err());
    }
}
