use anyhow::{ensure, Context, Result};
use num_integer::Integer;
use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};
use crate::instruction::utils::meteora_dlmm::constants::MAX_BIN_PER_ARRAY;
use crate::instruction::utils::meteora_dlmm::extensions::Bin;
use crate::instruction::utils::meteora_dlmm::pda::derive_bin_array_pda;

pub struct BinArray {
    pub index: i64,
    /// Version of binArray
    pub version: u8,
    _padding: [u8; 7],
    pub lb_pair: [u8; 32], // pubkey 通常是 32 字节
    pub bins: [Bin; 70],
}

pub trait BinArrayExtension {
    fn is_bin_id_within_range(&self, bin_id: i32) -> Result<bool>;
    fn get_bin_index_in_array(&self, bin_id: i32) -> Result<usize>;

    fn get_bin_array_lower_upper_bin_id(index: i32) -> Result<(i32, i32)>;
    fn bin_id_to_bin_array_index(bin_id: i32) -> Result<i32>;
    fn bin_id_to_bin_array_key(lb_pair: Pubkey, bin_id: i32) -> Result<Pubkey>;

    fn get_bin_mut<'a>(&'a mut self, bin_id: i32) -> Result<&'a mut Bin>;
    fn get_bin<'a>(&'a self, bin_id: i32) -> Result<&'a Bin>;

    fn get_bin_array_account_metas_coverage(
        lower_bin_id: i32,
        upper_bin_id: i32,
        lb_pair: Pubkey,
    ) -> Result<Vec<AccountMeta>>;

    fn get_bin_array_indexes_coverage(lower_bin_id: i32, upper_bin_id: i32) -> Result<Vec<i32>>;
}

impl BinArrayExtension for BinArray {
    fn get_bin_array_lower_upper_bin_id(index: i32) -> Result<(i32, i32)> {
        let lower_bin_id = index
            .checked_mul(MAX_BIN_PER_ARRAY as i32)
            .context("overflow")?;

        let upper_bin_id = lower_bin_id
            .checked_add(MAX_BIN_PER_ARRAY as i32)
            .context("overflow")?
            .checked_sub(1)
            .context("overflow")?;

        Ok((lower_bin_id, upper_bin_id))
    }

    fn is_bin_id_within_range(&self, bin_id: i32) -> Result<bool> {
        let (lower_bin_id, upper_bin_id) =
            BinArray::get_bin_array_lower_upper_bin_id(self.index as i32)?;

        Ok(bin_id >= lower_bin_id && bin_id <= upper_bin_id)
    }

    fn get_bin_mut<'a>(&'a mut self, bin_id: i32) -> Result<&'a mut Bin> {
        Ok(&mut self.bins[self.get_bin_index_in_array(bin_id)?])
    }

    fn get_bin<'a>(&'a self, bin_id: i32) -> Result<&'a Bin> {
        Ok(&self.bins[self.get_bin_index_in_array(bin_id)?])
    }

    fn get_bin_index_in_array(&self, bin_id: i32) -> Result<usize> {
        ensure!(self.is_bin_id_within_range(bin_id)?, "Bin id out of range");
        let (lower_bin_id, _) = BinArray::get_bin_array_lower_upper_bin_id(self.index as i32)?;
        let index = bin_id.checked_sub(lower_bin_id).context("overflow")?;
        Ok(index as usize)
    }

    fn bin_id_to_bin_array_index(bin_id: i32) -> Result<i32> {
        let (idx, rem) = bin_id.div_rem(&(MAX_BIN_PER_ARRAY as i32));

        if bin_id.is_negative() && rem != 0 {
            Ok(idx.checked_sub(1).context("overflow")?)
        } else {
            Ok(idx)
        }
    }

    fn bin_id_to_bin_array_key(lb_pair: Pubkey, bin_id: i32) -> Result<Pubkey> {
        let bin_array_index = Self::bin_id_to_bin_array_index(bin_id)?;
        Ok(derive_bin_array_pda(lb_pair, bin_array_index.into()).0)
    }

    fn get_bin_array_indexes_coverage(lower_bin_id: i32, upper_bin_id: i32) -> Result<Vec<i32>> {
        let lower_idx = BinArray::bin_id_to_bin_array_index(lower_bin_id)?;
        let upper_idx = BinArray::bin_id_to_bin_array_index(upper_bin_id)?;

        let mut indexes = vec![];

        for i in lower_idx..=upper_idx {
            indexes.push(i);
        }

        Ok(indexes)
    }

    fn get_bin_array_account_metas_coverage(
        lower_bin_id: i32,
        upper_bin_id: i32,
        lb_pair: Pubkey,
    ) -> Result<Vec<AccountMeta>> {
        let bin_array_indexes =
            BinArray::get_bin_array_indexes_coverage(lower_bin_id, upper_bin_id)?;

        Ok(bin_array_indexes
            .into_iter()
            .map(|index| AccountMeta {
                pubkey: derive_bin_array_pda(lb_pair, index.into()).0,
                is_signer: false,
                is_writable: true,
            })
            .collect())
    }
}
