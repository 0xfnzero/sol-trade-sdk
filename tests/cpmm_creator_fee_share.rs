use sol_parser_sdk::{core::events::DexEvent, instr::raydium_cpmm::parse_instruction};
use sol_trade_sdk::instruction::{
    raydium_cpmm::{collect_creator_fee, collect_creator_fee_permissionless},
    utils::{
        raydium_cpmm::{
            accounts, estimate_creator_fee_payout, get_creator_fee_share_pda, split_creator_fee,
        },
        raydium_cpmm_types::{
            amm_config_decode, creator_fee_share_decode, PoolState, AMM_CONFIG_DISCRIMINATOR,
            CREATOR_FEE_SHARE_DISCRIMINATOR,
        },
    },
};
use solana_sdk::{pubkey, pubkey::Pubkey, signature::Signature};

#[test]
fn collection_builders_match_idl_and_parser() {
    let pool_address = Pubkey::new_unique();
    let pool = PoolState {
        amm_config: Pubkey::new_unique(),
        pool_creator: Pubkey::new_unique(),
        token0_vault: Pubkey::new_unique(),
        token1_vault: Pubkey::new_unique(),
        token0_mint: Pubkey::new_unique(),
        token1_mint: Pubkey::new_unique(),
        token0_program: pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
        token1_program: pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"),
        ..Default::default()
    };
    let payer = Pubkey::new_unique();
    let idl: serde_json::Value =
        serde_json::from_str(include_str!("../idl/raydium_cpmm.json")).unwrap();
    for permissionless in [false, true] {
        let ix = if permissionless {
            collect_creator_fee_permissionless(&payer, &pool_address, &pool)
        } else {
            collect_creator_fee(&pool_address, &pool)
        };
        let name = if permissionless {
            "collect_creator_fee_permissionless"
        } else {
            "collect_creator_fee"
        };
        let spec =
            idl["instructions"].as_array().unwrap().iter().find(|x| x["name"] == name).unwrap();
        let spec_accounts = spec["accounts"].as_array().unwrap();
        assert_eq!(ix.program_id, accounts::RAYDIUM_CPMM);
        assert_eq!(
            ix.data,
            spec["discriminator"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        );
        assert_eq!(ix.accounts.len(), if permissionless { 16 } else { 15 });
        for (meta, spec) in ix.accounts.iter().zip(spec_accounts) {
            assert_eq!(meta.is_signer, spec["signer"].as_bool().unwrap_or(false));
            assert_eq!(meta.is_writable, spec["writable"].as_bool().unwrap_or(false));
        }
        let ata = |mint: Pubkey, program: Pubkey| {
            Pubkey::find_program_address(
                &[pool.pool_creator.as_ref(), program.as_ref(), mint.as_ref()],
                &pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
            )
            .0
        };
        let mut original_accounts = if permissionless {
            vec![payer, pool.pool_creator, accounts::AUTHORITY, pool_address]
        } else {
            vec![pool.pool_creator, accounts::AUTHORITY, pool_address, pool.amm_config]
        };
        original_accounts.extend([
            pool.token0_vault,
            pool.token1_vault,
            pool.token0_mint,
            pool.token1_mint,
            ata(pool.token0_mint, pool.token0_program),
            ata(pool.token1_mint, pool.token1_program),
            pool.token0_program,
            pool.token1_program,
            pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
            Pubkey::default(),
        ]);
        assert_eq!(
            ix.accounts[..14].iter().map(|a| a.pubkey).collect::<Vec<_>>(),
            original_accounts,
            "The original 14 accounts must retain their exact order"
        );
        if permissionless {
            assert_eq!(ix.accounts[14].pubkey, pool.amm_config);
        }
        let share = Pubkey::find_program_address(
            &[b"creator_fee_share", pool.pool_creator.as_ref(), pool.amm_config.as_ref()],
            &accounts::RAYDIUM_CPMM,
        )
        .0;
        assert_eq!(share, get_creator_fee_share_pda(&pool.pool_creator, &pool.amm_config));
        assert_eq!(ix.accounts.last().unwrap().pubkey, share);
        let keys: Vec<_> = ix.accounts.iter().map(|a| a.pubkey).collect();
        let DexEvent::RaydiumCpmmCollectCreatorFee(event) =
            parse_instruction(&ix.data, &keys, Signature::default(), 1, 0, None).unwrap()
        else {
            panic!("collection");
        };
        assert_eq!(event.permissionless, permissionless);
        assert_eq!(event.creator, pool.pool_creator);
        assert_eq!(event.payer, if permissionless { payer } else { pool.pool_creator });
        assert_eq!(event.pool_state, pool_address);
        assert_eq!(event.amm_config, pool.amm_config);
        assert_eq!(
            (event.token_0_vault, event.token_1_vault),
            (pool.token0_vault, pool.token1_vault)
        );
        assert_eq!(event.creator_fee_share, share);
        for (index, mint, program) in
            [(8, pool.token0_mint, pool.token0_program), (9, pool.token1_mint, pool.token1_program)]
        {
            let ata = Pubkey::find_program_address(
                &[pool.pool_creator.as_ref(), program.as_ref(), mint.as_ref()],
                &pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
            )
            .0;
            assert_eq!(keys[index], ata);
        }
        for len in 0..keys.len() {
            assert!(parse_instruction(&ix.data, &keys[..len], Signature::default(), 1, 0, None)
                .is_none());
        }
    }
}

#[test]
fn config_and_custom_share_layouts_match_chain_sizes() {
    let mut config = vec![0u8; 236];
    config[..8].copy_from_slice(&AMM_CONFIG_DISCRIMINATOR);
    config[108..116].copy_from_slice(&2500u64.to_le_bytes());
    config[116..124].copy_from_slice(&200_000u64.to_le_bytes());
    config[228..236].copy_from_slice(&42u64.to_le_bytes());
    let decoded = amm_config_decode(&config[8..]).unwrap();
    assert_eq!(decoded.creator_fee_rate, 2500);
    assert_eq!(decoded.creator_fee_share_rate, 200_000);
    assert_eq!(decoded.padding[13], 42);
    assert!(amm_config_decode(&config[8..235]).is_none());
    let mut share = vec![0; 145];
    share[..8].copy_from_slice(&CREATOR_FEE_SHARE_DISCRIMINATOR);
    share[8] = 255;
    share[73..81].copy_from_slice(&300_000u64.to_le_bytes());
    assert_eq!(creator_fee_share_decode(&share).unwrap().share_rate, 300_000);
    assert!(creator_fee_share_decode(&share[..144]).is_none());
    share[0] ^= 1;
    assert!(creator_fee_share_decode(&share).is_none());
}

#[test]
fn payout_rounding_and_large_values_match_program() {
    assert_eq!(split_creator_fee(1000, 200_000).unwrap(), (800, 200));
    assert_eq!(split_creator_fee(9, 200_000).unwrap(), (8, 1));
    assert_eq!(split_creator_fee(1, 200_000).unwrap(), (1, 0));
    assert!(split_creator_fee(1, 1_000_001).is_err());
    for fee in [0, 1, 9, 1000, u64::MAX] {
        for rate in [0, 1, 333_333, 999_999, 1_000_000] {
            let (creator, protocol) = split_creator_fee(fee, rate).unwrap();
            assert_eq!(creator + protocol, fee);
            assert_eq!(protocol, (u128::from(fee) * u128::from(rate) / 1_000_000) as u64);
        }
    }
    let pool =
        PoolState { creator_fees_token0: 1000, creator_fees_token1: 9, ..Default::default() };
    assert_eq!(estimate_creator_fee_payout(&pool, 200_000).unwrap(), (800, 8));
}

#[test]
fn share_rate_overrides_config_and_missing_account_falls_back() {
    use sol_trade_sdk::instruction::utils::{
        raydium_cpmm::resolve_creator_fee_share_rate, raydium_cpmm_types::AmmConfig,
    };
    use solana_sdk::account::Account;
    let creator = Pubkey::new_unique();
    let config_address = Pubkey::new_unique();
    let config = AmmConfig { creator_fee_share_rate: 200_000, ..Default::default() };
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, None).unwrap(),
        200_000
    );
    let mut account = Account { lamports: 1, owner: accounts::RAYDIUM_CPMM, ..Default::default() };
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, Some(&account)).unwrap(),
        200_000
    );
    account.data = vec![0; 145];
    account.data[..8].copy_from_slice(&CREATOR_FEE_SHARE_DISCRIMINATOR);
    account.data[9..41].copy_from_slice(creator.as_ref());
    account.data[41..73].copy_from_slice(config_address.as_ref());
    account.data[73..81].copy_from_slice(&300_000u64.to_le_bytes());
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, Some(&account)).unwrap(),
        300_000
    );
    account.data[73..81].copy_from_slice(&0u64.to_le_bytes());
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, Some(&account)).unwrap(),
        0
    );
    account.data[0] ^= 1;
    assert!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, Some(&account)).is_err()
    );
    account.owner = Pubkey::default();
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &config_address, Some(&account)).unwrap(),
        200_000
    );
}

/// Golden bytes were returned by mainnet simulateTransaction, not generated by the SDK.
#[test]
fn saved_mainnet_simulation_snapshots_match_parser_and_trade_decoders() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sol_parser_sdk::{
        accounts::{parse_account_unified, AccountData},
        core::events::EventMetadata,
    };
    use sol_trade_sdk::instruction::utils::raydium_cpmm_types::pool_state_decode;
    for (report, custom, nonzero_transfer_fee) in [
        (include_str!("fixtures/cpmm_creator_fee_default_mainnet.json"), false, false),
        (include_str!("fixtures/cpmm_creator_fee_override_mainnet.json"), true, false),
        (include_str!("fixtures/cpmm_creator_fee_token2022_mainnet.json"), false, true),
    ] {
        let report: serde_json::Value = serde_json::from_str(report).unwrap();
        assert_eq!(report["broadcast"], false);
        assert_eq!(report["signature_verification"], false);
        for result in report["results"].as_array().unwrap() {
            assert_eq!(result["validated"], true);
            assert_eq!(
                result["checks"].as_array().unwrap().iter().any(|c| c["transfer_fee"]
                    .as_str()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
                    > 0),
                nonzero_transfer_fee,
                "Fixture must exercise its declared transfer-fee scenario"
            );
            assert_eq!(result["share_account_exists"], custom);
            assert_eq!(result["share_rate"], if custom { 0 } else { 50000 });
            assert_eq!(result["config_share_rate"], 50000);
            assert_eq!(
                result["legacy_error"],
                serde_json::json!({"InstructionError":[0,{"Custom":3005}]})
            );
            assert_eq!(
                result["wrong_pda_error"],
                serde_json::json!({"InstructionError":[0,{"Custom":2006}]})
            );
            assert_eq!(
                result["duplicate_collection_error"],
                serde_json::json!({"InstructionError":[1,{"Custom":6014}]})
            );
            if result["permissionless"] == true {
                assert_eq!(
                    result["permissionless_missing_share_error"],
                    serde_json::json!({"InstructionError":[0,{"Custom":3005}]})
                );
            }
            for (phase, index) in
                [("before_accounts", 0), ("before_accounts", 1), ("simulated_accounts", 0)]
            {
                let raw = &result[phase][index];
                let data = STANDARD.decode(raw["data_base64"].as_str().unwrap()).unwrap();
                let account = AccountData {
                    pubkey: raw["pubkey"].as_str().unwrap().parse().unwrap(),
                    owner: raw["owner"].as_str().unwrap().parse().unwrap(),
                    data: data.clone(),
                    lamports: raw["lamports"].as_u64().unwrap(),
                    executable: false,
                    rent_epoch: 0,
                };
                let parsed =
                    parse_account_unified(&account, EventMetadata::default(), None).unwrap();
                if index == 1 {
                    let DexEvent::RaydiumCpmmAmmConfigAccount(e) = parsed else {
                        panic!("config");
                    };
                    let trade = amm_config_decode(&data[8..]).unwrap();
                    assert_eq!(
                        serde_json::to_value(e.amm_config).unwrap(),
                        serde_json::to_value(trade).unwrap()
                    );
                } else {
                    let DexEvent::RaydiumCpmmPoolStateAccount(e) = parsed else {
                        panic!("pool");
                    };
                    let trade = pool_state_decode(&data[8..]).unwrap();
                    let parsed = serde_json::to_value(e.pool_state).unwrap();
                    let mapped: serde_json::Map<_, _> = parsed
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.replace("token_0", "token0")
                                    .replace("token_1", "token1")
                                    .replace("mint_0", "mint0")
                                    .replace("mint_1", "mint1"),
                                v.clone(),
                            )
                        })
                        .collect();
                    assert_eq!(
                        serde_json::Value::Object(mapped),
                        serde_json::to_value(trade).unwrap()
                    );
                }
            }
            if custom {
                let raw = &result["before_accounts"][2];
                let data = STANDARD.decode(raw["data_base64"].as_str().unwrap()).unwrap();
                let account = AccountData {
                    pubkey: raw["pubkey"].as_str().unwrap().parse().unwrap(),
                    owner: raw["owner"].as_str().unwrap().parse().unwrap(),
                    data: data.clone(),
                    lamports: 1,
                    executable: false,
                    rent_epoch: 0,
                };
                let DexEvent::RaydiumCpmmCreatorFeeShareAccount(e) =
                    parse_account_unified(&account, EventMetadata::default(), None).unwrap()
                else {
                    panic!("share");
                };
                assert_eq!(
                    serde_json::to_value(e.creator_fee_share).unwrap(),
                    serde_json::to_value(creator_fee_share_decode(&data).unwrap()).unwrap()
                );
            }
            let bytes = |phase: &str, index: usize| {
                STANDARD.decode(result[phase][index]["data_base64"].as_str().unwrap()).unwrap()
            };
            let before = pool_state_decode(&bytes("before_accounts", 0)[8..]).unwrap();
            let after = pool_state_decode(&bytes("simulated_accounts", 0)[8..]).unwrap();
            assert_eq!((after.creator_fees_token0, after.creator_fees_token1), (0, 0));
            for (index, gross, protocol_before, protocol_after) in [
                (
                    0,
                    before.creator_fees_token0,
                    before.protocol_fees_token0,
                    after.protocol_fees_token0,
                ),
                (
                    1,
                    before.creator_fees_token1,
                    before.protocol_fees_token1,
                    after.protocol_fees_token1,
                ),
            ] {
                let (creator, protocol) =
                    split_creator_fee(gross, result["share_rate"].as_u64().unwrap()).unwrap();
                assert_eq!(protocol_after - protocol_before, protocol);
                let amount = |phase: &str, j: usize| {
                    u64::from_le_bytes(bytes(phase, j)[64..72].try_into().unwrap())
                };
                let fund =
                    if index == 0 { before.fund_fees_token0 } else { before.fund_fees_token1 };
                let reserve_before = u128::from(amount("before_accounts", 3 + index))
                    - u128::from(protocol_before)
                    - u128::from(fund)
                    - u128::from(gross);
                let reserve_after = u128::from(amount("simulated_accounts", 3 + index))
                    - u128::from(protocol_after)
                    - u128::from(fund);
                assert_eq!(
                    reserve_before, reserve_after,
                    "Collection must preserve LP trading reserves"
                );
                assert_eq!(
                    amount("before_accounts", 3 + index) - amount("simulated_accounts", 3 + index),
                    creator
                );
                let check = &result["checks"][index];
                assert_eq!(check["creator_before_transfer_fee"], creator.to_string());
                let fee = check["transfer_fee"].as_str().unwrap().parse::<u64>().unwrap();
                use spl_token_2022_interface::{
                    extension::{
                        transfer_fee::{TransferFeeAmount, TransferFeeConfig},
                        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
                    },
                    state::{Account as TokenAccount, Mint},
                };
                let program =
                    if index == 0 { before.token0_program } else { before.token1_program };
                let expected_fee = if program.to_bytes() == spl_token_2022_interface::ID.to_bytes()
                {
                    let mint_data = bytes("before_accounts", 7 + index);
                    let mint = StateWithExtensions::<Mint>::unpack(&mint_data).unwrap();
                    if mint
                        .get_extension_types()
                        .unwrap()
                        .contains(&ExtensionType::TransferFeeConfig)
                    {
                        mint.get_extension::<TransferFeeConfig>()
                            .unwrap()
                            .calculate_epoch_fee(after.recent_epoch, creator)
                            .unwrap()
                    } else {
                        0
                    }
                } else {
                    0
                };
                assert_eq!(fee, expected_fee, "Reported fee must match the captured mint schedule");
                let withheld = |phase: &str, j: usize| {
                    if program.to_bytes() != spl_token_2022_interface::ID.to_bytes()
                        || result[phase][j].is_null()
                    {
                        return 0u64;
                    }
                    let data = bytes(phase, j);
                    let account = StateWithExtensions::<TokenAccount>::unpack(&data).unwrap();
                    if account
                        .get_extension_types()
                        .unwrap()
                        .contains(&ExtensionType::TransferFeeAmount)
                    {
                        u64::from(
                            account.get_extension::<TransferFeeAmount>().unwrap().withheld_amount,
                        )
                    } else {
                        0
                    }
                };
                let withheld_before = withheld("before_accounts", 5 + index);
                let withheld_after = withheld("simulated_accounts", 5 + index);
                assert_eq!(withheld_after - withheld_before, fee);
                assert_eq!(
                    withheld("before_accounts", 3 + index),
                    withheld("simulated_accounts", 3 + index)
                );
                if !check["withheld_after"].is_null() {
                    assert_eq!(check["withheld_before"], withheld_before.to_string());
                    assert_eq!(check["withheld_after"], withheld_after.to_string());
                }

                let recipient_before = if result["before_accounts"][5 + index].is_null() {
                    0
                } else {
                    amount("before_accounts", 5 + index)
                };
                assert_eq!(
                    amount("simulated_accounts", 5 + index) - recipient_before,
                    creator - fee
                );
            }
        }
    }
}

fn fee_snapshot_rpc(values: Vec<serde_json::Value>) -> sol_trade_sdk::common::SolanaRpcClient {
    use solana_rpc_client_api::request::RpcRequest;
    sol_trade_sdk::common::SolanaRpcClient::new_mock_with_mocks(
        "succeeds".to_owned(),
        std::collections::HashMap::from([(
            RpcRequest::GetMultipleAccounts,
            serde_json::json!({"context":{"slot":123},"value":values}),
        )]),
    )
}

fn rpc_account(owner: Pubkey, data: &[u8]) -> serde_json::Value {
    use base64::{engine::general_purpose::STANDARD, Engine};
    serde_json::json!({"lamports":1,"owner":owner.to_string(),"executable":false,
        "rentEpoch":0,"data":[STANDARD.encode(data),"base64"]})
}

fn fee_config_bytes(rate: u64) -> Vec<u8> {
    let mut data = vec![0; 236];
    data[..8].copy_from_slice(&AMM_CONFIG_DISCRIMINATOR);
    data[116..124].copy_from_slice(&rate.to_le_bytes());
    data
}

fn fee_share_bytes(creator: Pubkey, config: Pubkey, rate: u64) -> Vec<u8> {
    let mut data = vec![0; 145];
    data[..8].copy_from_slice(&CREATOR_FEE_SHARE_DISCRIMINATOR);
    data[9..41].copy_from_slice(creator.as_ref());
    data[41..73].copy_from_slice(config.as_ref());
    data[73..81].copy_from_slice(&rate.to_le_bytes());
    data
}

#[tokio::test]
async fn fee_rate_rpc_uses_one_snapshot_for_config_and_override() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::fetch_creator_fee_share_rate;
    let creator = Pubkey::new_unique();
    let config = Pubkey::new_unique();
    for (share, expected) in [
        (serde_json::Value::Null, 200_000),
        (rpc_account(accounts::RAYDIUM_CPMM, &[]), 200_000),
        (rpc_account(Pubkey::default(), &[42]), 200_000),
        (rpc_account(accounts::RAYDIUM_CPMM, &fee_share_bytes(creator, config, 0)), 0),
        (
            rpc_account(accounts::RAYDIUM_CPMM, &fee_share_bytes(creator, config, 1_000_000)),
            1_000_000,
        ),
    ] {
        // Only GetMultipleAccounts is mocked: separate GetAccount reads cannot pass.
        let rpc = fee_snapshot_rpc(vec![
            rpc_account(accounts::RAYDIUM_CPMM, &fee_config_bytes(200_000)),
            share,
        ]);
        assert_eq!(fetch_creator_fee_share_rate(&rpc, &creator, &config).await.unwrap(), expected);
    }
}

#[tokio::test]
async fn fee_rate_rpc_rejects_missing_or_invalid_config_and_incomplete_snapshot() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::fetch_creator_fee_share_rate;
    let creator = Pubkey::new_unique();
    let config = Pubkey::new_unique();
    let mut bad_discriminator = fee_config_bytes(200_000);
    bad_discriminator[0] ^= 1;
    for (values, expected) in [
        (vec![], "Incomplete"),
        (vec![serde_json::Value::Null], "Incomplete"),
        (vec![serde_json::Value::Null, serde_json::Value::Null], "missing"),
        (
            vec![
                rpc_account(Pubkey::default(), &fee_config_bytes(200_000)),
                serde_json::Value::Null,
            ],
            "not owned",
        ),
        (
            vec![rpc_account(accounts::RAYDIUM_CPMM, &[0; 235]), serde_json::Value::Null],
            "too short",
        ),
        (
            vec![rpc_account(accounts::RAYDIUM_CPMM, &bad_discriminator), serde_json::Value::Null],
            "discriminator",
        ),
        (
            vec![
                rpc_account(accounts::RAYDIUM_CPMM, &fee_config_bytes(1_000_001)),
                serde_json::Value::Null,
            ],
            "exceeds",
        ),
    ] {
        let error = fetch_creator_fee_share_rate(&fee_snapshot_rpc(values), &creator, &config)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    let error = fetch_creator_fee_share_rate(
        &sol_trade_sdk::common::SolanaRpcClient::new_mock("fails".to_owned()),
        &creator,
        &config,
    )
    .await;
    assert!(error.is_err(), "RPC failures must not become config fallback");
}

#[tokio::test]
async fn fee_rate_rpc_rejects_corrupt_or_mismatched_override() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::fetch_creator_fee_share_rate;
    let creator = Pubkey::new_unique();
    let config = Pubkey::new_unique();
    let mut wrong_discriminator = fee_share_bytes(creator, config, 0);
    wrong_discriminator[0] ^= 1;
    for (share, expected) in [
        (vec![0; 144], "Invalid"),
        (wrong_discriminator, "Invalid"),
        (fee_share_bytes(Pubkey::new_unique(), config, 0), "mismatch"),
        (fee_share_bytes(creator, Pubkey::new_unique(), 0), "mismatch"),
        (fee_share_bytes(creator, config, 1_000_001), "exceeds"),
    ] {
        let rpc = fee_snapshot_rpc(vec![
            rpc_account(accounts::RAYDIUM_CPMM, &fee_config_bytes(200_000)),
            rpc_account(accounts::RAYDIUM_CPMM, &share),
        ]);
        let error = fetch_creator_fee_share_rate(&rpc, &creator, &config).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn closed_share_observations_ignore_retained_bytes_and_use_config_rate() {
    use sol_trade_sdk::instruction::utils::{
        raydium_cpmm::resolve_creator_fee_share_rate, raydium_cpmm_types::AmmConfig,
    };
    use solana_sdk::account::Account;
    let creator = Pubkey::new_unique();
    let address = Pubkey::new_unique();
    let mut config = AmmConfig { creator_fee_share_rate: 200_000, ..Default::default() };
    let mut share = Account {
        owner: accounts::RAYDIUM_CPMM,
        lamports: 1,
        data: vec![0; 145],
        ..Default::default()
    };
    share.data[..8].copy_from_slice(&CREATOR_FEE_SHARE_DISCRIMINATOR);
    share.data[9..41].copy_from_slice(creator.as_ref());
    share.data[41..73].copy_from_slice(address.as_ref());
    share.data[73..81].copy_from_slice(&300_000u64.to_le_bytes());
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &address, Some(&share)).unwrap(),
        300_000
    );
    share.lamports = 0;
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &address, Some(&share)).unwrap(),
        200_000
    );
    // Closed bytes need not be decodable. Reactivating malformed bytes is an error.
    share.data[0] ^= 1;
    assert_eq!(
        resolve_creator_fee_share_rate(&config, &creator, &address, Some(&share)).unwrap(),
        200_000
    );
    share.lamports = 1;
    assert!(resolve_creator_fee_share_rate(&config, &creator, &address, Some(&share)).is_err());
    share.lamports = 0;
    config.creator_fee_share_rate = 1_000_001;
    assert!(resolve_creator_fee_share_rate(&config, &creator, &address, Some(&share)).is_err());
}

#[test]
fn parser_and_trade_reject_the_same_malformed_cpmm_boolean_bytes() {
    use sol_parser_sdk::{
        accounts::{raydium_cpmm as parser, AccountData},
        core::events::EventMetadata,
    };
    use sol_trade_sdk::instruction::utils::raydium_cpmm_types::{
        amm_config_decode, pool_state_decode,
    };
    for value in 0..=u8::MAX {
        for pool in [false, true] {
            let mut account = AccountData {
                pubkey: Pubkey::new_unique(),
                owner: accounts::RAYDIUM_CPMM,
                data: vec![0; if pool { 637 } else { 236 }],
                executable: false,
                lamports: 1,
                rent_epoch: 0,
            };
            let discriminator = if pool {
                parser::discriminators::POOL_STATE
            } else {
                parser::discriminators::AMM_CONFIG
            };
            account.data[..8].copy_from_slice(discriminator);
            account.data[if pool { 390 } else { 9 }] = value;
            let parsed = parser::parse_account(&account, EventMetadata::default()).is_some();
            let decoded = if pool {
                pool_state_decode(&account.data[8..]).is_some()
            } else {
                amm_config_decode(&account.data[8..]).is_some()
            };
            assert_eq!(parsed, decoded, "pool={pool}, bool={value}");
            assert_eq!(decoded, value <= 1);
        }
    }
}
