use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use rspack_error::{error, Result};

use crate::pack::{PackKeys, Strategy};

pub async fn batch_read_keys(
  candidates: Vec<PathBuf>,
  strategy: Arc<dyn Strategy>,
) -> Result<Vec<PackKeys>> {
  let tasks = candidates.into_iter().map(|path| {
    let strategy = strategy.to_owned();
    tokio::spawn(async move { strategy.read_keys(&path) }).map_err(|e| error!("{}", e))
  });

  let readed = join_all(tasks)
    .await
    .into_iter()
    .collect::<Result<Vec<Result<Option<PackKeys>>>>>()?;

  let mut res = vec![];
  for keys in readed {
    res.push(keys?.unwrap_or_default());
  }
  Ok(res)
}
