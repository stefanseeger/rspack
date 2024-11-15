use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use rspack_error::{error, Result};

use crate::pack::{PackContents, PackKeys, Strategy};

pub struct PackValidateCandidate {
  pub path: PathBuf,
  pub hash: String,
  pub keys: PackKeys,
  pub contents: PackContents,
}

pub fn validate_pack(
  hash: &str,
  path: &PathBuf,
  keys: &PackKeys,
  contents: &PackContents,
  strategy: Arc<dyn Strategy>,
) -> Result<bool> {
  let pack_hash = strategy.get_hash(path, keys, contents)?;
  Ok(*hash == pack_hash)
}

pub async fn batch_validate(
  candidates: Vec<PackValidateCandidate>,
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
