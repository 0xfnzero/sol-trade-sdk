use anyhow::{Context, Result};
use solana_sdk::signature::Keypair;

/// Load a Solana keypair from a base58 string or a 64-byte JSON array in an environment variable.
pub fn load_keypair_from_env(name: &str) -> Result<Keypair> {
    let value = std::env::var(name).with_context(|| format!("{} is required", name))?;
    load_keypair_from_string(&value).with_context(|| format!("invalid {}", name))
}

/// Parse a Solana keypair without the panic behavior of `Keypair::from_base58_string`.
pub fn load_keypair_from_string(value: &str) -> Result<Keypair> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("keypair is empty");
    }

    let bytes = if value.starts_with('[') {
        serde_json::from_str::<Vec<u8>>(value).context("keypair JSON must be a byte array")?
    } else {
        bs58::decode(value)
            .into_vec()
            .context("keypair must be valid base58 or a 64-byte JSON array")?
    };
    if bytes.len() != 64 {
        anyhow::bail!("keypair must contain 64 bytes, got {}", bytes.len());
    }
    Keypair::try_from(bytes.as_slice()).context("invalid 64-byte Solana keypair")
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Signer;

    #[test]
    fn parses_base58_and_json_keypairs() {
        let keypair = Keypair::new();
        let base58 = bs58::encode(keypair.to_bytes()).into_string();
        let json = serde_json::to_string(&keypair.to_bytes().to_vec()).unwrap();

        assert_eq!(load_keypair_from_string(&base58).unwrap().pubkey(), keypair.pubkey());
        assert_eq!(load_keypair_from_string(&json).unwrap().pubkey(), keypair.pubkey());
    }

    #[test]
    fn rejects_placeholders_and_wrong_lengths() {
        assert!(load_keypair_from_string("use_your_payer_keypair_here").is_err());
        assert!(load_keypair_from_string("[1,2,3]").is_err());
    }
}
