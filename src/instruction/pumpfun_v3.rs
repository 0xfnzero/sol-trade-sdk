//! V3 integration for direct trades and shared quote routes. No RPC or implicit quotes.
use super::{
    pump_compact_accounts::{PumpCompactAccountParams, PumpV3Accounts},
    pump_upgrade::PumpV3Instruction,
    token_account_setup::push_create_user_token_account,
};
use crate::{
    constants::{SOL_TOKEN_ACCOUNT, TOKEN_PROGRAM, TOKEN_PROGRAM_2022, WSOL_TOKEN_ACCOUNT},
    trading::core::params::{PumpFunV3Params, SwapParams},
};
use anyhow::{ensure, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};

pub(super) fn validate(p: &PumpFunV3Params, mint: Pubkey) -> Result<(Pubkey, Pubkey, Pubkey)> {
    let curve = &p.curve.bonding_curve;
    let quote = curve.effective_quote_mint();
    ensure!(!curve.complete && !curve.is_cashback_coin, "V3 requires an active non-cashback curve");
    ensure!(
        Some(curve.account) == super::utils::pumpfun::get_bonding_curve_pda(&mint),
        "V3 curve PDA mismatch"
    );
    ensure!(mint != Pubkey::default() && mint != quote, "Invalid V3 base mint");
    ensure!(
        crate::common::bonding_curve::BondingCurveAccount::normalize_quote_mint(p.curve.quote_mint)
            == quote,
        "V3 quote mint mismatch"
    );
    let base_program = super::pumpfun::effective_pump_mint_token_program(&p.curve, &mint);
    for program in [base_program, p.quote_token_program] {
        ensure!(
            program == TOKEN_PROGRAM || program == TOKEN_PROGRAM_2022,
            "Invalid V3 token program"
        );
    }
    ensure!(
        quote != WSOL_TOKEN_ACCOUNT || p.quote_token_program == TOKEN_PROGRAM,
        "WSOL requires SPL Token"
    );
    ensure!(
        p.base_transfer_fee.basis_points == 0 && p.quote_transfer_fee.basis_points == 0,
        "V3 integration requires verified zero transfer fees"
    );
    Ok((quote, base_program, p.quote_token_program))
}

pub(super) fn build(p: &SwapParams, v3: &PumpFunV3Params, buy: bool) -> Result<Vec<Instruction>> {
    let mint = if buy { p.output_mint } else { p.input_mint };
    let (quote, base_program, quote_program) = validate(v3, mint)?;
    let quote_input = if buy { p.input_mint } else { p.output_mint };
    let native = quote == WSOL_TOKEN_ACCOUNT;
    ensure!(
        quote_input == if native { SOL_TOKEN_ACCOUNT } else { quote },
        "V3 quote endpoint mismatch; native curves consume/return SOL, not WSOL tokens"
    );
    let amount = p.input_amount.unwrap_or(0);
    ensure!(amount > 0, "V3 requires nonzero input");
    ensure!(
        p.use_exact_sol_amount != Some(false) || (buy && p.fixed_output_amount.is_some()),
        "V3 requires exact input or an explicit exact-output buy"
    );
    let minimum = if buy {
        v3.minimum_output_amount
    } else {
        p.fixed_output_amount.unwrap_or(v3.minimum_output_amount)
    };
    ensure!(
        if buy { p.fixed_output_amount.unwrap_or(minimum) } else { minimum } > 0,
        "V3 requires a nonzero output limit"
    );
    let user = p.payer.pubkey();
    let accounts = PumpV3Accounts::derive(PumpCompactAccountParams {
        user,
        base_mint: mint,
        quote_mint: quote,
        base_token_program: base_program,
        quote_token_program: quote_program,
        buyback_recipient: super::utils::pumpfun::get_buyback_fee_recipient_random(),
        cashback: v3.curve.bonding_curve.is_cashback_coin,
        complete: v3.curve.bonding_curve.complete,
    })?;
    // Canonical ATAs must match the compact account derivation, regardless of seed settings.
    let mut ixs = Vec::with_capacity(4);
    let create_base = if buy { p.create_output_mint_ata } else { p.create_input_mint_ata };
    let create_quote = if buy { p.create_input_mint_ata } else { p.create_output_mint_ata };
    if create_base {
        push_create_user_token_account(&mut ixs, &user, &mint, &base_program, false);
    }
    if create_quote && !native {
        push_create_user_token_account(&mut ixs, &user, &quote, &quote_program, false);
    }
    ixs.push(if buy {
        if let Some(output) = p.fixed_output_amount {
            accounts.instruction(PumpV3Instruction::Buy, output, amount)
        } else {
            accounts.instruction(PumpV3Instruction::BuyExactQuoteIn, amount, minimum)
        }
    } else {
        accounts.instruction(PumpV3Instruction::Sell, amount, minimum)
    });
    if !buy && (p.close_input_mint_ata || v3.curve.close_token_account_when_sell.unwrap_or(false)) {
        ixs.push(crate::common::spl_token::close_account(
            &base_program,
            &accounts.associated_base_user(),
            &user,
            &user,
            &[&user],
        )?);
    }
    Ok(ixs)
}
