//! Compact Pump PDA/ATA derivation. Bare instructions require existing token accounts.
use anyhow::{ensure, Result};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
const PUMP: Pubkey = solana_sdk::pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
const AMM: Pubkey = solana_sdk::pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
const FEES: Pubkey = solana_sdk::pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
const ATA: Pubkey = solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const WSOL: Pubkey = solana_sdk::pubkey!("So11111111111111111111111111111111111111112");
fn pda(program: Pubkey, seed: &str, key: Option<Pubkey>) -> Pubkey {
    if let Some(k) = key {
        Pubkey::find_program_address(&[seed.as_bytes(), k.as_ref()], &program).0
    } else {
        Pubkey::find_program_address(&[seed.as_bytes()], &program).0
    }
}
fn ata(owner: Pubkey, mint: Pubkey, token: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), token.as_ref(), mint.as_ref()], &ATA).0
}
#[derive(Clone, Copy)]
pub struct PumpCompactAccountParams {
    pub user: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub buyback_recipient: Pubkey,
    pub cashback: bool,
    pub complete: bool,
}
pub fn derive_pump_v3_accounts(p: PumpCompactAccountParams) -> Result<HashMap<String, Pubkey>> {
    ensure!(!p.cashback, "cashback requires legacy trades");
    ensure!(!p.complete, "BondingCurveComplete");
    let curve = pda(PUMP, "bonding-curve", Some(p.base_mint));
    Ok(HashMap::from([
        ("base_mint".to_owned(), p.base_mint),
        ("quote_mint".to_owned(), p.quote_mint),
        ("base_token_program".to_owned(), p.base_token_program),
        ("quote_token_program".to_owned(), p.quote_token_program),
        ("bonding_curve".to_owned(), curve),
        ("associated_base_bonding_curve".to_owned(), ata(curve, p.base_mint, p.base_token_program)),
        (
            "associated_quote_bonding_curve".to_owned(),
            ata(curve, p.quote_mint, p.quote_token_program),
        ),
        ("user".to_owned(), p.user),
        ("associated_base_user".to_owned(), ata(p.user, p.base_mint, p.base_token_program)),
        ("associated_quote_user".to_owned(), ata(p.user, p.quote_mint, p.quote_token_program)),
        ("user_volume_accumulator".to_owned(), pda(PUMP, "user_volume_accumulator", Some(p.user))),
        ("fee_config".to_owned(), pda(FEES, "fee_config", Some(PUMP))),
        (
            "buyback_fee_recipient".to_owned(),
            if p.quote_mint == WSOL {
                p.buyback_recipient
            } else {
                ata(p.buyback_recipient, p.quote_mint, p.quote_token_program)
            },
        ),
        ("system_program".to_owned(), Pubkey::default()),
        ("event_authority".to_owned(), pda(PUMP, "__event_authority", None)),
        ("program".to_owned(), PUMP),
        ("global".to_owned(), pda(PUMP, "global", None)),
    ]))
}
pub fn derive_pump_swap_v2_accounts(
    p: PumpCompactAccountParams,
    pool: Pubkey,
    base_vault: Pubkey,
    quote_vault: Pubkey,
) -> Result<HashMap<String, Pubkey>> {
    ensure!(!p.cashback, "cashback requires legacy trades");
    Ok(HashMap::from([
        ("pool".to_owned(), pool),
        ("user".to_owned(), p.user),
        ("global_config".to_owned(), pda(AMM, "global_config", None)),
        ("base_mint".to_owned(), p.base_mint),
        ("quote_mint".to_owned(), p.quote_mint),
        ("user_base_token_account".to_owned(), ata(p.user, p.base_mint, p.base_token_program)),
        ("user_quote_token_account".to_owned(), ata(p.user, p.quote_mint, p.quote_token_program)),
        ("pool_base_token_account".to_owned(), base_vault),
        ("pool_quote_token_account".to_owned(), quote_vault),
        ("base_token_program".to_owned(), p.base_token_program),
        ("quote_token_program".to_owned(), p.quote_token_program),
        ("system_program".to_owned(), Pubkey::default()),
        ("user_volume_accumulator".to_owned(), pda(AMM, "user_volume_accumulator", Some(p.user))),
        ("fee_config".to_owned(), pda(FEES, "fee_config", Some(AMM))),
        (
            "buyback_fee_recipient".to_owned(),
            ata(p.buyback_recipient, p.quote_mint, p.quote_token_program),
        ),
        ("event_authority".to_owned(), pda(AMM, "__event_authority", None)),
        ("program".to_owned(), AMM),
    ]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpMultiHopVenue {
    Curve,
    Pool,
}
#[derive(Clone, Copy)]
pub struct PumpMultiHop {
    pub venue: PumpMultiHopVenue,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub address: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub mayhem: bool,
    pub cashback: bool,
    pub complete: bool,
    pub index: u16,
    pub creator: Pubkey,
}
/// Derives bare route accounts from decoded venue state. Four hops require v0 + ALT.
pub fn derive_pump_multi_hop_accounts(
    user: Pubkey,
    input_mint: Pubkey,
    output_mint: Pubkey,
    buyback_recipient: Pubkey,
    hops: &[PumpMultiHop],
    use_v0_with_alt: bool,
) -> Result<(HashMap<String, Pubkey>, Vec<solana_sdk::instruction::AccountMeta>)> {
    use solana_sdk::instruction::AccountMeta;
    ensure!(
        !hops.is_empty() && (hops.len() < 4 || use_v0_with_alt),
        "route requires hops and v0 with ALT for four or more hops"
    );
    let mut current = input_mint;
    let mut side = None;
    let mut remaining = Vec::new();
    for (i, h) in hops.iter().enumerate() {
        ensure!(!h.mayhem && h.base_mint != h.quote_mint, "invalid multi-hop venue");
        let buy = current == h.quote_mint;
        ensure!(buy || current == h.base_mint, "discontinuous route");
        ensure!(side.is_none_or(|s| s == buy), "mixed route direction");
        side = Some(buy);
        ensure!(
            !h.cashback || i == if buy { 0 } else { hops.len() - 1 },
            "cashback must be at currency endpoint"
        );
        match h.venue {
            PumpMultiHopVenue::Curve => {
                let curve = pda(PUMP, "bonding-curve", Some(h.base_mint));
                ensure!(
                    !h.complete
                        && h.address == curve
                        && h.base_vault == ata(curve, h.base_mint, h.base_token_program)
                        && h.quote_vault == ata(curve, h.quote_mint, h.quote_token_program),
                    "invalid curve accounts"
                );
            }
            PumpMultiHopVenue::Pool => {
                let authority = pda(PUMP, "pool-authority", Some(h.base_mint));
                let pool = Pubkey::find_program_address(
                    &[
                        b"pool",
                        &[0, 0],
                        authority.as_ref(),
                        h.base_mint.as_ref(),
                        h.quote_mint.as_ref(),
                    ],
                    &AMM,
                )
                .0;
                ensure!(
                    h.index == 0 && h.creator == authority && h.address == pool,
                    "noncanonical Pump pool"
                );
            }
        }
        remaining.extend([
            AccountMeta::new_readonly(h.base_mint, false),
            AccountMeta::new_readonly(h.quote_mint, false),
            AccountMeta::new(h.address, false),
            AccountMeta::new(h.base_vault, false),
            AccountMeta::new(h.quote_vault, false),
        ]);
        current = if buy { h.base_mint } else { h.quote_mint };
    }
    ensure!(current == output_mint, "wrong output mint");
    let buy = side.unwrap();
    let first = hops.first().unwrap();
    let last = hops.last().unwrap();
    let currency = if buy { first } else { last };
    let accounts = HashMap::from([
        ("user".into(), user),
        (
            "user_in_token_account".into(),
            ata(
                user,
                input_mint,
                if buy { first.quote_token_program } else { first.base_token_program },
            ),
        ),
        (
            "user_out_token_account".into(),
            ata(
                user,
                output_mint,
                if buy { last.base_token_program } else { last.quote_token_program },
            ),
        ),
        ("global_config".into(), pda(AMM, "global_config", None)),
        ("fee_config".into(), pda(FEES, "fee_config", Some(AMM))),
        ("user_volume_accumulator".into(), pda(AMM, "user_volume_accumulator", Some(user))),
        (
            "buyback_fee_recipient".into(),
            ata(buyback_recipient, currency.quote_mint, currency.quote_token_program),
        ),
        (
            "token_program".into(),
            solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
        ),
        (
            "token_2022_program".into(),
            solana_sdk::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"),
        ),
        ("system_program".into(), Pubkey::default()),
        ("event_authority".into(), pda(AMM, "__event_authority", None)),
        ("program".into(), AMM),
        ("pump_program".into(), PUMP),
        ("pump_global".into(), pda(PUMP, "global", None)),
        ("pump_fee_config".into(), pda(FEES, "fee_config", Some(PUMP))),
        ("pump_event_authority".into(), pda(PUMP, "__event_authority", None)),
    ]);
    Ok((accounts, remaining))
}

/// Extra create_v2 roles for an unlisted Pump coin quote from decoded state.
pub fn derive_pump_coin_quote_create_accounts(
    new_mint: Pubkey,
    quote: PumpMultiHop,
    depth: u8,
    max_depth: u8,
    listed_quote_mints: &[Pubkey],
) -> Result<Vec<solana_sdk::instruction::AccountMeta>> {
    use solana_sdk::instruction::AccountMeta;
    ensure!(depth < max_depth, "CurveDepthExceeded");
    ensure!(
        depth > 0
            || quote.quote_mint == WSOL
            || quote.quote_mint
                == solana_sdk::pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
            || listed_quote_mints.contains(&quote.quote_mint),
        "QuoteBondingCurveNotEligible"
    );
    derive_pump_multi_hop_accounts(
        new_mint,
        quote.quote_mint,
        quote.base_mint,
        new_mint,
        &[quote],
        false,
    )?;
    let curve = pda(PUMP, "bonding-curve", Some(new_mint));
    let mut keys = vec![
        quote.base_mint,
        ata(curve, quote.base_mint, quote.base_token_program),
        quote.base_token_program,
        pda(PUMP, "quote-control", None),
        pda(PUMP, "bonding-curve", Some(quote.base_mint)),
    ];
    if quote.venue == PumpMultiHopVenue::Pool {
        keys.extend([quote.address, quote.base_vault, quote.quote_vault]);
    }
    Ok(keys
        .into_iter()
        .enumerate()
        .map(|(i, key)| {
            if i == 1 {
                AccountMeta::new(key, false)
            } else {
                AccountMeta::new_readonly(key, false)
            }
        })
        .collect())
}
