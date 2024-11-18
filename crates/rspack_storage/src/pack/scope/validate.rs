use std::{
  path::PathBuf,
  sync::Arc,
  time::{SystemTime, UNIX_EPOCH},
};

use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use pollster::block_on;
use rspack_error::{error, Result};
use tokio::task::unconstrained;

use super::{get_pack_meta_pairs, PackScope};
use crate::{
  pack::{PackContents, PackKeys, Strategy},
  PackStorageOptions,
};

#[derive(Debug)]
pub enum ValidateResult {
  Valid,
  Invalid(String),
}

#[derive(Debug)]
pub struct ValidatingPack {
  pub path: PathBuf,
  pub hash: String,
  pub keys: PackKeys,
  pub contents: PackContents,
}

pub fn validate_meta(scope: &PackScope, options: &PackStorageOptions) -> Result<ValidateResult> {
  let meta = scope.meta.expect_value();
  if meta.buckets != options.buckets || meta.max_pack_size != options.max_pack_size {
    return Ok(ValidateResult::Invalid("scope options changed".to_string()));
  }

  let current_time = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("get current time failed")
    .as_secs();

  if current_time - meta.last_modified > options.expires {
    return Ok(ValidateResult::Invalid("scope expired".to_string()));
  }

  return Ok(ValidateResult::Valid);
}

pub fn validate_packs(scope: &PackScope) -> Result<ValidateResult> {
  let candidates = get_pack_meta_pairs(scope)?
    .iter()
    .map(|(bucket_id, _, pack_meta, pack)| ValidatingPack {
      path: scope.path.join(bucket_id.to_string()).join(&pack_meta.name),
      hash: pack_meta.hash.to_owned(),
      keys: pack.keys.expect_value().to_owned(),
      contents: pack.keys.expect_value().to_owned(),
    })
    .collect_vec();
  let validate_results = block_on(unconstrained(batch_validate_packs(
    candidates,
    scope.strategy.clone(),
  )))?;
  if validate_results.into_iter().all(|v| v) {
    Ok(ValidateResult::Valid)
  } else {
    Ok(ValidateResult::Invalid("packs is not validate".to_string()))
  }
}

async fn batch_validate_packs(
  candidates: Vec<ValidatingPack>,
  strategy: Arc<dyn Strategy>,
) -> Result<Vec<bool>> {
  let tasks = candidates.into_iter().map(|pack| {
    let strategy = strategy.clone();
    tokio::spawn(async move {
      match strategy.get_hash(&pack.path, &pack.keys, &pack.contents) {
        Ok(res) => pack.hash == res,
        Err(_) => false,
      }
    })
    .map_err(|e| error!("{}", e))
  });

  join_all(tasks)
    .await
    .into_iter()
    .collect::<Result<Vec<bool>>>()
}
