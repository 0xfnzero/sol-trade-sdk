use solana_sdk::pubkey::Pubkey;
use solana_streamer::streaming::event_parser::protocols::raydium_clmm::parser::RAYDIUM_CLMM_PROGRAM_ID;
use solana_streamer::streaming::event_parser::protocols::raydium_clmm::states::pool::{PoolState, POOL_TICK_ARRAY_BITMAP_SEED};
use solana_streamer::streaming::event_parser::protocols::raydium_clmm::states::tick_array::TICK_ARRAY_SEED;
use solana_streamer::streaming::event_parser::protocols::raydium_clmm::states::tickarray_bitmap_extension::TickArrayBitmapExtension;
use crate::common::fast_fn::{get_cached_pda, PdaCacheKey};

pub const SWAP_DISCRIMINATOR: &[u8] = &[43, 4, 237, 11, 26, 201, 30, 98];

pub fn get_tick_array_bitmap_extension_pda(pool: &Pubkey) -> Option<Pubkey> {
    get_cached_pda(
        PdaCacheKey::RaydiumClmmTickArrayBitmapExtension(*pool),
        || {
            let seeds: &[&[u8]; 2] = &[POOL_TICK_ARRAY_BITMAP_SEED.as_bytes(), pool.as_ref()];
            let program_id = &RAYDIUM_CLMM_PROGRAM_ID;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

pub fn get_tick_array_pda(pool: &Pubkey, index: i32) -> Option<Pubkey> {
    get_cached_pda(
        PdaCacheKey::RaydiumClmmTickArray(*pool, index),
        || {
            let seeds: &[&[u8]; 3] = &[TICK_ARRAY_SEED.as_bytes(), pool.as_ref(), &index.to_be_bytes()];
            let program_id = &RAYDIUM_CLMM_PROGRAM_ID;
            let pda: Option<(Pubkey, u8)> = Pubkey::try_find_program_address(seeds, program_id);
            pda.map(|pubkey| pubkey.0)
        },
    )
}

pub fn get_tick_arrays(
    pool_key: &Pubkey,
    pool_state: &PoolState,
    tickarray_bitmap_extension_key: &Pubkey,
    tickarray_bitmap_extension: &TickArrayBitmapExtension,
    zero_for_one: bool,
) -> Vec<Pubkey> {
    let mut tick_array_keys = Vec::new();

    // 需要先把tickarray_bitmap_extension_key加入
    tick_array_keys.push(*tickarray_bitmap_extension_key);

    let (_, mut current_valid_tick_array_start_index) = pool_state
        .get_first_initialized_tick_array(&mut Some(*tickarray_bitmap_extension), zero_for_one)
        .unwrap();
    tick_array_keys.push(get_tick_array_pda(pool_key, current_valid_tick_array_start_index).unwrap());
    let mut max_array_size = 3;
    while max_array_size != 0 {
        let next_tick_array_index = pool_state
            .next_initialized_tick_array_start_index(
                &Some(*tickarray_bitmap_extension),
                current_valid_tick_array_start_index,
                zero_for_one,
            )
            .unwrap();
        if next_tick_array_index.is_none() {
            break;
        }
        current_valid_tick_array_start_index = next_tick_array_index.unwrap();
        tick_array_keys.push(get_tick_array_pda(pool_key, current_valid_tick_array_start_index).unwrap());
        max_array_size -= 1;
    }
    tick_array_keys
}