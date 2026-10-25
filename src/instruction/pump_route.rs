//! Choose the official Pump route without turning sequential hop quotes into a route quote.
use super::{
    pump_compact_accounts::{derive_pump_multi_hop_accounts, PumpMultiHop},
    pump_upgrade::build_pump_amm_multi_hop_swap_instruction,
};
use anyhow::{ensure, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

/// Independent of the Pump.fun single-trade instruction version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PumpRouteExecution {
    /// Prefer official execution when supported and an explicit final limit is available.
    #[default]
    Auto,
    /// Require official execution; unsupported routes return an error.
    Official,
    /// Keep independent swap instructions in one atomic transaction.
    Sequential,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PumpRouteOptions {
    pub execution: PumpRouteExecution,
    /// Fresh final recipient credit for the official route, in raw output units.
    /// Sequential hop minima are not a quote for official execution.
    pub minimum_output_amount: Option<u64>,
    /// Recipient wallet from the current PumpSwap GlobalConfig.
    pub buyback_recipient: Option<Pubkey>,
}

/// `None` selects the existing sequential builder. Missing state, mixed venues,
/// splits and unsupported official layouts must be represented by `hops = None`.
/// A failed simulation/submission must never be retried on a different route here.
pub fn select_pump_route(
    options: PumpRouteOptions,
    user: Pubkey,
    input: Pubkey,
    output: Pubkey,
    amount: u64,
    hops: Option<&[PumpMultiHop]>,
) -> Result<Option<Instruction>> {
    use PumpRouteExecution::*;
    ensure!(amount > 0, "Pump route input must be nonzero");
    if options.execution == Sequential {
        return Ok(None);
    }
    if let Some(minimum) = options.minimum_output_amount {
        ensure!(minimum > 0, "Official Pump route minimum must be nonzero");
    }
    if let Some(recipient) = options.buyback_recipient {
        ensure!(recipient != Pubkey::default(), "Missing Pump buyback recipient");
    }
    if options.execution == Auto
        && (options.minimum_output_amount.is_none()
            || options.buyback_recipient.is_none()
            || hops.is_none_or(|h| !(2..=3).contains(&h.len())))
    {
        return Ok(None);
    }
    let candidate = (|| {
        let hops = hops.ok_or_else(|| anyhow::anyhow!("Route is not an official Pump chain"))?;
        ensure!((2..=3).contains(&hops.len()), "Automatic Pump routes support two or three hops");
        ensure!(user != Pubkey::default() && input != output, "Invalid Pump route endpoints");
        let minimum = options
            .minimum_output_amount
            .ok_or_else(|| anyhow::anyhow!("Provide an official route minimum output"))?;
        let recipient = options
            .buyback_recipient
            .ok_or_else(|| anyhow::anyhow!("Provide the current Pump buyback recipient"))?;
        let mut seen = std::collections::HashSet::from([input]);
        let mut current = input;
        let mut previous_program = None;
        for hop in hops {
            for program in [hop.base_token_program, hop.quote_token_program] {
                ensure!(
                    program == crate::constants::TOKEN_PROGRAM
                        || program == crate::constants::TOKEN_PROGRAM_2022,
                    "Unknown Pump route token program"
                );
            }
            let buy = current == hop.quote_mint;
            let (next, input_program, output_program) = if buy {
                (hop.base_mint, hop.quote_token_program, hop.base_token_program)
            } else {
                ensure!(current == hop.base_mint, "Disconnected Pump route");
                (hop.quote_mint, hop.base_token_program, hop.quote_token_program)
            };
            ensure!(
                previous_program.is_none_or(|p| p == input_program),
                "Inconsistent intermediate token program"
            );
            ensure!(seen.insert(next), "Pump route contains an asset cycle");
            ensure!(
                hop.address != Pubkey::default()
                    && hop.base_vault != Pubkey::default()
                    && hop.quote_vault != Pubkey::default()
                    && hop.base_vault != hop.quote_vault,
                "Incomplete Pump route venue"
            );
            previous_program = Some(output_program);
            current = next;
        }
        let (accounts, remaining) =
            derive_pump_multi_hop_accounts(user, input, output, recipient, hops, false)?;
        build_pump_amm_multi_hop_swap_instruction(&accounts, amount, minimum, &remaining)
    })();
    match candidate {
        Ok(ix) => Ok(Some(ix)),
        Err(_) if options.execution == Auto => Ok(None),
        Err(error) => Err(error),
    }
}
