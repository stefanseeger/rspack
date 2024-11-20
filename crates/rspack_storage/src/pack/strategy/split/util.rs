use std::{hash::Hasher, sync::Arc};

use itertools::Itertools;
use rspack_error::Result;
use rustc_hash::FxHasher;

use crate::pack::{Pack, PackContents, PackFileMeta, PackKeys, PackScope};

pub fn get_pack_meta_pairs(
  scope: &PackScope,
) -> Result<Vec<(usize, usize, Arc<PackFileMeta>, &Pack)>> {
  let meta = scope.meta.expect_value();
  let packs = scope.packs.expect_value();

  Ok(
    meta
      .packs
      .iter()
      .enumerate()
      .map(|(bucket_id, pack_meta_list)| {
        let bucket_packs = packs.get(bucket_id).expect("should have bucket packs");
        pack_meta_list
          .iter()
          .enumerate()
          .map(|(pack_pos, pack_meta)| {
            (
              bucket_id,
              pack_pos,
              pack_meta.clone(),
              bucket_packs.get(pack_pos).expect("should have bucket pack"),
            )
          })
          .collect_vec()
      })
      .flatten()
      .collect_vec(),
  )
}

pub fn get_name(keys: &PackKeys, _: &PackContents) -> String {
  let mut hasher = FxHasher::default();
  for k in keys {
    hasher.write(k);
  }
  hasher.write_usize(keys.len());

  format!("{:016x}", hasher.finish())
}

pub fn choose_bucket(key: &Vec<u8>, total: usize) -> usize {
  let num = key.iter().fold(0_usize, |acc, i| acc + *i as usize);
  num % total
}
