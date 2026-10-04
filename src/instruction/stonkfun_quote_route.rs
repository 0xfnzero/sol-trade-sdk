use super::{
    stonkfun::{amm_v4_is_coin_in, cpmm_is_base_in, normalize_native_sol},
    token_account_setup,
};
use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::core::{
        params::{
            DexParamEnum, StonkFunQuoteHop, StonkFunQuoteRoute, StonkFunQuoteVenue, SwapParams,
        },
        traits::InstructionBuilder,
    },
};
use anyhow::{anyhow, bail, Result};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};
use std::collections::HashMap;

pub(crate) struct PlannedHop<'a> {
    pub hop: &'a StonkFunQuoteHop,
    pub amount: u64,
    pub input: Pubkey,
    pub output: Pubkey,
    pub input_program: Pubkey,
    pub output_program: Pubkey,
}
pub(crate) struct RoutePlan<'a> {
    pub hops: Vec<PlannedHop<'a>>,
    pub minimum_credit: u64,
}

fn pair(hop: &StonkFunQuoteHop) -> Result<(Pubkey, Pubkey)> {
    use StonkFunQuoteVenue::*;
    let input = normalize_native_sol(hop.input_mint);
    let output = normalize_native_sol(hop.output_mint);
    let (a, b, pa, pb) = match &hop.venue {
        RaydiumCpmm(p) => (p.base_mint, p.quote_mint, p.base_token_program, p.quote_token_program),
        RaydiumAmmV4(p) => (
            p.coin_mint,
            p.pc_mint,
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
        ),
        RaydiumClmm(p) => (p.token_0_mint, p.token_1_mint, p.token_0_program, p.token_1_program),
        OrcaWhirlpool(p) => (p.mint_a, p.mint_b, p.token_program_a, p.token_program_b),
        MeteoraDlmm(p) => (p.token_x_mint, p.token_y_mint, p.token_x_program, p.token_y_program),
    };
    if input == a && output == b {
        Ok((pa, pb))
    } else if input == b && output == a {
        Ok((pb, pa))
    } else {
        bail!("StonkFun quote route hop pair does not match its pool")
    }
}

pub(crate) fn plan_route(
    route: &StonkFunQuoteRoute,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    slippage: u64,
    is_buy: bool,
) -> Result<RoutePlan<'_>> {
    let source = normalize_native_sol(input_mint);
    let target = normalize_native_sol(output_mint);
    if slippage > 10_000 {
        bail!("Quote route slippage exceeds 10000 basis points")
    }
    let steps = if is_buy { &route.buy_hops } else { &route.sell_hops };
    if steps.is_empty() {
        bail!("StonkFun quote route has no hops for this direction")
    }
    if source == target || amount == 0 {
        bail!("StonkFun quote route needs distinct endpoints and nonzero input")
    }
    let mut balances = HashMap::from([(source, amount)]);
    let mut programs = HashMap::new();
    let mut edges: HashMap<Pubkey, Vec<Pubkey>> = HashMap::new();
    let mut planned = Vec::with_capacity(steps.len());
    for hop in steps {
        let input = normalize_native_sol(hop.input_mint);
        let output = normalize_native_sol(hop.output_mint);
        if input == output || input == target || output == source {
            bail!("StonkFun quote route contains a cycle or spends its final output")
        }
        let mut pending = vec![output];
        let mut visited = std::collections::HashSet::new();
        while let Some(mint) = pending.pop() {
            if mint == input {
                bail!("StonkFun quote route contains an intermediate mint cycle")
            }
            if visited.insert(mint) {
                pending.extend(edges.get(&mint).into_iter().flatten().copied());
            }
        }
        edges.entry(input).or_default().push(output);
        let (input_program, output_program) = pair(hop)?;
        for (mint, program) in [(input, input_program), (output, output_program)] {
            if program != crate::constants::TOKEN_PROGRAM
                && program != crate::constants::TOKEN_PROGRAM_2022
            {
                bail!("Unsupported quote route token program")
            }
            if let Some(previous) = programs.insert(mint, program) {
                if previous != program {
                    bail!("Inconsistent token programs for quote route mint")
                }
            }
        }
        let available = balances.get(&input).copied().unwrap_or(0);
        let spend = hop.input_amount.unwrap_or(available);
        if spend == 0 || spend > available {
            bail!("StonkFun quote route hop overspends guaranteed balance")
        }
        if hop.minimum_amount_out == 0 {
            bail!("StonkFun quote route requires nonzero min-out protection")
        }
        let minimum_credit = match &hop.venue {
            StonkFunQuoteVenue::RaydiumCpmm(p) => {
                let actual = crate::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
                    p,
                    cpmm_is_base_in(p, input, output)?,
                    spend,
                    slippage,
                )?
                .min_amount_out;
                if hop.minimum_amount_out > actual {
                    bail!("CPMM route min-out exceeds the SDK's current protected net quote")
                }
                hop.minimum_amount_out
            }
            StonkFunQuoteVenue::RaydiumAmmV4(p) => {
                let actual = crate::utils::calc::raydium_amm_v4::compute_swap_amount_for_pool(
                    p,
                    amm_v4_is_coin_in(p, input, output)?,
                    spend,
                    slippage,
                )?
                .min_amount_out;
                if hop.minimum_amount_out > actual {
                    bail!("AMM v4 route min-out exceeds the SDK's current quote")
                }
                hop.minimum_amount_out
            }
            _ => {
                let fee = if output_program == crate::constants::TOKEN_PROGRAM_2022 {
                    hop.output_transfer_fee.ok_or_else(|| {
                        anyhow!("Token-2022 quote route output requires an active fee snapshot")
                    })?
                } else {
                    crate::trading::core::params::TokenTransferFee::default()
                };
                if fee.basis_points > 10_000 {
                    bail!("Invalid quote route fee basis points")
                }
                // swap_v2/swap2 enforce the recipient's post-transfer-fee
                // threshold. Deducting the fee again would understate credit.
                hop.minimum_amount_out
            }
        };
        if minimum_credit == 0 {
            bail!("Quote route produces zero protected net output")
        }
        balances.insert(input, available - spend);
        let credit = balances
            .get(&output)
            .copied()
            .unwrap_or(0)
            .checked_add(minimum_credit)
            .ok_or_else(|| anyhow!("Quote route credit overflow"))?;
        balances.insert(output, credit);
        planned.push(PlannedHop {
            hop,
            amount: spend,
            input,
            output,
            input_program,
            output_program,
        });
    }
    if balances.get(&source).copied().unwrap_or(0) != 0 {
        bail!("Quote route does not spend the full requested input")
    }
    if balances.iter().any(|(mint, balance)| *mint != target && *balance != 0) {
        bail!("Quote route leaves guaranteed intermediate credit unconverted")
    }
    if planned.last().map(|h| h.output) != Some(target) {
        bail!("Quote route does not end in the requested output mint")
    }
    let minimum_credit = balances.get(&target).copied().unwrap_or(0);
    if minimum_credit == 0 {
        bail!("Quote route cannot fund its final output")
    }
    Ok(RoutePlan { hops: planned, minimum_credit })
}

pub(crate) async fn build_quote_route(
    params: &SwapParams,
    route: &StonkFunQuoteRoute,
    is_buy: bool,
) -> Result<Vec<Instruction>> {
    let amount =
        params.input_amount.ok_or_else(|| anyhow!("Quote route requires an exact input amount"))?;
    let plan = plan_route(
        route,
        params.input_mint,
        params.output_mint,
        amount,
        params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        is_buy,
    )?;
    if params.fixed_output_amount.is_some_and(|minimum| minimum > plan.minimum_credit) {
        bail!("Quote route cannot guarantee the requested final output; refresh its hop quotes")
    }
    let source = normalize_native_sol(params.input_mint);
    let target = normalize_native_sol(params.output_mint);
    for hop in &plan.hops {
        if hop.input == source
            && params.input_token_program.is_some_and(|program| program != hop.input_program)
        {
            bail!("Quote route input token program does not match the requested endpoint")
        }
        if hop.output == target
            && params.output_token_program.is_some_and(|program| program != hop.output_program)
        {
            bail!("Quote route output token program does not match the requested endpoint")
        }
    }
    let payer = params.payer.pubkey();
    let mut instructions = Vec::new();
    if params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT && !params.create_input_mint_ata {
        instructions.extend(crate::trading::common::fund_existing_wsol(&payer, amount));
    }
    let mut created = std::collections::HashSet::new();
    // Set up every mint once, wrap the requested SOL once, and never close a bridge ATA.
    for hop in &plan.hops {
        for (mint, program) in [(hop.input, hop.input_program), (hop.output, hop.output_program)] {
            let create = if mint == source {
                params.create_input_mint_ata
            } else if mint == target {
                params.create_output_mint_ata
            } else {
                params.create_input_mint_ata || params.create_output_mint_ata
            };
            if create && created.insert(mint) {
                if mint == source && params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT {
                    token_account_setup::push_create_or_wrap_user_token_account(
                        &mut instructions,
                        &payer,
                        &mint,
                        &program,
                        amount,
                        params.open_seed_optimize,
                    )
                } else {
                    token_account_setup::push_create_user_token_account(
                        &mut instructions,
                        &payer,
                        &mint,
                        &program,
                        params.open_seed_optimize,
                    )
                }
            }
        }
    }
    for hop in &plan.hops {
        let mut leg = params.clone();
        leg.input_mint = hop.input;
        leg.output_mint = hop.output;
        leg.input_amount = Some(hop.amount);
        leg.input_token_program = Some(hop.input_program);
        leg.output_token_program = Some(hop.output_program);
        leg.create_input_mint_ata = false;
        leg.create_output_mint_ata = false;
        leg.close_input_mint_ata = false;
        leg.close_output_mint_ata = false;
        use StonkFunQuoteVenue::*;
        let ixs = match &hop.hop.venue {
            RaydiumCpmm(p) => {
                leg.protocol_params = DexParamEnum::RaydiumCpmm(p.clone());
                leg.fixed_output_amount = None;
                super::raydium_cpmm::RaydiumCpmmInstructionBuilder
                    .build_buy_instructions(&leg)
                    .await?
            }
            RaydiumAmmV4(p) => {
                leg.protocol_params = DexParamEnum::RaydiumAmmV4(p.clone());
                leg.fixed_output_amount = None;
                super::raydium_amm_v4::RaydiumAmmV4InstructionBuilder
                    .build_buy_instructions(&leg)
                    .await?
            }
            RaydiumClmm(p) => {
                leg.protocol_params = DexParamEnum::RaydiumClmm(p.clone());
                leg.fixed_output_amount = Some(hop.hop.minimum_amount_out);
                super::raydium_clmm::RaydiumClmmInstructionBuilder
                    .build_buy_instructions(&leg)
                    .await?
            }
            OrcaWhirlpool(p) => {
                leg.protocol_params = DexParamEnum::OrcaWhirlpool(p.clone());
                leg.fixed_output_amount = Some(hop.hop.minimum_amount_out);
                super::whirlpool::WhirlpoolInstructionBuilder.build_buy_instructions(&leg).await?
            }
            MeteoraDlmm(p) => {
                leg.protocol_params = DexParamEnum::MeteoraDlmm(p.clone());
                leg.fixed_output_amount = Some(hop.hop.minimum_amount_out);
                super::meteora_dlmm::MeteoraDlmmInstructionBuilder
                    .build_buy_instructions(&leg)
                    .await?
            }
        };
        instructions.extend(ixs);
    }
    if params.close_input_mint_ata {
        token_account_setup::push_close_wsol_if_needed(&mut instructions, &payer, &source)
    }
    if params.close_output_mint_ata {
        token_account_setup::push_close_wsol_if_needed(&mut instructions, &payer, &target)
    }
    Ok(instructions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swqos::TradeType;
    use crate::trading::core::params::{
        MeteoraDlmmParams, RaydiumClmmParams, StonkFunViaSolParams, TokenTransferFee,
        WhirlpoolParams,
    };
    fn pk(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }
    fn whirlpool(input: Pubkey, output: Pubkey) -> StonkFunQuoteVenue {
        StonkFunQuoteVenue::OrcaWhirlpool(WhirlpoolParams::new(
            pk(1),
            input,
            output,
            pk(2),
            pk(3),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            vec![pk(4), pk(5), pk(6)],
        ))
    }
    fn clmm(input: Pubkey, output: Pubkey) -> StonkFunQuoteVenue {
        StonkFunQuoteVenue::RaydiumClmm(RaydiumClmmParams::new(
            pk(10),
            pk(11),
            pk(12),
            input,
            output,
            pk(13),
            pk(14),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            vec![pk(15)],
        ))
    }
    fn dlmm(input: Pubkey, output: Pubkey) -> StonkFunQuoteVenue {
        StonkFunQuoteVenue::MeteoraDlmm(MeteoraDlmmParams::new(
            pk(20),
            pk(21),
            pk(22),
            input,
            output,
            pk(23),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            vec![pk(24)],
        ))
    }
    #[test]
    fn rejects_intermediate_cycles_and_stranded_branch_credit() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let a = pk(30);
        let b = pk(31);
        let quote = pk(32);
        let cyclic = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(whirlpool(sol, a), sol, a, 20),
            StonkFunQuoteHop::exact_in(clmm(a, b), a, b, 30),
            StonkFunQuoteHop::exact_in(dlmm(b, a), b, a, 40),
            StonkFunQuoteHop::exact_in(whirlpool(a, quote), a, quote, 50),
        ]);
        assert!(cyclic
            .preview(sol, quote, 10, 100, true)
            .unwrap_err()
            .to_string()
            .contains("intermediate mint cycle"));
        let stranded = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(whirlpool(sol, a), sol, a, 20).with_input_amount(4),
            StonkFunQuoteHop::exact_in(whirlpool(sol, quote), sol, quote, 40),
        ]);
        assert!(stranded
            .preview(sol, quote, 10, 100, true)
            .unwrap_err()
            .to_string()
            .contains("intermediate credit"));
    }
    #[test]
    fn plans_split_merge_and_multihop_without_using_existing_wallet_balances() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let usdc = pk(30);
        let quote = pk(31);
        let route = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(whirlpool(sol, quote), sol, quote, 20).with_input_amount(4),
            StonkFunQuoteHop::exact_in(clmm(sol, usdc), sol, usdc, 30),
            StonkFunQuoteHop::exact_in(dlmm(usdc, quote), usdc, quote, 40),
        ]);
        let plan = plan_route(&route, sol, quote, 10, 100, true).unwrap();
        assert_eq!(plan.hops.iter().map(|h| h.amount).collect::<Vec<_>>(), vec![4, 6, 30]);
        assert_eq!(plan.minimum_credit, 60);
        let preview = route.preview(sol, quote, 10, 100, true).unwrap();
        assert_eq!(preview.minimum_output_credit, 60);
        assert!(route.preview(sol, quote, 10, 10_001, true).is_err());
        assert_eq!(
            preview.hops.iter().map(|hop| hop.input_amount).collect::<Vec<_>>(),
            vec![4, 6, 30]
        );
        let mut bad = route.clone();
        bad.buy_hops[2].input_amount = Some(31);
        assert!(plan_route(&bad, sol, quote, 10, 100, true)
            .err()
            .unwrap()
            .to_string()
            .contains("overspends"));
    }
    #[test]
    fn rejects_missing_direction_disconnected_pairs_zero_threshold_and_unspent_input() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = pk(31);
        let mut route = StonkFunQuoteRoute::buy(vec![StonkFunQuoteHop::exact_in(
            whirlpool(sol, quote),
            sol,
            quote,
            10,
        )]);
        assert!(plan_route(&route, quote, sol, 10, 100, false).is_err());
        route.buy_hops[0].input_amount = Some(9);
        assert!(plan_route(&route, sol, quote, 10, 100, true).is_err());
        route.buy_hops[0].input_amount = None;
        route.buy_hops[0].minimum_amount_out = 0;
        assert!(plan_route(&route, sol, quote, 10, 100, true).is_err());
        route.buy_hops[0].minimum_amount_out = 10;
        route.buy_hops[0].input_mint = pk(33);
        assert!(plan_route(&route, sol, quote, 10, 100, true).is_err());
    }
    #[test]
    fn token_2022_credit_requires_fee_snapshot_and_uses_net_amount() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = pk(31);
        let mut venue = whirlpool(sol, quote);
        if let StonkFunQuoteVenue::OrcaWhirlpool(p) = &mut venue {
            p.token_program_b = crate::constants::TOKEN_PROGRAM_2022;
        }
        let mut route =
            StonkFunQuoteRoute::buy(vec![StonkFunQuoteHop::exact_in(venue, sol, quote, 1000)]);
        assert!(plan_route(&route, sol, quote, 10, 100, true).is_err());
        route.buy_hops[0].output_transfer_fee =
            Some(TokenTransferFee { basis_points: 100, maximum_fee: 100 });
        assert_eq!(plan_route(&route, sol, quote, 10, 100, true).unwrap().minimum_credit, 1000);
    }
    #[tokio::test]
    async fn rejects_endpoint_token_program_mismatch() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = pk(31);
        let route = StonkFunQuoteRoute::buy(vec![StonkFunQuoteHop::exact_in(
            whirlpool(sol, quote),
            sol,
            quote,
            10,
        )]);
        let mut params = super::super::stonkfun::tests::swap_params(
            TradeType::Buy,
            sol,
            quote,
            DexParamEnum::StonkFunQuoteRoute(route.clone()),
        );
        params.output_token_program = Some(crate::constants::TOKEN_PROGRAM_2022);
        assert!(build_quote_route(&params, &route, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("output token program"));
    }
    #[tokio::test]
    async fn usdc_buy_selects_buy_hops_and_independent_sell_uses_sell_hops() {
        let usdc = crate::constants::USDC_TOKEN_ACCOUNT;
        let bridge = pk(30);
        let quote = pk(31);
        let meme = pk(32);
        let route = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(clmm(usdc, bridge), usdc, bridge, 1000),
            StonkFunQuoteHop::exact_in(dlmm(bridge, quote), bridge, quote, 2000),
        ])
        .with_sell_hops(vec![StonkFunQuoteHop::exact_in(
            whirlpool(quote, usdc),
            quote,
            usdc,
            100,
        )]);
        let via =
            StonkFunViaSolParams::curve(super::super::stonkfun::tests::curve_params(quote), route);
        let mut buy = super::super::stonkfun::tests::swap_params(
            TradeType::Buy,
            usdc,
            meme,
            via.clone().into(),
        );
        buy.close_input_mint_ata = false;
        let ixs = super::super::stonkfun::StonkFunInstructionBuilder
            .build_buy_instructions(&buy)
            .await
            .unwrap();
        let dlmm_ix = ixs
            .iter()
            .find(|ix| ix.program_id == super::super::utils::meteora_dlmm::PROGRAM_ID)
            .unwrap();
        assert_eq!(u64::from_le_bytes(dlmm_ix.data[8..16].try_into().unwrap()), 1000);
        let mut sell =
            super::super::stonkfun::tests::swap_params(TradeType::Sell, meme, usdc, via.into());
        sell.close_input_mint_ata = false;
        sell.close_output_mint_ata = false;
        let ixs = super::super::stonkfun::StonkFunInstructionBuilder
            .build_sell_instructions(&sell)
            .await
            .unwrap();
        assert!(ixs.iter().any(|ix| ix.program_id == super::super::utils::whirlpool::PROGRAM_ID));
        assert!(!ixs
            .iter()
            .any(|ix| ix.program_id == super::super::utils::meteora_dlmm::PROGRAM_ID));
    }
    #[tokio::test]
    async fn curve_buy_via_clmm_and_dlmm_uses_protected_bridge_amount() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let usdc = pk(30);
        let quote = pk(31);
        let meme = pk(32);
        let route = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(clmm(sol, usdc), sol, usdc, 1000),
            StonkFunQuoteHop::exact_in(dlmm(usdc, quote), usdc, quote, 2000),
        ]);
        let via =
            StonkFunViaSolParams::curve(super::super::stonkfun::tests::curve_params(quote), route);
        let mut params = super::super::stonkfun::tests::swap_params(
            TradeType::Buy,
            sol,
            meme,
            DexParamEnum::StonkFunViaSol(via),
        );
        params.create_input_mint_ata = false;
        params.create_output_mint_ata = false;
        params.close_input_mint_ata = false;
        params.close_output_mint_ata = false;
        params.close_input_mint_ata = false;
        let ixs = super::super::stonkfun::StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .unwrap();
        assert_eq!(ixs.len(), 3);
        assert_eq!(ixs[0].program_id, super::super::utils::raydium_clmm::PROGRAM_ID);
        assert_eq!(ixs[1].program_id, super::super::utils::meteora_dlmm::PROGRAM_ID);
        assert_eq!(u64::from_le_bytes(ixs[1].data[8..16].try_into().unwrap()), 1000);
        assert_eq!(u64::from_le_bytes(ixs[2].data[8..16].try_into().unwrap()), 2000);
    }
    #[tokio::test]
    async fn sell_uses_independent_reverse_path_and_rejects_unprotected_final_limit() {
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = pk(31);
        let meme = pk(32);
        let route = StonkFunQuoteRoute::sell(vec![StonkFunQuoteHop::exact_in(
            whirlpool(quote, sol),
            quote,
            sol,
            100,
        )]);
        let via =
            StonkFunViaSolParams::curve(super::super::stonkfun::tests::curve_params(quote), route);
        let mut params = super::super::stonkfun::tests::swap_params(
            TradeType::Sell,
            meme,
            sol,
            DexParamEnum::StonkFunViaSol(via),
        );
        params.create_input_mint_ata = false;
        params.create_output_mint_ata = false;
        params.close_output_mint_ata = false;
        params.close_input_mint_ata = false;
        let ixs = super::super::stonkfun::StonkFunInstructionBuilder
            .build_sell_instructions(&params)
            .await
            .unwrap();
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[1].program_id, super::super::utils::whirlpool::PROGRAM_ID);
        params.fixed_output_amount = Some(101);
        assert!(super::super::stonkfun::StonkFunInstructionBuilder
            .build_sell_instructions(&params)
            .await
            .unwrap_err()
            .to_string()
            .contains("final output"));
    }
    #[tokio::test]
    async fn quoted_whirlpool_route_mainnet_roundtrip_simulation() {
        use crate::common::mainnet_sim::{self, fixtures};
        if !mainnet_sim::enabled() {
            return;
        }
        let rpc = mainnet_sim::rpc_client();
        let wallet = mainnet_sim::create_wallet();
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let usdc = fixtures::USDC_MINT;
        let buy_pool = crate::trading::core::params::WhirlpoolParams::from_pool_address_by_rpc(
            &rpc,
            &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
            &sol,
            &usdc,
        )
        .await
        .unwrap();
        let sell_pool = crate::trading::core::params::WhirlpoolParams::from_pool_address_by_rpc(
            &rpc,
            &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
            &usdc,
            &sol,
        )
        .await
        .unwrap();
        // Deliberately conservative thresholds validate instruction execution,
        // not the quality of an external price quote.
        let route = StonkFunQuoteRoute::buy(vec![StonkFunQuoteHop::exact_in(
            StonkFunQuoteVenue::OrcaWhirlpool(buy_pool),
            sol,
            usdc,
            20_000,
        )])
        .with_sell_hops(vec![StonkFunQuoteHop::exact_in(
            StonkFunQuoteVenue::OrcaWhirlpool(sell_pool),
            usdc,
            sol,
            100_000,
        )]);
        let mut buy = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            crate::constants::SOL_TOKEN_ACCOUNT,
            usdc,
            1_000_000,
            100,
            DexParamEnum::StonkFunQuoteRoute(route.clone()),
        );
        buy.close_input_mint_ata = false;
        let mut sell = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Sell,
            usdc,
            sol,
            20_000,
            100,
            DexParamEnum::StonkFunQuoteRoute(route),
        );
        sell.create_input_mint_ata = false;
        sell.create_output_mint_ata = false;
        sell.close_input_mint_ata = false;
        sell.close_output_mint_ata = true;
        let mut instructions = super::super::stonkfun::StonkFunInstructionBuilder
            .build_buy_instructions(&buy)
            .await
            .unwrap();
        instructions.extend(
            super::super::stonkfun::StonkFunInstructionBuilder
                .build_sell_instructions(&sell)
                .await
                .unwrap(),
        );
        mainnet_sim::run_business_sim(
            &rpc,
            &wallet,
            instructions,
            &[],
            "StonkFun quoted Whirlpool conversion roundtrip",
        )
        .await;
    }
    #[tokio::test]
    async fn stonkfun_usdc_clmm_mainnet_simulation() {
        use crate::common::mainnet_sim;
        if !mainnet_sim::enabled() {
            return;
        }
        let rpc = mainnet_sim::rpc_client();
        let wallet = mainnet_sim::create_wallet();
        let usdc = crate::constants::USDC_TOKEN_ACCOUNT;
        let quote = solana_sdk::pubkey!("Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh");
        let meme = solana_sdk::pubkey!("DQsYFPcRKaKZWTjY4TiqvmxjN87vjumWMECJ7s4U1HbN");
        let curve = crate::trading::core::params::StonkFunParams::from_pool_by_rpc(
            &rpc,
            &solana_sdk::pubkey!("BmQj9pBopxouHecN5rYvVLqN7a48CVndfTkhESEZzWgN"),
        )
        .await
        .unwrap();
        let pool = RaydiumClmmParams::from_pool_address_by_rpc(
            &rpc,
            &solana_sdk::pubkey!("49iMatQtoyabsYAQc8GafVq6aeBFVDxSRH44oiatyyw6"),
            &usdc,
            &quote,
        )
        .await
        .unwrap();
        // Keep the direction-specific arrays loaded from current pool state:
        // even a small trade can cross an array boundary as liquidity moves.
        // Simulation-only funding of an ephemeral wallet; the user trade starts
        // with USDC and consists only of CLMM conversion + the curve buy.
        let (mut funding, usdc_amount) =
            mainnet_sim::build_sol_to_usdc_hop(&rpc, wallet.clone(), 10_000_000).await.unwrap();
        let hop =
            StonkFunQuoteHop::exact_in(StonkFunQuoteVenue::RaydiumClmm(pool), usdc, quote, 1000)
                .with_output_transfer_fee_by_rpc(&rpc)
                .await
                .unwrap();
        let params = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            usdc,
            meme,
            usdc_amount,
            100,
            StonkFunViaSolParams::curve(curve, StonkFunQuoteRoute::buy(vec![hop])).into(),
        );
        funding.extend(
            super::super::stonkfun::StonkFunInstructionBuilder
                .build_buy_instructions(&params)
                .await
                .unwrap(),
        );
        let mut tables = Vec::new();
        for address in [
            solana_sdk::pubkey!("58tUNfyBtwkMeU8gpyZYKEo4Hyb4E25ui12w5vKYQx3P"),
            solana_sdk::pubkey!("214Swu4BZtAUbnLfSWWwbrtpiestCM52nLcFAfJko4Uj"),
            solana_sdk::pubkey!("6nBVEfyJEs33X1tBeqo3TDjhd2CacHH3Q6y1JARpGd5G"),
        ] {
            tables.push(
                crate::common::address_lookup::fetch_address_lookup_table_account(&rpc, &address)
                    .await
                    .unwrap(),
            );
        }
        mainnet_sim::run_business_sim_with_compute_limit(
            &rpc,
            &wallet,
            funding,
            &tables,
            "StonkFun USDC CLMM inner buy",
            None,
        )
        .await;
    }
    #[tokio::test]
    async fn stonkfun_curve_whirlpool_mainnet_simulation() {
        use crate::common::mainnet_sim;
        if !mainnet_sim::enabled() {
            return;
        }
        let rpc = mainnet_sim::rpc_client();
        let wallet = mainnet_sim::create_wallet();
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = solana_sdk::pubkey!("pSo1f9nQXWgXibFtKf7NWYxb5enAM4qfP6UJSiXRQfL");
        let meme = solana_sdk::pubkey!("KUCxmmJJYmnzTbKWiXhauwxoa81Z38tC7FJtqti6Bhz");
        let curve = crate::trading::core::params::StonkFunParams::from_pool_by_rpc(
            &rpc,
            &solana_sdk::pubkey!("HcB1nwuqh1csZRJaHPJbpByJ8v5CQcbfi2uVbZcXP5Kx"),
        )
        .await
        .unwrap();
        let pool = WhirlpoolParams::from_pool_address_by_rpc(
            &rpc,
            &solana_sdk::pubkey!("3XLkRVg69AgwKAbnSjJpm3PB4QgVeXFEjiXfw5shWMBT"),
            &sol,
            &quote,
        )
        .await
        .unwrap();
        let hop = StonkFunQuoteHop::exact_in(
            StonkFunQuoteVenue::OrcaWhirlpool(pool),
            sol,
            quote,
            100_000,
        )
        .with_output_transfer_fee_by_rpc(&rpc)
        .await
        .unwrap();
        let via = StonkFunViaSolParams::curve(curve, StonkFunQuoteRoute::buy(vec![hop]));
        let lut = crate::common::address_lookup::fetch_address_lookup_table_account(
            &rpc,
            &solana_sdk::pubkey!("FAMYkkNCEpGEYAnq3SFwFvi5iTXjdNNqembfxX4EYimD"),
        )
        .await
        .unwrap();
        for asset in [crate::constants::SOL_TOKEN_ACCOUNT, crate::constants::WSOL_TOKEN_ACCOUNT] {
            let params = mainnet_sim::swap_params(
                wallet.clone(),
                TradeType::Buy,
                asset,
                meme,
                1_000_000,
                100,
                DexParamEnum::StonkFunViaSol(via.clone()),
            );
            let mut business = Vec::new();
            if asset == crate::constants::WSOL_TOKEN_ACCOUNT {
                // Fixture funding only. The built WSOL user buy must not fund again.
                business.extend(crate::trading::common::handle_wsol(&wallet.pubkey(), 1_000_000));
            }
            business.extend(
                super::super::stonkfun::StonkFunInstructionBuilder
                    .build_buy_instructions(&params)
                    .await
                    .unwrap(),
            );
            mainnet_sim::run_business_sim(
                &rpc,
                &wallet,
                business,
                std::slice::from_ref(&lut),
                if asset == crate::constants::SOL_TOKEN_ACCOUNT {
                    "StonkFun SOL inner buy"
                } else {
                    "StonkFun existing WSOL inner buy"
                },
            )
            .await;
        }
    }

    #[tokio::test]
    async fn stonkfun_graduated_dlmm_mainnet_simulation() {
        use crate::common::{
            address_lookup::fetch_address_lookup_table_account,
            mainnet_sim::{self, fixtures},
        };
        if !mainnet_sim::enabled() {
            return;
        }
        let rpc = mainnet_sim::rpc_client();
        let wallet = mainnet_sim::create_wallet();
        let sol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let quote = fixtures::GRAD_QUOTE_STONK;
        let graduated = crate::trading::core::params::RaydiumCpmmParams::from_pool_address_by_rpc(
            &rpc,
            &fixtures::GRAD_POOL,
        )
        .await
        .unwrap();
        let pool = MeteoraDlmmParams::from_pool_address_by_rpc(
            &rpc,
            &solana_sdk::pubkey!("48M3tRdbVYmEbf5rCTFVAgqCCaZdChVmeg3VPBrmgT8m"),
            &sol,
            &quote,
        )
        .await
        .unwrap();
        let hop =
            StonkFunQuoteHop::exact_in(StonkFunQuoteVenue::MeteoraDlmm(pool), sol, quote, 100_000)
                .with_output_transfer_fee_by_rpc(&rpc)
                .await
                .unwrap();
        let via = StonkFunViaSolParams::graduated(graduated, StonkFunQuoteRoute::buy(vec![hop]));
        let params = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            crate::constants::SOL_TOKEN_ACCOUNT,
            fixtures::GRAD_MEME_KNOTS,
            1_000_000,
            100,
            DexParamEnum::StonkFunViaSol(via),
        );
        let business = super::super::stonkfun::StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await
            .unwrap();
        let lut = fetch_address_lookup_table_account(
            &rpc,
            &solana_sdk::pubkey!("9rVP9Ly5RC1nix3WDm5QgkoWJbxV7Kteth1KHtYk5hT9"),
        )
        .await
        .unwrap();
        mainnet_sim::run_business_sim(
            &rpc,
            &wallet,
            business,
            &[lut],
            "StonkFun graduated buy via quoted DLMM",
        )
        .await;
    }
}
