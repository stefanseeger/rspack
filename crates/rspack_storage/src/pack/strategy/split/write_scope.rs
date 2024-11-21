use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;
use futures::TryFutureExt;
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rspack_error::{error, Result};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::{util::choose_bucket, SplitPackStrategy};
use crate::pack::{
  Pack, PackFileMeta, PackScope, PackWriteStrategy, ScopeMetaState, ScopePacksState,
  ScopeWriteStrategy, WriteScopeResult,
};

#[async_trait]
impl ScopeWriteStrategy for SplitPackStrategy {
  async fn before_save(&self) -> Result<()> {
    self.fs.remove_dir(&self.temp_root).await?;
    self.fs.ensure_dir(&self.temp_root).await?;
    self.fs.ensure_dir(&self.root).await?;
    Ok(())
  }

  async fn after_save(
    &self,
    writed_files: Vec<PathBuf>,
    removed_files: Vec<PathBuf>,
  ) -> Result<()> {
    self.move_temp_files(writed_files).await?;
    self.remove_files(removed_files).await?;
    self.fs.remove_dir(&self.temp_root).await?;
    Ok(())
  }
  async fn update_scope(
    &self,
    scope: &mut PackScope,
    updates: HashMap<Vec<u8>, Option<Vec<u8>>>,
  ) -> Result<()> {
    if !scope.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
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
    let mut bucket_task_ids = vec![];
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

      bucket_tasks.push(self.update_packs(
        scope.path.join(dirty_bucket_id.to_string()),
        scope.options.as_ref(),
        dirty_bucket_packs,
        dirty_items,
      ));
      bucket_task_ids.push(dirty_bucket_id);
    }

    // generate dirty buckets
    let dirty_bucket_results = bucket_task_ids
      .into_iter()
      .zip(join_all(bucket_tasks).await.into_iter())
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
        scope_packs[bucket_id].push(pack);
        scope_meta.packs[bucket_id].push(Arc::new(pack_meta));
      }

      scope.removed.extend(bucket_result.removed_files);
    }

    // should not remove pack files
    scope.removed.retain(|r| !total_files.contains(r));

    scope.packs = ScopePacksState::Value(scope_packs);
    scope.meta = ScopeMetaState::Value(scope_meta);

    Ok(())
  }

  async fn write_packs(&self, scope: &mut PackScope) -> Result<WriteScopeResult> {
    if !scope.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
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

    let write_results =
      batch_write_packs(candidates.iter().map(|i| i.0.clone()).collect_vec(), &self).await?;

    for ((_, meta), (hash, path, size)) in candidates.iter_mut().zip(write_results.into_iter()) {
      let _ = std::mem::replace(
        *meta,
        Arc::new(PackFileMeta {
          hash,
          size,
          name: meta.name.clone(),
          writed: true,
        }),
      );
      writed_files.insert(path);
    }

    Ok(WriteScopeResult {
      writed_files,
      removed_files,
    })
  }

  async fn write_meta(&self, scope: &mut PackScope) -> Result<WriteScopeResult> {
    if !scope.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
    let meta = scope.meta.expect_value();
    let path = self.get_temp_path(&meta.path)?;
    self
      .fs
      .ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))
      .await?;

    let mut writer = self.fs.write_file(&path).await?;

    writer
      .line(
        format!(
          "{} {} {}",
          meta.buckets, meta.max_pack_size, meta.last_modified
        )
        .as_str(),
      )
      .await?;

    for bucket_id in 0..meta.buckets {
      let line = meta
        .packs
        .get(bucket_id)
        .map(|packs| {
          packs
            .iter()
            .map(|meta| format!("{},{},{}", meta.name, meta.hash, meta.size))
            .join(" ")
        })
        .unwrap_or_default();
      writer.line(&line).await?;
    }

    writer.flush().await?;

    Ok(WriteScopeResult {
      writed_files: HashSet::from_iter(vec![meta.path.clone()]),
      removed_files: Default::default(),
    })
  }

  async fn write_scope(&self, scope: &mut PackScope) -> Result<WriteScopeResult> {
    let mut res = WriteScopeResult::default();
    res.extend(self.write_packs(scope).await?);
    res.extend(self.write_meta(scope).await?);
    Ok(res)
  }
}

async fn save_pack(pack: Pack, strategy: &SplitPackStrategy) -> Result<(String, PathBuf, usize)> {
  let keys = pack.keys.expect_value();
  let contents = pack.contents.expect_value();
  if keys.len() != contents.len() {
    return Err(error!("pack keys and contents length not match"));
  }
  strategy.write_pack(&pack).await?;
  let hash = strategy
    .get_pack_hash(&strategy.get_temp_path(&pack.path)?, keys, contents)
    .await?;
  Ok((hash, pack.path.clone(), pack.size()))
}

async fn batch_write_packs(
  packs: Vec<Pack>,
  strategy: &SplitPackStrategy,
) -> Result<Vec<(String, PathBuf, usize)>> {
  let tasks = packs.into_iter().map(|pack| {
    let strategy = strategy.to_owned();
    tokio::spawn(async move { save_pack(pack, &strategy).await }).map_err(|e| error!("{}", e))
  });

  let writed = join_all(tasks)
    .await
    .into_iter()
    .collect::<Result<Vec<Result<(String, PathBuf, usize)>>>>()?;

  let mut res = vec![];
  for item in writed {
    res.push(item?);
  }
  Ok(res)
}

#[cfg(test)]
mod tests {
  use std::{collections::HashMap, path::PathBuf, sync::Arc};

  use rspack_error::Result;

  use crate::{
    pack::{
      strategy::split::test::test_pack_utils::{
        count_bucket_packs, count_scope_packs, get_bucket_pack_sizes,
      },
      PackFs, PackMemoryFs, PackScope, ScopeWriteStrategy, SplitPackStrategy,
    },
    PackOptions,
  };

  async fn test_short_value(
    scope: &mut PackScope,
    strategy: &SplitPackStrategy,
    pre: usize,
    size: usize,
  ) -> Result<()> {
    let mut updates = HashMap::default();

    for i in pre..pre + size {
      let key = format!("{:0>4}_key", i);
      let val = format!("{:0>4}_val", i);
      updates.insert(key.as_bytes().to_vec(), Some(val.as_bytes().to_vec()));
    }
    strategy.update_scope(scope, updates).await?;

    let contents = scope.get_contents().into_iter().collect::<HashMap<_, _>>();

    assert_eq!(contents.len(), pre + size);
    assert_eq!(
      **contents
        .get(&format!("{:0>4}_key", pre).as_bytes().to_vec())
        .expect("should have key"),
      format!("{:0>4}_val", pre).as_bytes().to_vec()
    );
    assert_eq!(
      **contents
        .get(&format!("{:0>4}_key", pre + size - 1).as_bytes().to_vec())
        .expect("should have key"),
      format!("{:0>4}_val", pre + size - 1).as_bytes().to_vec()
    );

    Ok(())
  }

  async fn test_long_value(
    scope: &mut PackScope,
    strategy: &SplitPackStrategy,
    size: usize,
  ) -> Result<()> {
    let mut updates = HashMap::default();

    for i in 0..size {
      let key = format!("{:0>20}_key", i);
      let val = format!("{:0>20}_val", i);
      updates.insert(key.as_bytes().to_vec(), Some(val.as_bytes().to_vec()));
    }

    let pre_item_count = scope.get_contents().len();
    strategy.update_scope(scope, updates).await?;
    let contents = scope.get_contents().into_iter().collect::<HashMap<_, _>>();

    assert_eq!(contents.len(), pre_item_count + size);
    assert_eq!(
      **contents
        .get(&format!("{:0>20}_key", 0).as_bytes().to_vec())
        .expect("should have key"),
      format!("{:0>20}_val", 0).as_bytes().to_vec()
    );
    assert_eq!(
      **contents
        .get(&format!("{:0>20}_key", size - 1).as_bytes().to_vec())
        .expect("should have key"),
      format!("{:0>20}_val", size - 1).as_bytes().to_vec()
    );
    Ok(())
  }

  async fn test_update_value(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    let mut updates = HashMap::default();
    let key = format!("{:0>4}_key", 0);
    let val = format!("{:0>4}_new", 0);
    updates.insert(key.as_bytes().to_vec(), Some(val.as_bytes().to_vec()));

    let pre_item_count = scope.get_contents().len();
    strategy.update_scope(scope, updates).await?;
    let contents = scope.get_contents().into_iter().collect::<HashMap<_, _>>();

    assert_eq!(contents.len(), pre_item_count);

    assert_eq!(
      **contents
        .get(&format!("{:0>4}_key", 0).as_bytes().to_vec())
        .expect("should have key"),
      format!("{:0>4}_new", 0).as_bytes().to_vec()
    );

    Ok(())
  }

  async fn test_remove_value(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    let mut updates = HashMap::default();
    let key = format!("{:0>4}_key", 1);
    updates.insert(key.as_bytes().to_vec(), None);
    let pre_item_count = scope.get_contents().len();
    strategy.update_scope(scope, updates).await?;
    let contents = scope.get_contents().into_iter().collect::<HashMap<_, _>>();

    assert_eq!(contents.len(), pre_item_count - 1);
    assert!(contents
      .get(&format!("{:0>4}_key", 1).as_bytes().to_vec())
      .is_none());
    Ok(())
  }

  async fn test_single_bucket(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    test_short_value(scope, strategy, 0, 10).await?;
    assert_eq!(count_scope_packs(&scope), 5);
    let res = strategy.write_scope(scope).await?;
    // 5 packs + 1 meta
    assert_eq!(res.writed_files.len(), 6);
    assert_eq!(res.removed_files.len(), 0);

    test_long_value(scope, strategy, 5).await?;
    assert_eq!(count_scope_packs(&scope), 10);
    let res = strategy.write_scope(scope).await?;
    // 5 packs + 1 meta
    assert_eq!(res.writed_files.len(), 6);
    assert_eq!(res.removed_files.len(), 0);

    test_update_value(scope, strategy).await?;
    assert_eq!(count_scope_packs(&scope), 10);
    let res = strategy.write_scope(scope).await?;
    // 1 packs + 1 meta
    assert_eq!(res.writed_files.len(), 2);
    assert_eq!(res.removed_files.len(), 1);

    test_remove_value(scope, strategy).await?;
    assert_eq!(count_scope_packs(&scope), 10);
    let res = strategy.write_scope(scope).await?;
    // 1 packs + 1 meta
    assert_eq!(res.writed_files.len(), 2);
    assert_eq!(res.removed_files.len(), 1);

    Ok(())
  }

  async fn test_multi_bucket(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    test_short_value(scope, strategy, 0, 100).await?;
    assert_eq!(count_bucket_packs(&scope), vec![5; 10]);

    let res = strategy.write_scope(scope).await?;
    // 50 packs + 1 meta
    assert_eq!(res.writed_files.len(), 51);
    assert_eq!(res.removed_files.len(), 0);

    test_long_value(scope, strategy, 50).await?;
    assert_eq!(count_bucket_packs(&scope), vec![10; 10]);
    let res = strategy.write_scope(scope).await?;
    // 50 packs + 1 meta
    assert_eq!(res.writed_files.len(), 51);
    assert_eq!(res.removed_files.len(), 0);

    test_update_value(scope, strategy).await?;
    assert_eq!(count_bucket_packs(&scope), vec![10; 10]);
    let res = strategy.write_scope(scope).await?;
    // 1 packs + 1 meta
    assert_eq!(res.writed_files.len(), 2);
    assert_eq!(res.removed_files.len(), 1);

    test_remove_value(scope, strategy).await?;
    assert_eq!(count_bucket_packs(&scope), vec![10; 10]);
    let res = strategy.write_scope(scope).await?;
    // 1 packs + 1 meta
    assert_eq!(res.writed_files.len(), 2);
    assert_eq!(res.removed_files.len(), 1);

    Ok(())
  }

  async fn test_big_bucket(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    // 200 * 16 = 3200 = 2000 + 1200
    test_short_value(scope, strategy, 0, 200).await?;
    assert_eq!(count_scope_packs(&scope), 2);
    assert_eq!(get_bucket_pack_sizes(&scope), [1200, 2000]);

    // 3200 + 100 * 16 = 4800 = 2000 + 2000 + 800
    test_short_value(scope, strategy, 200, 100).await?;
    assert_eq!(count_scope_packs(&scope), 3);
    assert_eq!(get_bucket_pack_sizes(&scope), [800, 2000, 2000]);

    // 4800 + 60 * 16 = 5760 = 2000 + 2000 + 1760(>1600)
    test_short_value(scope, strategy, 300, 60).await?;
    assert_eq!(count_scope_packs(&scope), 3);
    assert_eq!(get_bucket_pack_sizes(&scope), [1760, 2000, 2000]);

    // 5760 + 160 = 5920 = 2000 + 2000 + 1760(>1600) + 160
    test_short_value(scope, strategy, 360, 10).await?;
    assert_eq!(count_scope_packs(&scope), 4);
    assert_eq!(get_bucket_pack_sizes(&scope), [160, 1760, 2000, 2000]);

    Ok(())
  }

  async fn clean_scope_path(scope: &PackScope, strategy: &SplitPackStrategy, fs: Arc<dyn PackFs>) {
    fs.remove_dir(&scope.path).await.expect("should remove dir");
    fs.remove_dir(
      &strategy
        .get_temp_path(&scope.path)
        .expect("should get temp path"),
    )
    .await
    .expect("should remove dir");
  }

  #[tokio::test]
  async fn should_write_single_bucket_scope() {
    let fs = Arc::new(PackMemoryFs::default());
    let strategy =
      SplitPackStrategy::new(PathBuf::from("/cache"), PathBuf::from("/temp"), fs.clone());
    let options = Arc::new(PackOptions {
      buckets: 1,
      max_pack_size: 32,
      expires: 1000000,
    });
    let mut scope = PackScope::empty(PathBuf::from("/cache/test_single_bucket"), options.clone());
    clean_scope_path(&scope, &strategy, fs.clone()).await;

    let _ = test_single_bucket(&mut scope, &strategy)
      .await
      .map_err(|e| {
        panic!("{}", e);
      });
  }

  #[tokio::test]
  async fn should_write_multi_bucket_scope() {
    let fs = Arc::new(PackMemoryFs::default());
    let strategy =
      SplitPackStrategy::new(PathBuf::from("/cache"), PathBuf::from("/temp"), fs.clone());
    let options = Arc::new(PackOptions {
      buckets: 10,
      max_pack_size: 32,
      expires: 1000000,
    });
    let mut scope = PackScope::empty(PathBuf::from("/cache/test_multi_bucket"), options.clone());
    clean_scope_path(&scope, &strategy, fs.clone()).await;

    let _ = test_multi_bucket(&mut scope, &strategy).await.map_err(|e| {
      panic!("{}", e);
    });
  }

  #[tokio::test]
  async fn should_write_big_bucket_scope() {
    let fs = Arc::new(PackMemoryFs::default());
    let strategy =
      SplitPackStrategy::new(PathBuf::from("/cache"), PathBuf::from("/temp"), fs.clone());
    let options = Arc::new(PackOptions {
      buckets: 1,
      max_pack_size: 2000,
      expires: 1000000,
    });
    let mut scope = PackScope::empty(PathBuf::from("/cache/test_big_bucket"), options.clone());
    clean_scope_path(&scope, &strategy, fs.clone()).await;

    let _ = test_big_bucket(&mut scope, &strategy).await.map_err(|e| {
      panic!("{}", e);
    });
  }
}
