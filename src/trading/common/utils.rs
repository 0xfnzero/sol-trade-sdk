use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer, transaction::Transaction};
use solana_system_interface::instruction as system_instruction;

use crate::common::{
    fast_fn::{
        get_associated_token_address_with_program_id_fast,
        get_associated_token_address_with_program_id_fast_use_seed,
    },
    spl_token::close_account,
    SolanaRpcClient,
};
use anyhow::anyhow;

/// SPL Token account `amount` field (u64 LE at offset 64).
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_AMOUNT_END: usize = 72;

fn token_account_amount(data: &[u8], label: &str) -> Result<u64, anyhow::Error> {
    data.get(TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_END)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| anyhow!("{label}: token account data too short for amount"))
}

/// Get the balances of two tokens in the pool.
///
/// Uses `getMultipleAccounts` + local SPL layout decode instead of
/// `getTokenAccountBalance`, which some public RPCs (e.g. PublicNode/Allnodes)
/// gate behind a personal token as an "indexed" method.
///
/// # Returns
/// Returns token0_balance, token1_balance
pub async fn get_multi_token_balances(
    rpc: &SolanaRpcClient,
    token0_vault: &Pubkey,
    token1_vault: &Pubkey,
) -> Result<(u64, u64), anyhow::Error> {
    let accounts = rpc.get_multiple_accounts(&[*token0_vault, *token1_vault]).await?;
    let token0_data = accounts
        .first()
        .and_then(Option::as_ref)
        .map(|a| a.data.as_slice())
        .ok_or_else(|| anyhow!("token0 vault account not found: {token0_vault}"))?;
    let token1_data = accounts
        .get(1)
        .and_then(Option::as_ref)
        .map(|a| a.data.as_slice())
        .ok_or_else(|| anyhow!("token1 vault account not found: {token1_vault}"))?;
    Ok((
        token_account_amount(token0_data, "token0")?,
        token_account_amount(token1_data, "token1")?,
    ))
}

#[inline]
pub async fn get_token_balance(
    rpc: &SolanaRpcClient,
    payer: &Pubkey,
    mint: &Pubkey,
) -> Result<u64, anyhow::Error> {
    get_token_balance_with_options(rpc, payer, mint, &crate::constants::TOKEN_PROGRAM, false).await
}

/// 使用与交易指令一致的 ATA 推导（可选 seed）查询余额；卖出/余额查询应与买入使用同一 ATA 地址。
#[inline]
pub async fn get_token_balance_with_options(
    rpc: &SolanaRpcClient,
    payer: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    use_seed: bool,
) -> Result<u64, anyhow::Error> {
    let ata = get_associated_token_address_with_program_id_fast_use_seed(
        payer,
        mint,
        token_program,
        use_seed,
    );
    let account = rpc.get_account(&ata).await?;
    token_account_amount(&account.data, "ata")
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

    let transfer_instruction =
        system_instruction::transfer(&payer.pubkey(), receive_wallet, amount);

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
