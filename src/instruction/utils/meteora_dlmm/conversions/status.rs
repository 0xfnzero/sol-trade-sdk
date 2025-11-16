use crate::*;
use crate::instruction::utils::meteora_dlmm::extensions::PairStatus;

impl TryFrom<u8> for PairStatus {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PairStatus::Enabled),
            1 => Ok(PairStatus::Disabled),
            _ => Err(anyhow::anyhow!("Invalid PairStatus value: {}", value)),
        }
    }
}

impl PartialEq for PairStatus {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (PairStatus::Enabled, PairStatus::Enabled)
                | (PairStatus::Disabled, PairStatus::Disabled)
        )
    }
}
