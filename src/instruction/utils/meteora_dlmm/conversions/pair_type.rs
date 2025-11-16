use crate::*;
use crate::instruction::utils::meteora_dlmm::extensions::PairType;

impl TryFrom<u8> for PairType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PairType::Permissionless),
            1 => Ok(PairType::Permission),
            2 => Ok(PairType::CustomizablePermissionless),
            3 => Ok(PairType::PermissionlessV2),
            _ => Err(anyhow::anyhow!("Invalid PairType value: {}", value)),
        }
    }
}

impl PartialEq for PairType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (&PairType::Permissionless, &PairType::Permissionless) => true,
            (&PairType::Permission, &PairType::Permission) => true,
            (&PairType::CustomizablePermissionless, &PairType::CustomizablePermissionless) => true,
            (&PairType::PermissionlessV2, &PairType::PermissionlessV2) => true,
            _ => false,
        }
    }
}
