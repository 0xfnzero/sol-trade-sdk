use anyhow::{anyhow, Context, Result};
use ruint::aliases::U512;
use crate::instruction::utils::meteora_dlmm::constants::{BIN_ARRAY_BITMAP_SIZE, EXTENSION_BINARRAY_BITMAP_SIZE};

pub trait BinArrayBitmapExtExtension {
    fn bitmap_range() -> (i32, i32);
    fn get_bitmap_offset(bin_array_index: i32) -> Result<usize>;
    fn bin_array_offset_in_bitmap(bin_array_index: i32) -> Result<usize>;
    fn to_bin_array_index(offset: usize, bin_array_offset: usize, is_positive: bool)
                          -> Result<i32>;

    fn get_bitmap(&self, bin_array_index: i32) -> Result<(usize, [u64; 8])>;
    fn bit(&self, bin_array_index: i32) -> Result<bool>;
    fn iter_bitmap(&self, start_index: i32, end_index: i32) -> Result<Option<i32>>;
    fn next_bin_array_index_with_liquidity(
        &self,
        swap_for_y: bool,
        start_index: i32,
    ) -> Result<(i32, bool)>;
}

impl BinArrayBitmapExtExtension for BinArrayBitmapExtension {
    fn bitmap_range() -> (i32, i32) {
        return (
            -BIN_ARRAY_BITMAP_SIZE * (EXTENSION_BINARRAY_BITMAP_SIZE as i32 + 1),
            BIN_ARRAY_BITMAP_SIZE * (EXTENSION_BINARRAY_BITMAP_SIZE as i32 + 1) - 1,
        );
    }

    fn next_bin_array_index_with_liquidity(
        &self,
        swap_for_y: bool,
        start_index: i32,
    ) -> Result<(i32, bool)> {
        let (min_bitmap_id, max_bit_map_id) = BinArrayBitmapExtension::bitmap_range();
        if start_index > 0 {
            if swap_for_y {
                match self.iter_bitmap(start_index, BIN_ARRAY_BITMAP_SIZE)? {
                    Some(value) => return Ok((value, true)),
                    None => return Ok((BIN_ARRAY_BITMAP_SIZE - 1, false)),
                }
            } else {
                match self.iter_bitmap(start_index, max_bit_map_id)? {
                    Some(value) => return Ok((value, true)),
                    None => return Err(anyhow!("Cannot find non-zero liquidity bin array id")),
                }
            }
        } else if swap_for_y {
            match self.iter_bitmap(start_index, min_bitmap_id)? {
                Some(value) => return Ok((value, true)),
                None => return Err(anyhow!("Cannot find non-zero liquidity bin array id")),
            }
        } else {
            match self.iter_bitmap(start_index, -BIN_ARRAY_BITMAP_SIZE - 1)? {
                Some(value) => return Ok((value, true)),
                None => return Ok((-BIN_ARRAY_BITMAP_SIZE, false)),
            }
        }
    }

    fn bit(&self, bin_array_index: i32) -> Result<bool> {
        let (_, bin_array_bitmap) = self.get_bitmap(bin_array_index)?;
        let bin_array_offset_in_bitmap = Self::bin_array_offset_in_bitmap(bin_array_index)?;
        let bin_array_bitmap = U512::from_limbs(bin_array_bitmap);
        Ok(bin_array_bitmap.bit(bin_array_offset_in_bitmap as usize))
    }

    fn get_bitmap(&self, bin_array_index: i32) -> Result<(usize, [u64; 8])> {
        let offset = Self::get_bitmap_offset(bin_array_index)?;
        if bin_array_index < 0 {
            Ok((offset, self.negative_bin_array_bitmap[offset]))
        } else {
            Ok((offset, self.positive_bin_array_bitmap[offset]))
        }
    }

    fn to_bin_array_index(
        offset: usize,
        bin_array_offset: usize,
        is_positive: bool,
    ) -> Result<i32> {
        let offset = offset as i32;
        let bin_array_offset = bin_array_offset as i32;
        if is_positive {
            Ok((offset + 1) * BIN_ARRAY_BITMAP_SIZE + bin_array_offset)
        } else {
            Ok(-((offset + 1) * BIN_ARRAY_BITMAP_SIZE + bin_array_offset) - 1)
        }
    }

    fn bin_array_offset_in_bitmap(bin_array_index: i32) -> Result<usize> {
        if bin_array_index > 0 {
            Ok(bin_array_index
                .checked_rem(BIN_ARRAY_BITMAP_SIZE)
                .context("overflow")? as usize)
        } else {
            Ok((-(bin_array_index + 1))
                .checked_rem(BIN_ARRAY_BITMAP_SIZE)
                .context("overflow")? as usize)
        }
    }

    fn get_bitmap_offset(bin_array_index: i32) -> Result<usize> {
        let offset = if bin_array_index > 0 {
            bin_array_index / BIN_ARRAY_BITMAP_SIZE - 1
        } else {
            -(bin_array_index + 1) / BIN_ARRAY_BITMAP_SIZE - 1
        };
        Ok(offset as usize)
    }

    fn iter_bitmap(&self, start_index: i32, end_index: i32) -> Result<Option<i32>> {
        if start_index == end_index {
            if self.bit(start_index)? {
                return Ok(Some(start_index));
            } else {
                return Ok(None);
            }
        }
        let offset: usize = Self::get_bitmap_offset(start_index)?;
        let bin_array_offset = Self::bin_array_offset_in_bitmap(start_index)?;
        if start_index < 0 {
            // iter in negative_bin_array_bitmap
            if start_index < end_index {
                for i in (0..=offset).rev() {
                    let mut bin_array_bitmap = U512::from_limbs(self.negative_bin_array_bitmap[i]);

                    if i == offset {
                        bin_array_bitmap = bin_array_bitmap
                            << BIN_ARRAY_BITMAP_SIZE as usize - bin_array_offset - 1;
                        if bin_array_bitmap.eq(&U512::ZERO) {
                            continue;
                        }

                        let bin_array_offset_in_bitmap =
                            bin_array_offset - bin_array_bitmap.leading_zeros();

                        return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                            i,
                            bin_array_offset_in_bitmap,
                            false,
                        )?));
                    }
                    if bin_array_bitmap.eq(&U512::ZERO) {
                        continue;
                    }
                    let bin_array_offset_in_bitmap =
                        BIN_ARRAY_BITMAP_SIZE as usize - bin_array_bitmap.leading_zeros() - 1;
                    return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                        i,
                        bin_array_offset_in_bitmap,
                        false,
                    )?));
                }
            } else {
                for i in offset..EXTENSION_BINARRAY_BITMAP_SIZE {
                    let mut bin_array_bitmap = U512::from_limbs(self.negative_bin_array_bitmap[i]);
                    if i == offset {
                        bin_array_bitmap = bin_array_bitmap >> bin_array_offset;
                        if bin_array_bitmap.eq(&U512::ZERO) {
                            continue;
                        }

                        let bin_array_offset_in_bitmap =
                            bin_array_offset + bin_array_bitmap.trailing_zeros();

                        return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                            i,
                            bin_array_offset_in_bitmap,
                            false,
                        )?));
                    }

                    if bin_array_bitmap.eq(&U512::ZERO) {
                        continue;
                    }
                    let bin_array_offset_in_bitmap = bin_array_bitmap.trailing_zeros();

                    return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                        i,
                        bin_array_offset_in_bitmap,
                        false,
                    )?));
                }
            }
        } else {
            // iter in possitive_bin_array_bitmap
            if start_index < end_index {
                for i in offset..EXTENSION_BINARRAY_BITMAP_SIZE {
                    let mut bin_array_bitmap = U512::from_limbs(self.positive_bin_array_bitmap[i]);
                    if i == offset {
                        bin_array_bitmap = bin_array_bitmap >> bin_array_offset;
                        if bin_array_bitmap.eq(&U512::ZERO) {
                            continue;
                        }

                        let bin_array_offset_in_bitmap =
                            bin_array_offset + bin_array_bitmap.trailing_zeros();
                        return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                            i,
                            bin_array_offset_in_bitmap,
                            true,
                        )?));
                    }

                    if bin_array_bitmap.eq(&U512::ZERO) {
                        continue;
                    }

                    let bin_array_offset_in_bitmap = bin_array_bitmap.trailing_zeros();
                    return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                        i,
                        bin_array_offset_in_bitmap,
                        true,
                    )?));
                }
            } else {
                for i in (0..=offset).rev() {
                    let mut bin_array_bitmap = U512::from_limbs(self.positive_bin_array_bitmap[i]);

                    if i == offset {
                        bin_array_bitmap = bin_array_bitmap
                            << BIN_ARRAY_BITMAP_SIZE as usize - bin_array_offset - 1;

                        if bin_array_bitmap.eq(&U512::ZERO) {
                            continue;
                        }
                        let bin_array_offset_in_bitmap =
                            bin_array_offset - bin_array_bitmap.leading_zeros();
                        return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                            i,
                            bin_array_offset_in_bitmap,
                            true,
                        )?));
                    }

                    if bin_array_bitmap.eq(&U512::ZERO) {
                        continue;
                    }
                    let bin_array_offset_in_bitmap =
                        BIN_ARRAY_BITMAP_SIZE as usize - bin_array_bitmap.leading_zeros() - 1;
                    return Ok(Some(BinArrayBitmapExtension::to_bin_array_index(
                        i,
                        bin_array_offset_in_bitmap,
                        true,
                    )?));
                }
            }
        }
        Ok(None)
    }
}
