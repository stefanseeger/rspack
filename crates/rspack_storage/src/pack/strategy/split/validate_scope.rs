use std::{
  path::PathBuf,
  time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use rspack_error::{error, Result};

use super::{util::get_indexed_packs, SplitPackStrategy};
use crate::pack::{PackContents, PackKeys, PackScope, ScopeValidateStrategy, ValidateResult};

#[derive(Debug)]
pub struct ValidatingPack {
  pub path: PathBuf,
  pub hash: String,
  pub keys: PackKeys,
  pub contents: PackContents,
}

#[async_trait]
impl ScopeValidateStrategy for SplitPackStrategy {
  async fn validate_meta(&self, scope: &mut PackScope) -> Result<ValidateResult> {
    let meta = scope.meta.expect_value();
    if meta.buckets != scope.options.buckets || meta.max_pack_size != scope.options.max_pack_size {
      return Ok(ValidateResult::Invalid("scope options changed".to_string()));
    }

    let current_time = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("get current time failed")
      .as_secs();

    if current_time - meta.last_modified > scope.options.expires {
      return Ok(ValidateResult::Invalid("scope expired".to_string()));
    }

    return Ok(ValidateResult::Valid);
  }

  async fn validate_packs(&self, scope: &mut PackScope) -> Result<ValidateResult> {
    let (_, pack_list) = get_indexed_packs(scope)?;

    let candidates = pack_list
      .into_iter()
      .map(|(pack_meta, pack)| ValidatingPack {
        path: pack.path.to_owned(),
        hash: pack_meta.hash.to_owned(),
        keys: pack.keys.expect_value().to_owned(),
        contents: pack.keys.expect_value().to_owned(),
      })
      .collect_vec();
    let validate_results = batch_validate_packs(candidates, self).await?;
    if validate_results.into_iter().all(|v| v) {
      Ok(ValidateResult::Valid)
    } else {
      Ok(ValidateResult::Invalid("packs is not validate".to_string()))
    }
  }
}

async fn batch_validate_packs(
  candidates: Vec<ValidatingPack>,
  strategy: &SplitPackStrategy,
) -> Result<Vec<bool>> {
  let tasks = candidates.into_iter().map(|pack| {
    let strategy = strategy.clone();
    tokio::spawn(async move {
      match strategy
        .get_pack_hash(&pack.path, &pack.keys, &pack.contents)
        .await
      {
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
