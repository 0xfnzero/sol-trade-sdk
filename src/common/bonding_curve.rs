//! Bonding curve account for the Pump.fun Solana Program
//!
//! This module contains the definition for the bonding curve account.
//!
//! # Bonding Curve Account
//!
//! The bonding curve account is used to manage token pricing and liquidity.
//!
//! # Fields
//!
//! - `discriminator`: Unique identifier for the bonding curve
//! - `virtual_token_reserves`: Virtual token reserves used for price calculations
//! - `virtual_sol_reserves`: Virtual SOL reserves used for price calculations
//! - `real_token_reserves`: Actual token reserves available for trading
//! - `real_sol_reserves`: Actual SOL reserves available for trading
//! - `token_total_supply`: Total supply of tokens
//! - `complete`: Whether the bonding curve is complete/finalized
//!
//! # Methods
//!
//! - `new`: Creates a new bonding curve instance
//! - `get_buy_price`: Calculates the amount of tokens received for a given SOL amount
//! - `get_sell_price`: Calculates the amount of SOL received for selling tokens
//! - `get_market_cap_sol`: Calculates the current market cap in SOL
//! - `get_final_market_cap_sol`: Calculates the final market cap in SOL after all tokens are sold
//! - `get_buy_out_price`: Calculates the price to buy out all remaining tokens

use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::instruction::utils::pumpfun::global_constants::{
    INITIAL_REAL_TOKEN_RESERVES, INITIAL_VIRTUAL_SOL_RESERVES, INITIAL_VIRTUAL_TOKEN_RESERVES,
    TOKEN_TOTAL_SUPPLY,
};
use crate::instruction::utils::pumpfun::{get_bonding_curve_pda, get_creator_vault_pda};

/// Represents the global configuration account for token pricing and fees
#[derive(Debug, Clone, Serialize, Deserialize, Default, BorshDeserialize)]
pub struct BondingCurveAccount {
    /// Unique identifier for the bonding curve
    #[borsh(skip)]
    pub discriminator: u64,
    /// Account address
    #[borsh(skip)]
    pub account: Pubkey,
    /// Virtual token reserves used for price calculations
    pub virtual_token_reserves: u64,
    /// Virtual SOL reserves used for price calculations
    pub virtual_sol_reserves: u64,
    /// Actual token reserves available for trading
    pub real_token_reserves: u64,
    /// Actual SOL reserves available for trading
    pub real_sol_reserves: u64,
    /// Total supply of tokens
    pub token_total_supply: u64,
    /// Whether the bonding curve is complete/finalized
    pub complete: bool,
    /// Creator of the bonding curve
    pub creator: Pubkey,
    /// Whether this is a mayhem mode token (Token2022)
    pub is_mayhem_mode: bool,
}

impl BondingCurveAccount {
    pub fn from_dev_trade(
        bonding_curve: Pubkey,
        mint: &Pubkey,
        dev_token_amount: u64,
        dev_sol_amount: u64,
        creator: Pubkey,
        is_mayhem_mode: bool,
    ) -> Self {
        let account = if bonding_curve != Pubkey::default() {
            bonding_curve
        } else {
            get_bonding_curve_pda(&mint).unwrap()
        };
        Self {
            discriminator: 0,
            account: account,
            virtual_token_reserves: INITIAL_VIRTUAL_TOKEN_RESERVES - dev_token_amount,
            virtual_sol_reserves: INITIAL_VIRTUAL_SOL_RESERVES + dev_sol_amount,
            real_token_reserves: INITIAL_REAL_TOKEN_RESERVES - dev_token_amount,
            real_sol_reserves: dev_sol_amount,
            token_total_supply: TOKEN_TOTAL_SUPPLY,
            complete: false,
            creator: creator,
            is_mayhem_mode: is_mayhem_mode,
        }
    }

    pub fn from_trade(
        bonding_curve: Pubkey,
        mint: Pubkey,
        creator: Pubkey,
        virtual_token_reserves: u64,
        virtual_sol_reserves: u64,
        real_token_reserves: u64,
        real_sol_reserves: u64,
        is_mayhem_mode: bool,
    ) -> Self {
        let account = if bonding_curve != Pubkey::default() {
            bonding_curve
        } else {
            get_bonding_curve_pda(&mint).unwrap()
        };
        Self {
            discriminator: 0,
            account: account,
            virtual_token_reserves: virtual_token_reserves,
            virtual_sol_reserves: virtual_sol_reserves,
            real_token_reserves: real_token_reserves,
            real_sol_reserves: real_sol_reserves,
            token_total_supply: TOKEN_TOTAL_SUPPLY,
            complete: false,
            creator: creator,
            is_mayhem_mode: is_mayhem_mode,
        }
    }

    pub fn get_creator_vault_pda(&self) -> Pubkey {
        get_creator_vault_pda(&self.creator).unwrap()
    }

    /// Calculates the amount of tokens received for a given SOL amount
    ///
    /// # Arguments
    /// * `amount` - Amount of SOL to spend
    ///
    /// # Returns
    /// * `Ok(u64)` - Amount of tokens that would be received
    /// * `Err(&str)` - Error message if curve is complete
    pub fn get_buy_price(&self, amount: u64) -> Result<u64, &'static str> {
        if self.complete {
            return Err("Curve is complete");
        }

        if amount == 0 {
            return Ok(0);
        }

        // Calculate the product of virtual reserves using u128 to avoid overflow
        let n: u128 = (self.virtual_sol_reserves as u128) * (self.virtual_token_reserves as u128);

        // Calculate the new virtual sol reserves after the purchase
        let i: u128 = (self.virtual_sol_reserves as u128) + (amount as u128);

        // Calculate the new virtual token reserves after the purchase
        let r: u128 = n / i + 1;

        // Calculate the amount of tokens to be purchased
        let s: u128 = (self.virtual_token_reserves as u128) - r;

        // Convert back to u64 and return the minimum of calculated tokens and real reserves
        let s_u64 = s as u64;
        Ok(if s_u64 < self.real_token_reserves { s_u64 } else { self.real_token_reserves })
    }

    /// Calculates the amount of SOL received for selling tokens
    ///
    /// # Arguments
    /// * `amount` - Amount of tokens to sell
    /// * `fee_basis_points` - Fee in basis points (1/100th of a percent)
    ///
    /// # Returns
    /// * `Ok(u64)` - Amount of SOL that would be received after fees
    /// * `Err(&str)` - Error message if curve is complete
    pub fn get_sell_price(&self, amount: u64, fee_basis_points: u64) -> Result<u64, &'static str> {
        if self.complete {
            return Err("Curve is complete");
        }

        if amount == 0 {
            return Ok(0);
        }

        // Calculate the proportional amount of virtual sol reserves to be received using u128
        let n: u128 = ((amount as u128) * (self.virtual_sol_reserves as u128))
            / ((self.virtual_token_reserves as u128) + (amount as u128));

        // Calculate the fee amount in the same units
        let a: u128 = (n * (fee_basis_points as u128)) / 10000;

        // Return the net amount after deducting the fee, converting back to u64
        Ok((n - a) as u64)
    }

    /// Calculates the current market cap in SOL
    pub fn get_market_cap_sol(&self) -> u64 {
        if self.virtual_token_reserves == 0 {
            return 0;
        }

        ((self.token_total_supply as u128) * (self.virtual_sol_reserves as u128)
            / (self.virtual_token_reserves as u128)) as u64
    }

    /// Calculates the final market cap in SOL after all tokens are sold
    ///
    /// # Arguments
    /// * `fee_basis_points` - Fee in basis points (1/100th of a percent)
    pub fn get_final_market_cap_sol(&self, fee_basis_points: u64) -> u64 {
        let total_sell_value: u128 =
            self.get_buy_out_price(self.real_token_reserves, fee_basis_points) as u128;
        let total_virtual_value: u128 = (self.virtual_sol_reserves as u128) + total_sell_value;
        let total_virtual_tokens: u128 =
            (self.virtual_token_reserves as u128) - (self.real_token_reserves as u128);

        if total_virtual_tokens == 0 {
            return 0;
        }

        ((self.token_total_supply as u128) * total_virtual_value / total_virtual_tokens) as u64
    }

    /// Calculates the price to buy out all remaining tokens
    ///
    /// # Arguments
    /// * `amount` - Amount of tokens to buy
    /// * `fee_basis_points` - Fee in basis points (1/100th of a percent)
    pub fn get_buy_out_price(&self, amount: u64, fee_basis_points: u64) -> u64 {
        // Get the effective amount of sol tokens
        let sol_tokens: u128 = if amount < self.real_sol_reserves {
            self.real_sol_reserves as u128
        } else {
            amount as u128
        };

        // Calculate total sell value
        let total_sell_value: u128 = (sol_tokens * (self.virtual_sol_reserves as u128))
            / ((self.virtual_token_reserves as u128) - sol_tokens)
            + 1;

        // Calculate fee
        let fee: u128 = (total_sell_value * (fee_basis_points as u128)) / 10000;

        // Return total including fee, converting back to u64
        (total_sell_value + fee) as u64
    }

    pub fn get_token_price(&self) -> f64 {
        let v_sol = self.virtual_sol_reserves as f64 / 100_000_000.0;
        let v_tokens = self.virtual_token_reserves as f64 / 100_000.0;
        let token_price = v_sol / v_tokens;
        token_price
    }
}
