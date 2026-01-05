pub mod calc;
pub mod price;
use crate::trading;
use crate::TradingClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;

impl TradingClient {
    #[inline]
    pub async fn get_sol_balance(&self, payer: &Pubkey) -> Result<u64, anyhow::Error> {
        trading::common::utils::get_sol_balance(&self.infrastructure.rpc, payer).await
    }

    #[inline]
    pub async fn get_payer_sol_balance(&self) -> Result<u64, anyhow::Error> {
        trading::common::utils::get_sol_balance(&self.infrastructure.rpc, &self.payer.pubkey()).await
    }

    #[inline]
    pub async fn get_token_balance(
        &self,
        payer: &Pubkey,
        mint: &Pubkey,
    ) -> Result<u64, anyhow::Error> {
        trading::common::utils::get_token_balance(&self.infrastructure.rpc, payer, mint).await
    }

    #[inline]
    pub async fn get_payer_token_balance(&self, mint: &Pubkey) -> Result<u64, anyhow::Error> {
        trading::common::utils::get_token_balance(&self.infrastructure.rpc, &self.payer.pubkey(), mint).await
    }

    #[inline]
    pub fn get_payer_pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    #[inline]
    pub fn get_payer(&self) -> &Keypair {
        self.payer.as_ref()
    }

    #[inline]
    pub async fn transfer_sol(
        &self,
        payer: &Keypair,
        receive_wallet: &Pubkey,
        amount: u64,
    ) -> Result<(), anyhow::Error> {
        trading::common::utils::transfer_sol(&self.infrastructure.rpc, payer, receive_wallet, amount).await
    }

    #[inline]
    pub async fn close_token_account(&self, mint: &Pubkey) -> Result<(), anyhow::Error> {
        trading::common::utils::close_token_account(&self.infrastructure.rpc, self.payer.as_ref(), mint).await
    }
}
