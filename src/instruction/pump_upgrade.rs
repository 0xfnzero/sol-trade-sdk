//! Compact Pump trades and sweeps. Official IDL commit 8cda1fa.
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::collections::HashMap;
struct Spec {
    program: &'static str,
    disc: [u8; 8],
    accounts: &'static [(&'static str, bool, bool)],
    args: usize,
    partial: bool,
}
fn spec(name: &str) -> Option<Spec> {
    Some(match name {
        "pump_buy_v3" => Spec {
            program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            disc: [7, 5, 29, 196, 245, 23, 101, 80],
            accounts: &[
                ("global", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("bonding_curve", true, false),
                ("associated_base_bonding_curve", true, false),
                ("associated_quote_bonding_curve", true, false),
                ("user", true, true),
                ("associated_base_user", true, false),
                ("associated_quote_user", true, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("system_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: true,
        },
        "pump_buy_exact_quote_in_v3" => Spec {
            program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            disc: [225, 247, 80, 30, 213, 179, 132, 136],
            accounts: &[
                ("global", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("bonding_curve", true, false),
                ("associated_base_bonding_curve", true, false),
                ("associated_quote_bonding_curve", true, false),
                ("user", true, true),
                ("associated_base_user", true, false),
                ("associated_quote_user", true, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("system_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: true,
        },
        "pump_sell_v3" => Spec {
            program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            disc: [28, 146, 222, 119, 38, 196, 105, 213],
            accounts: &[
                ("global", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("bonding_curve", true, false),
                ("associated_base_bonding_curve", true, false),
                ("associated_quote_bonding_curve", true, false),
                ("user", true, true),
                ("associated_base_user", true, false),
                ("associated_quote_user", true, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("system_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: false,
        },
        "pump_sweep_creator_fee" => Spec {
            program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            disc: [32, 246, 191, 52, 8, 201, 73, 186],
            accounts: &[
                ("payer", true, true),
                ("global", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("quote_token_program", false, false),
                ("associated_token_program", false, false),
                ("system_program", false, false),
                ("bonding_curve", true, false),
                ("associated_quote_bonding_curve", true, false),
                ("recipient", true, false),
                ("associated_quote_recipient", true, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 0,
            partial: false,
        },
        "pump_sweep_protocol_fee" => Spec {
            program: "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            disc: [8, 48, 190, 7, 182, 68, 183, 229],
            accounts: &[
                ("payer", true, true),
                ("global", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("quote_token_program", false, false),
                ("associated_token_program", false, false),
                ("system_program", false, false),
                ("bonding_curve", true, false),
                ("associated_quote_bonding_curve", true, false),
                ("recipient", true, false),
                ("associated_quote_recipient", true, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 0,
            partial: false,
        },
        "pump_amm_buy_v2" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [184, 23, 238, 97, 103, 197, 211, 61],
            accounts: &[
                ("pool", true, false),
                ("user", true, true),
                ("global_config", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("user_base_token_account", true, false),
                ("user_quote_token_account", true, false),
                ("pool_base_token_account", true, false),
                ("pool_quote_token_account", true, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("system_program", false, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: false,
        },
        "pump_amm_buy_exact_quote_in_v2" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [194, 171, 28, 70, 104, 77, 91, 47],
            accounts: &[
                ("pool", true, false),
                ("user", true, true),
                ("global_config", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("user_base_token_account", true, false),
                ("user_quote_token_account", true, false),
                ("pool_base_token_account", true, false),
                ("pool_quote_token_account", true, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("system_program", false, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: false,
        },
        "pump_amm_sell_v2" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [93, 246, 130, 60, 231, 233, 64, 178],
            accounts: &[
                ("pool", true, false),
                ("user", true, true),
                ("global_config", false, false),
                ("base_mint", false, false),
                ("quote_mint", false, false),
                ("user_base_token_account", true, false),
                ("user_quote_token_account", true, false),
                ("pool_base_token_account", true, false),
                ("pool_quote_token_account", true, false),
                ("base_token_program", false, false),
                ("quote_token_program", false, false),
                ("system_program", false, false),
                ("user_volume_accumulator", true, false),
                ("fee_config", false, false),
                ("buyback_fee_recipient", true, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 2,
            partial: false,
        },
        "pump_amm_multi_hop_swap" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [43, 100, 73, 19, 233, 246, 111, 148],
            accounts: &[
                ("user", true, true),
                ("user_in_token_account", true, false),
                ("user_out_token_account", true, false),
                ("global_config", false, false),
                ("fee_config", false, false),
                ("user_volume_accumulator", true, false),
                ("buyback_fee_recipient", true, false),
                ("token_program", false, false),
                ("token_2022_program", false, false),
                ("system_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
                ("pump_program", false, false),
                ("pump_global", false, false),
                ("pump_fee_config", false, false),
                ("pump_event_authority", false, false),
            ],
            args: 2,
            partial: false,
        },
        "pump_amm_sweep_creator_fee" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [32, 246, 191, 52, 8, 201, 73, 186],
            accounts: &[
                ("payer", true, true),
                ("global_config", false, false),
                ("pool", true, false),
                ("quote_mint", false, false),
                ("quote_token_program", false, false),
                ("pool_quote_token_account", true, false),
                ("recipient", false, false),
                ("recipient_token_account", true, false),
                ("system_program", false, false),
                ("associated_token_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 0,
            partial: false,
        },
        "pump_amm_sweep_protocol_fee" => Spec {
            program: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            disc: [8, 48, 190, 7, 182, 68, 183, 229],
            accounts: &[
                ("payer", true, true),
                ("global_config", false, false),
                ("pool", true, false),
                ("quote_mint", false, false),
                ("quote_token_program", false, false),
                ("pool_quote_token_account", true, false),
                ("recipient", false, false),
                ("recipient_token_account", true, false),
                ("system_program", false, false),
                ("associated_token_program", false, false),
                ("event_authority", false, false),
                ("program", false, false),
            ],
            args: 0,
            partial: false,
        },
        _ => return None,
    })
}
/// Bare instruction; caller validates state and supplies existing token accounts.
pub fn build_pump_upgrade_instruction(
    name: &str,
    accounts: &HashMap<String, Pubkey>,
    amounts: &[u64],
    partial_fill: Option<bool>,
    remaining: &[AccountMeta],
) -> Result<Instruction, anyhow::Error> {
    let s = spec(name).ok_or_else(|| anyhow::anyhow!("unsupported Pump upgrade instruction"))?;
    anyhow::ensure!(amounts.len() == s.args, "invalid u64 argument count");
    anyhow::ensure!(amounts.first() != Some(&0), "input amount must be positive");
    if name == "pump_amm_multi_hop_swap" {
        anyhow::ensure!(
            amounts[1] > 0 && remaining.len() >= 5 && remaining.len() % 5 == 0,
            "invalid multi-hop route or minimum output"
        );
    } else {
        anyhow::ensure!(
            remaining.is_empty(),
            "compact trades and sweeps take no remaining accounts"
        );
    }
    anyhow::ensure!(partial_fill.is_none() || s.partial, "invalid partial fill");
    let mut keys = Vec::with_capacity(s.accounts.len() + remaining.len());
    for &(name, writable, signer) in s.accounts {
        let key =
            *accounts.get(name).ok_or_else(|| anyhow::anyhow!("missing account: {}", name))?;
        keys.push(if writable {
            AccountMeta::new(key, signer)
        } else {
            AccountMeta::new_readonly(key, signer)
        });
    }
    anyhow::ensure!(
        remaining.iter().enumerate().all(|(i, a)| !a.is_signer && a.is_writable == (i % 5 >= 2)),
        "invalid hop account flags"
    );
    keys.extend_from_slice(remaining);
    let mut data = s.disc.to_vec();
    for amount in amounts {
        data.extend_from_slice(&amount.to_le_bytes());
    }
    if let Some(fill) = partial_fill {
        data.push(u8::from(fill));
    }
    Ok(Instruction { program_id: s.program.parse()?, accounts: keys, data })
}
pub fn build_pump_buy_v3_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
    partial_fill: Option<bool>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_buy_v3", accounts, &[amount, limit], partial_fill, &[])
}
pub fn build_pump_buy_exact_quote_in_v3_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
    partial_fill: Option<bool>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction(
        "pump_buy_exact_quote_in_v3",
        accounts,
        &[amount, limit],
        partial_fill,
        &[],
    )
}
pub fn build_pump_sell_v3_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_sell_v3", accounts, &[amount, limit], None, &[])
}
pub fn build_pump_sweep_creator_fee_instruction(
    accounts: &HashMap<String, Pubkey>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_sweep_creator_fee", accounts, &[], None, &[])
}
pub fn build_pump_sweep_protocol_fee_instruction(
    accounts: &HashMap<String, Pubkey>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_sweep_protocol_fee", accounts, &[], None, &[])
}
pub fn build_pump_amm_buy_v2_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_amm_buy_v2", accounts, &[amount, limit], None, &[])
}
pub fn build_pump_amm_buy_exact_quote_in_v2_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction(
        "pump_amm_buy_exact_quote_in_v2",
        accounts,
        &[amount, limit],
        None,
        &[],
    )
}
pub fn build_pump_amm_sell_v2_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_amm_sell_v2", accounts, &[amount, limit], None, &[])
}
pub fn build_pump_amm_multi_hop_swap_instruction(
    accounts: &HashMap<String, Pubkey>,
    amount: u64,
    limit: u64,
    remaining: &[AccountMeta],
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction(
        "pump_amm_multi_hop_swap",
        accounts,
        &[amount, limit],
        None,
        remaining,
    )
}
pub fn build_pump_amm_sweep_creator_fee_instruction(
    accounts: &HashMap<String, Pubkey>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_amm_sweep_creator_fee", accounts, &[], None, &[])
}
pub fn build_pump_amm_sweep_protocol_fee_instruction(
    accounts: &HashMap<String, Pubkey>,
) -> Result<Instruction, anyhow::Error> {
    build_pump_upgrade_instruction("pump_amm_sweep_protocol_fee", accounts, &[], None, &[])
}
