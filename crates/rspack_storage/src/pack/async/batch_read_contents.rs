use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use rspack_error::{error, Result};

use crate::pack::{PackContents, Strategy};

pub async fn batch_read_contents(
  candidates: Vec<PathBuf>,
  strategy: Arc<dyn Strategy>,
) -> Result<Vec<PackContents>> {
  let tasks = candidates.into_iter().map(|path| {
    let strategy = strategy.to_owned();
    tokio::spawn(async move { strategy.read_contents(&path) }).map_err(|e| error!("{}", e))
  });

  let readed = join_all(tasks)
    .await
    .into_iter()
    .collect::<Result<Vec<Result<Option<PackContents>>>>>()?;

  let mut res = vec![];
  for contents in readed {
    res.push(contents?.unwrap_or_default());
  }
  Ok(res)
}
