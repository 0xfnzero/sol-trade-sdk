use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer, transaction::Transaction};
use solana_system_interface::instruction::transfer;

use crate::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token::close_account,
    SolanaRpcClient,
};
use anyhow::anyhow;

/// Get the balances of two tokens in the pool
///
/// # Returns
/// Returns token0_balance, token1_balance
pub async fn get_multi_token_balances(
    rpc: &SolanaRpcClient,
    token0_vault: &Pubkey,
    token1_vault: &Pubkey,
) -> Result<(u64, u64), anyhow::Error> {
    let token0_balance = rpc.get_token_account_balance(&token0_vault).await?;
    let token1_balance = rpc.get_token_account_balance(&token1_vault).await?;
    // Parse balance string to u64
    let token0_amount = token0_balance
        .amount
        .parse::<u64>()
        .map_err(|e| anyhow!("Failed to parse token0 balance: {}", e))?;
    let token1_amount = token1_balance
        .amount
        .parse::<u64>()
        .map_err(|e| anyhow!("Failed to parse token1 balance: {}", e))?;
    Ok((token0_amount, token1_amount))
}

#[inline]
pub async fn get_token_balance(
    rpc: &SolanaRpcClient,
    payer: &Pubkey,
    mint: &Pubkey,
) -> Result<u64, anyhow::Error> {
    let ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        payer,
        mint,
        &crate::constants::TOKEN_PROGRAM,
    );
    let balance = rpc.get_token_account_balance(&ata).await?;
    let balance_u64 =
        balance.amount.parse::<u64>().map_err(|_| anyhow!("Failed to parse token balance"))?;
    Ok(balance_u64)
}

#[inline]
pub async fn get_sol_balance(
    rpc: &SolanaRpcClient,
    account: &Pubkey,
) -> Result<u64, anyhow::Error> {
    let balance = rpc.get_balance(account).await?;
    Ok(balance)
}

pub async fn transfer_sol(
    rpc: &SolanaRpcClient,
    payer: &Keypair,
    receive_wallet: &Pubkey,
    amount: u64,
) -> Result<(), anyhow::Error> {
    if amount == 0 {
        return Err(anyhow!("transfer_sol: Amount cannot be zero"));
    }

    let balance = get_sol_balance(rpc, &payer.pubkey()).await?;
    if balance < amount {
        return Err(anyhow!("Insufficient balance"));
    }

    let transfer_instruction = transfer(&payer.pubkey(), receive_wallet, amount);

    let recent_blockhash = rpc.get_latest_blockhash().await?;

    let transaction = Transaction::new_signed_with_payer(
        &[transfer_instruction],
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    );

    rpc.send_and_confirm_transaction(&transaction).await?;

    Ok(())
}

/// Close token account
///
/// This function is used to close the associated token account for a specified token,
/// transferring the token balance in the account to the account owner.
///
/// # Parameters
///
/// * `rpc` - Solana RPC client
/// * `payer` - Account that pays transaction fees
/// * `mint` - Token mint address
///
/// # Returns
///
/// Returns a Result, success returns (), failure returns error
pub async fn close_token_account(
    rpc: &SolanaRpcClient,
    payer: &Keypair,
    mint: &Pubkey,
) -> Result<(), anyhow::Error> {
    // Get associated token account address
    let ata = get_associated_token_address_with_program_id_fast(
        &payer.pubkey(),
        mint,
        &crate::constants::TOKEN_PROGRAM,
    );

    // Check if account exists
    let account_exists = rpc.get_account(&ata).await.is_ok();
    if !account_exists {
        return Ok(()); // If account doesn't exist, return success directly
    }

    // Build close account instruction
    let close_account_ix = close_account(
        &crate::constants::TOKEN_PROGRAM,
        &ata,
        &payer.pubkey(),
        &payer.pubkey(),
        &[&payer.pubkey()],
    )?;

    // Build transaction
    let recent_blockhash = rpc.get_latest_blockhash().await?;
    let transaction = Transaction::new_signed_with_payer(
        &[close_account_ix],
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    );

    // Send transaction
    rpc.send_and_confirm_transaction(&transaction).await?;

    Ok(())
}
