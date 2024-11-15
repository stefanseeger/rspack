use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use rspack_error::{error, Result};

use crate::pack::{Pack, Strategy};

async fn save_pack(pack: Pack, strategy: Arc<dyn Strategy>) -> Result<(String, PathBuf)> {
  let keys = pack.keys.expect_value();
  let contents = pack.contents.expect_value();
  if keys.len() != contents.len() {
    return Err(error!("pack keys and contents length not match"));
  }
  strategy.write(&pack.path, keys, contents)?;
  let hash = strategy.get_hash(&strategy.get_temp_path(&pack.path)?, keys, contents)?;
  Ok((hash, pack.path.clone()))
}

pub async fn batch_write_packs(
  packs: Vec<Pack>,
  strategy: Arc<dyn Strategy>,
) -> Result<Vec<(String, PathBuf)>> {
  let tasks = packs.into_iter().map(|pack| {
    let strategy = strategy.clone();
    tokio::spawn(async move { save_pack(pack, strategy).await }).map_err(|e| error!("{}", e))
  });

  let writed = join_all(tasks)
    .await
    .into_iter()
    .collect::<Result<Vec<Result<(String, PathBuf)>>>>()?;

  let mut res = vec![];
  for item in writed {
    res.push(item?);
  }
  Ok(res)
}
