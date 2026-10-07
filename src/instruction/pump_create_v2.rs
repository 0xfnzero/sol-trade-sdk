//! Pump create_v2. Explicit quote accounts are supplied from validated chain state.
use anyhow::{ensure, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::collections::HashMap;
pub struct PumpCreateV2Params<'a> {
    pub name: &'a str,
    pub symbol: &'a str,
    pub uri: &'a str,
    pub creator: Pubkey,
    pub mayhem: bool,
    pub creator_fee_bps: u64,
    pub holder_reward: bool,
}
pub fn build_pump_create_v2_instruction(
    accounts: &HashMap<String, Pubkey>,
    p: PumpCreateV2Params<'_>,
    remaining: &[AccountMeta],
) -> Result<Instruction> {
    ensure!(
        [0, 3, 4, 5, 8].contains(&remaining.len())
            && remaining.iter().enumerate().all(|(i, a)| !a.is_signer && a.is_writable == (i == 1)),
        "invalid quote creation accounts"
    );
    ensure!(!(remaining.len() >= 5 && p.mayhem), "mayhem pump coin quote not allowed");
    ensure!(p.creator_fee_bps <= 10000, "invalid creator fee");
    let mut data = [214, 144, 76, 236, 95, 139, 49, 180].to_vec();
    for s in [p.name, p.symbol, p.uri] {
        let n: u32 = s.len().try_into()?;
        data.extend_from_slice(&n.to_le_bytes());
        data.extend_from_slice(s.as_bytes());
    }
    data.extend_from_slice(p.creator.as_ref());
    data.extend_from_slice(&[u8::from(p.mayhem), 0]);
    data.extend_from_slice(&p.creator_fee_bps.to_le_bytes());
    data.push(u8::from(p.holder_reward));
    let mut keys = Vec::new();
    for (name, writable, signer) in [
        ("mint", true, true),
        ("mint_authority", false, false),
        ("bonding_curve", true, false),
        ("associated_bonding_curve", true, false),
        ("global", false, false),
        ("user", true, true),
        ("system_program", false, false),
        ("token_program", false, false),
        ("associated_token_program", false, false),
        ("mayhem_program_id", true, false),
        ("global_params", false, false),
        ("sol_vault", true, false),
        ("mayhem_state", true, false),
        ("mayhem_token_vault", true, false),
        ("event_authority", false, false),
        ("program", false, false),
    ] {
        let key =
            *accounts.get(name).ok_or_else(|| anyhow::anyhow!("missing account: {}", name))?;
        keys.push(if writable {
            AccountMeta::new(key, signer)
        } else {
            AccountMeta::new_readonly(key, signer)
        });
    }
    keys.extend_from_slice(remaining);
    Ok(Instruction {
        program_id: solana_sdk::pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        accounts: keys,
        data,
    })
}

pub struct PumpQuoteControl {
    pub admin: Pubkey,
    pub reserves_admin: Pubkey,
    pub mints: Vec<(Pubkey, u64)>,
}
/// Current QuoteControl header includes reserves_admin and reserved[32].
pub fn decode_pump_quote_control(data: &[u8]) -> Result<PumpQuoteControl> {
    ensure!(
        data.len() >= 108 && data[..8] == [56, 244, 35, 238, 193, 213, 162, 201],
        "invalid QuoteControl"
    );
    let count = u32::from_le_bytes(data[104..108].try_into()?) as usize;
    ensure!(count <= (data.len() - 108) / 40, "truncated QuoteControl");
    Ok(PumpQuoteControl {
        admin: Pubkey::new_from_array(data[8..40].try_into()?),
        reserves_admin: Pubkey::new_from_array(data[40..72].try_into()?),
        mints: (0..count)
            .map(|i| {
                let o = 108 + i * 40;
                (
                    Pubkey::new_from_array(data[o..o + 32].try_into().unwrap()),
                    u64::from_le_bytes(data[o + 32..o + 40].try_into().unwrap()),
                )
            })
            .collect(),
    })
}
