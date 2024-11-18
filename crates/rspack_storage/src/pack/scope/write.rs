use std::path::PathBuf;
use std::{hash::Hasher, sync::Arc};

use futures::future::join_all;
use futures::TryFutureExt;
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rspack_error::{error, Result};
use rustc_hash::FxHasher;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::{PackFileMeta, PackScope, ScopeMetaState, ScopePacksState};
use crate::pack::{Pack, Strategy};

pub fn update_scope(
  scope: &mut PackScope,
  updates: HashMap<Vec<u8>, Option<Vec<u8>>>,
) -> Result<()> {
  let mut scope_meta = scope.meta.take_value().expect("shoud have scope meta");
  let mut scope_packs = scope.packs.take_value().expect("shoud have scope packs");

  // get changed buckets
  let bucket_updates = updates
    .into_par_iter()
    .map(|(key, value)| {
      let bucket_id = choose_bucket(&key, scope.options.buckets);
      (bucket_id, key, value)
    })
    .collect::<Vec<_>>()
    .into_iter()
    .fold(
      HashMap::<usize, HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>>::default(),
      |mut res, (bucket_id, key, value)| {
        res
          .entry(bucket_id)
          .or_default()
          .insert(Arc::new(key.to_owned()), value.to_owned().map(Arc::new));
        res
      },
    );

  // get dirty buckets
  let mut bucket_tasks = vec![];
  for (dirty_bucket_id, dirty_items) in bucket_updates.into_iter() {
    let dirty_bucket_packs = {
      let mut packs = HashMap::default();

      let old_dirty_bucket_metas = std::mem::take(
        scope_meta
          .packs
          .get_mut(dirty_bucket_id)
          .expect("should have bucket pack metas"),
      )
      .into_iter()
      .enumerate()
      .collect::<HashMap<_, _>>();

      let mut old_dirty_bucket_packs = std::mem::take(
        scope_packs
          .get_mut(dirty_bucket_id)
          .expect("should have bucket packs"),
      )
      .into_iter()
      .enumerate()
      .collect::<HashMap<_, _>>();

      for (key, pack_meta) in old_dirty_bucket_metas.into_iter() {
        let pack = old_dirty_bucket_packs
          .remove(&key)
          .expect("should have bucket pack");
        packs.insert(pack_meta, pack);
      }
      packs
    };

    bucket_tasks.push((dirty_bucket_id, dirty_bucket_packs, dirty_items));
  }

  // generate dirty buckets
  let dirty_bucket_results = bucket_tasks
    .into_par_iter()
    .map(|(bucket_id, mut bucket_packs, mut bucket_updates)| {
      let bucket_res = scope.strategy.incremental(
        scope.path.join(bucket_id.to_string()),
        scope.options.clone(),
        &mut bucket_packs,
        &mut bucket_updates,
      );
      (bucket_id, bucket_res)
    })
    .collect::<HashMap<_, _>>();

  let mut total_files = HashSet::default();
  // link remain packs to scope
  for (bucket_id, bucket_result) in dirty_bucket_results {
    for (pack_meta, pack) in bucket_result.remain_packs {
      total_files.insert(pack.path.clone());
      scope_packs[bucket_id].push(pack);
      scope_meta.packs[bucket_id].push(pack_meta);
    }

    for (pack_meta, pack) in bucket_result.new_packs {
      total_files.insert(pack.path.clone());
      scope_packs[bucket_id].push(pack);
      scope_meta.packs[bucket_id].push(Arc::new(pack_meta));
    }

    scope.removed.extend(bucket_result.removed_files);
  }

  // should not remove pack files
  scope.removed = std::mem::take(&mut scope.removed)
    .into_iter()
    .filter(|r| total_files.contains(r))
    .collect();

  scope.packs = ScopePacksState::Value(scope_packs);
  scope.meta = ScopeMetaState::Value(scope_meta);

  Ok(())
}

fn choose_bucket(key: &Vec<u8>, total: usize) -> usize {
  let mut hasher = FxHasher::default();
  hasher.write(key);
  let bucket_id = usize::try_from(hasher.finish() % total as u64).expect("should get bucket id");
  bucket_id
}

pub struct ScopeSaveResult {
  pub writed_files: HashSet<PathBuf>,
  pub removed_files: HashSet<PathBuf>,
}

pub async fn save_scope(scope: &mut PackScope) -> Result<ScopeSaveResult> {
  let removed_files = std::mem::take(&mut scope.removed);
  let packs = scope.packs.expect_value();
  let meta = scope.meta.expect_value_mut();

  let mut writed_files = HashSet::default();

  let mut candidates = packs
    .iter()
    .flatten()
    .zip(meta.packs.iter_mut().flatten())
    .filter(|(_, meta)| !meta.writed)
    .collect_vec();

  let write_results = batch_write_packs(
    candidates.iter().map(|i| i.0.clone()).collect_vec(),
    scope.strategy.clone(),
  )
  .await?;

  for ((_, meta), (hash, path)) in candidates.iter_mut().zip(write_results.into_iter()) {
    let _ = std::mem::replace(
      *meta,
      Arc::new(PackFileMeta {
        hash,
        name: meta.name.clone(),
        writed: true,
      }),
    );
    writed_files.insert(path);
  }

  let meta = scope.meta.expect_value();
  writed_files.insert(meta.path.clone());
  scope.strategy.write_scope_meta(&meta)?;

  Ok(ScopeSaveResult {
    writed_files,
    removed_files,
  })
}

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

async fn batch_write_packs(
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
