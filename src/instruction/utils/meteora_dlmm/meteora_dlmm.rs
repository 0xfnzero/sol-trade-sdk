use solana_program::pubkey::Pubkey;
use solana_streamer::streaming::event_parser::protocols::meteora_dlmm::parser::METEORA_DLMM_PROGRAM_ID;
use crate::common::AnyResult;
use crate::common::fast_fn::{get_cached_pda, PdaCacheKey};
use crate::instruction::utils::meteora_dlmm::seeds::BIN_ARRAY_BITMAP_SEED;

pub mod seeds {
    pub const BIN_ARRAY: &[u8] = b"bin_array";

    pub const ORACLE: &[u8] = b"oracle";
}

pub mod accounts {
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    pub const EVENT_AUTHORITY: Pubkey = pubkey!("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6");

    pub const EVENT_AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: EVENT_AUTHORITY,
            is_signer: false,
            is_writable: false,
        };
}

pub const SWAP_DISCRIMINATOR: &[u8] = &[248, 198, 158, 145, 225, 117, 135, 20];

pub fn get_bin_array_bitmap_extension_pda(lb_pair: &Pubkey) -> Option<Pubkey> {
    get_cached_pda(
        PdaCacheKey::MeteoraDlmmBinArrayBitmapExtension(*lb_pair),
        || {
            let seeds: &[&[u8]; 2] = &[BIN_ARRAY_BITMAP_SEED, lb_pair.as_ref()];
            let program_id = &METEORA_DLMM_PROGRAM_ID;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}
