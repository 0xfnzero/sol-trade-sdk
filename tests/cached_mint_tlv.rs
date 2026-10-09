use base64::{engine::general_purpose::STANDARD, Engine};
use spl_token_2022_interface::{
    extension::{
        default_account_state::DefaultAccountState, pausable::PausableConfig,
        transfer_fee::TransferFeeConfig, transfer_hook::get_program_id, BaseStateWithExtensions,
        ExtensionType, StateWithExtensions,
    },
    state::Mint,
};

#[test]
fn official_cached_fee_and_eligibility_oracle() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cached_mint_tlv_20261009.json")).unwrap();
    for c in fixture["cases"].as_array().unwrap() {
        let data = STANDARD.decode(c["mint_data"].as_str().unwrap()).unwrap();
        let result = (|| -> Result<(u16, u64), Box<dyn std::error::Error>> {
            let mint = StateWithExtensions::<Mint>::unpack(&data)?;
            let mut fee = (0, 0);
            for kind in mint.get_extension_types()? {
                match kind {
                    ExtensionType::TransferFeeConfig => {
                        let config = mint.get_extension::<TransferFeeConfig>()?;
                        let schedule = config.get_epoch_fee(c["epoch"].as_u64().unwrap());
                        fee = (
                            u16::from(schedule.transfer_fee_basis_points),
                            u64::from(schedule.maximum_fee),
                        );
                    }
                    ExtensionType::TransferHook => {
                        if get_program_id(&mint).is_some() {
                            return Err("active Hook".into());
                        }
                    }
                    ExtensionType::Pausable => {
                        if bool::from(mint.get_extension::<PausableConfig>()?.paused) {
                            return Err("paused".into());
                        }
                    }
                    ExtensionType::DefaultAccountState => {
                        if mint.get_extension::<DefaultAccountState>()?.state != 1 {
                            return Err("frozen".into());
                        }
                    }
                    _ => return Err("unsupported".into()),
                }
            }
            Ok(fee)
        })();
        assert_eq!(result.is_ok(), c["eligible"].as_bool().unwrap(), "{} {result:?}", c["name"]);
        if let Ok(fee) = result {
            assert_eq!(
                fee,
                (c["basis_points"].as_u64().unwrap() as u16, c["maximum_fee"].as_u64().unwrap()),
                "{}",
                c["name"]
            );
        }
    }
}
