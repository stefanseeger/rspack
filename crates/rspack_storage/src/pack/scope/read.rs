use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use pollster::block_on;
use rspack_error::{error, Result};
use tokio::task::unconstrained;

use super::{PackFileMeta, PackScope, ScopeMeta, ScopePacks};
use crate::pack::{Pack, PackContents, PackContentsState, PackKeys, PackKeysState, Strategy};

pub struct ReadKeysResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub keys: PackKeys,
}

pub fn read_keys(scope: &PackScope) -> Result<Vec<ReadKeysResult>> {
  let candidates = get_pack_meta_pairs(scope)?
    .into_iter()
    .filter(|(_, _, _, pack)| matches!(pack.keys, PackKeysState::Pending))
    .map(|args| (args.0, args.1, args.3.path.to_owned()))
    .collect_vec();
  let readed = block_on(unconstrained(batch_read_keys(
    candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
    scope.strategy.clone(),
  )))?;

  Ok(
    readed
      .into_iter()
      .zip(candidates.into_iter())
      .map(|(keys, (bucket_id, pack_pos, _))| ReadKeysResult {
        bucket_id,
        pack_pos,
        keys,
      })
      .collect_vec(),
  )
}

async fn batch_read_keys(
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

pub struct ReacContentsResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub contents: PackContents,
}

pub fn read_contents(scope: &PackScope) -> Result<Vec<ReacContentsResult>> {
  let candidates = get_pack_meta_pairs(scope)?
    .into_iter()
    .filter(|(_, _, _, pack)| {
      matches!(pack.contents, PackContentsState::Pending)
        && matches!(pack.keys, PackKeysState::Value(_))
    })
    .map(|args| (args.0, args.1, args.3.path.to_owned()))
    .collect_vec();

  let readed: Vec<Vec<Arc<Vec<u8>>>> = block_on(unconstrained(batch_read_contents(
    candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
    scope.strategy.clone(),
  )))?;

  Ok(
    readed
      .into_iter()
      .zip(candidates.into_iter())
      .map(|(contents, (bucket_id, pack_pos, _))| ReacContentsResult {
        bucket_id,
        pack_pos,
        contents,
      })
      .collect_vec(),
  )
}

async fn batch_read_contents(
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

pub fn read_meta(scope: &PackScope) -> Result<ScopeMeta> {
  let scope_path = ScopeMeta::get_path(&scope.path);
  let meta = scope.strategy.read_scope_meta(&scope_path)?;
  if let Some(meta) = meta {
    Ok(meta)
  } else {
    Ok(ScopeMeta::new(&scope.path, scope.options.clone()))
  }
}

pub fn read_packs(scope: &PackScope) -> Result<ScopePacks> {
  Ok(
    scope
      .meta
      .expect_value()
      .packs
      .iter()
      .enumerate()
      .map(|(bucket_id, pack_meta_list)| {
        let bucket_dir = scope.path.join(bucket_id.to_string());
        pack_meta_list
          .iter()
          .map(|pack_meta| Pack::new(bucket_dir.join(&pack_meta.name)))
          .collect_vec()
      })
      .collect_vec(),
  )
}

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
