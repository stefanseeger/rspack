use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use rspack_error::{error, Result};

use super::{util::get_pack_meta_pairs, SplitPackStrategy};
use crate::pack::{
  Pack, PackContents, PackContentsState, PackFileMeta, PackFs, PackKeys, PackKeysState,
  PackReadStrategy, PackScope, ScopeMeta, ScopeMetaState, ScopePacksState, ScopeReadStrategy,
};

#[async_trait]
impl ScopeReadStrategy for SplitPackStrategy {
  async fn ensure_meta(&self, scope: &mut PackScope) -> Result<()> {
    if matches!(scope.meta, ScopeMetaState::Pending) {
      let scope_path = ScopeMeta::get_path(&scope.path);
      let meta = read_scope_meta(&scope_path, self.fs.clone())
        .await?
        .unwrap_or_else(|| ScopeMeta::new(&scope.path, &scope.options));
      scope.meta = ScopeMetaState::Value(meta);
    }
    Ok(())
  }

  async fn ensure_packs(&self, scope: &mut PackScope) -> Result<()> {
    self.ensure_meta(scope).await?;

    if matches!(scope.packs, ScopePacksState::Pending) {
      scope.packs = ScopePacksState::Value(
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
      );
    }
    Ok(())
  }

  async fn ensure_keys(&self, scope: &mut PackScope) -> Result<()> {
    self.ensure_packs(scope).await?;

    let packs_results = read_keys(&scope, self).await?;
    let packs = scope.packs.expect_value_mut();
    for pack_res in packs_results {
      if let Some(pack) = packs
        .get_mut(pack_res.bucket_id)
        .and_then(|packs| packs.get_mut(pack_res.pack_pos))
      {
        pack.keys = PackKeysState::Value(pack_res.keys);
      }
    }
    Ok(())
  }

  async fn ensure_contents(&self, scope: &mut PackScope) -> Result<()> {
    self.ensure_keys(scope).await?;

    let packs_results = read_contents(&scope, self).await?;
    let packs = scope.packs.expect_value_mut();
    for pack_res in packs_results {
      if let Some(pack) = packs
        .get_mut(pack_res.bucket_id)
        .and_then(|packs| packs.get_mut(pack_res.pack_pos))
      {
        pack.contents = PackContentsState::Value(pack_res.contents);
      }
    }
    Ok(())
  }

  fn get_path(&self, str: &str) -> PathBuf {
    self.root.join(str)
  }
}

async fn read_scope_meta(path: &PathBuf, fs: Arc<dyn PackFs>) -> Result<Option<ScopeMeta>> {
  if !fs.exists(path).await? {
    return Ok(None);
  }

  let mut reader = fs.read_file(path).await?;

  let meta_options = reader
    .line()
    .await?
    .split(" ")
    .map(|item| {
      item
        .parse::<usize>()
        .map_err(|e| error!("parse meta file failed: {}", e))
    })
    .collect::<Result<Vec<usize>>>()?;

  if meta_options.len() < 3 {
    return Err(error!("meta broken"));
  }

  let buckets = meta_options[0];
  let max_pack_size = meta_options[1];
  let last_modified = meta_options[2] as u64;

  let mut packs = vec![];
  for _ in 0..buckets {
    packs.push(
      reader
        .line()
        .await?
        .split(" ")
        .map(|i| i.split(",").collect::<Vec<_>>())
        .map(|i| {
          if i.len() < 3 {
            Err(error!("parse pack file info failed"))
          } else {
            Ok(Arc::new(PackFileMeta {
              name: i[0].to_owned(),
              hash: i[1].to_owned(),
              size: i[2].parse::<usize>().expect("should parse pack size"),
              writed: true,
            }))
          }
        })
        .collect::<Result<Vec<Arc<PackFileMeta>>>>()?,
    );
  }

  if packs.len() < buckets {
    return Err(error!("parse meta buckets failed"));
  }

  Ok(Some(ScopeMeta {
    path: path.clone(),
    buckets,
    max_pack_size,
    last_modified,
    packs,
  }))
}

#[derive(Debug)]
struct ReadKeysResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub keys: PackKeys,
}

async fn read_keys(scope: &PackScope, strategy: &SplitPackStrategy) -> Result<Vec<ReadKeysResult>> {
  let (candidates_index_list, pack_list) = get_pack_meta_pairs(scope)?;
  let tasks = pack_list
    .into_iter()
    .map(|i| {
      let strategy = strategy.to_owned();
      let path = i.1.path.to_owned();
      tokio::spawn(async move { strategy.read_pack_keys(&path).await }).map_err(|e| error!("{}", e))
    })
    .collect_vec();
  let readed = join_all(tasks).await.into_iter().process_results(|iter| {
    iter
      .into_iter()
      .process_results(|iter| iter.map(|x| x.unwrap_or_default()).collect_vec())
  })??;

  Ok(
    readed
      .into_iter()
      .zip(candidates_index_list.into_iter())
      .map(|(keys, (bucket_id, pack_pos))| ReadKeysResult {
        bucket_id,
        pack_pos,
        keys,
      })
      .collect_vec(),
  )
}

#[derive(Debug)]
struct ReadContentsResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub contents: PackContents,
}

async fn read_contents(
  scope: &PackScope,
  strategy: &SplitPackStrategy,
) -> Result<Vec<ReadContentsResult>> {
  let (candidates_index_list, pack_list) = get_pack_meta_pairs(scope)?;
  let tasks = pack_list
    .into_iter()
    .map(|i| {
      let strategy = strategy.to_owned();
      let path = i.1.path.to_owned();
      tokio::spawn(async move { strategy.read_pack_contents(&path).await })
        .map_err(|e| error!("{}", e))
    })
    .collect_vec();
  let readed = join_all(tasks).await.into_iter().process_results(|iter| {
    iter
      .into_iter()
      .process_results(|iter| iter.map(|x| x.unwrap_or_default()).collect_vec())
  })??;

  Ok(
    readed
      .into_iter()
      .zip(candidates_index_list.into_iter())
      .map(|(contents, (bucket_id, pack_pos))| ReadContentsResult {
        bucket_id,
        pack_pos,
        contents,
      })
      .collect_vec(),
  )
}

#[cfg(test)]
mod tests {

  use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
  };

  use itertools::Itertools;
  use rspack_error::Result;

  use crate::{
    pack::{
      PackFileMeta, PackFs, PackMemoryFs, PackScope, ScopeMeta, ScopeReadStrategy,
      SplitPackStrategy,
    },
    PackOptions,
  };

  async fn mock_packs(path: &PathBuf, fs: Arc<dyn PackFs>, options: &PackOptions) -> Result<()> {
    for bucket_id in 0..options.buckets {
      for pack_no in 0..3 {
        let pack_name = format!("pack_name_{}_{}", bucket_id, pack_no);
        fs.ensure_dir(&path.join(bucket_id.to_string())).await?;
        let mut writer = fs
          .write_file(&path.join(format!("./{}/{}", bucket_id, pack_name)))
          .await?;
        let mut keys = vec![];
        let mut contents = vec![];
        for i in 0..10 {
          keys.push(
            format!("key_{}_{}_{}", bucket_id, pack_no, i)
              .as_bytes()
              .to_vec(),
          );
          contents.push(
            format!("val_{}_{}_{}", bucket_id, pack_no, i)
              .as_bytes()
              .to_vec(),
          );
        }
        writer
          .line(keys.iter().map(|k| k.len()).join(" ").as_str())
          .await?;
        writer
          .line(contents.iter().map(|k| k.len()).join(" ").as_str())
          .await?;
        for key in keys {
          writer.bytes(&key).await?;
        }
        for content in contents {
          writer.bytes(&content).await?;
        }
        writer.flush().await?;
      }
    }

    Ok(())
  }

  async fn mock_meta(path: &PathBuf, fs: Arc<dyn PackFs>, options: &PackOptions) -> Result<()> {
    fs.ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))
      .await?;
    let mut writer = fs.write_file(path).await?;
    let current = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("should get current time")
      .as_millis() as u64;
    writer
      .line(format!("{} {} {}", options.buckets, options.max_pack_size, current).as_str())
      .await?;
    for bucket_id in 0..options.buckets {
      let mut pack_meta_list = vec![];
      for pack_no in 0..3 {
        let pack_name = format!("pack_name_{}_{}", bucket_id, pack_no);
        let pack_hash = format!("pack_hash_{}_{}", bucket_id, pack_no);
        let pack_size = 100;
        pack_meta_list.push(format!("{},{},{}", pack_name, pack_hash, pack_size));
      }
      writer.line(pack_meta_list.join(" ").as_str()).await?;
    }

    writer.flush().await?;
    Ok(())
  }

  async fn test_read_meta(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    strategy.ensure_meta(scope).await?;
    let meta = scope.meta.expect_value();
    assert_eq!(meta.path, ScopeMeta::get_path(scope.path.as_ref()));
    assert_eq!(meta.buckets, scope.options.buckets);
    assert_eq!(meta.max_pack_size, scope.options.max_pack_size);
    assert_eq!(meta.packs.len(), scope.options.buckets);
    assert_eq!(
      meta
        .packs
        .iter()
        .flatten()
        .map(|i| i.as_ref().to_owned())
        .collect_vec(),
      vec![
        PackFileMeta {
          name: "pack_name_0_0".to_string(),
          hash: "pack_hash_0_0".to_string(),
          size: 100,
          writed: true
        },
        PackFileMeta {
          name: "pack_name_0_1".to_string(),
          hash: "pack_hash_0_1".to_string(),
          size: 100,
          writed: true
        },
        PackFileMeta {
          name: "pack_name_0_2".to_string(),
          hash: "pack_hash_0_2".to_string(),
          size: 100,
          writed: true
        }
      ]
    );

    Ok(())
  }

  async fn test_read_packs(scope: &mut PackScope, strategy: &SplitPackStrategy) -> Result<()> {
    strategy.ensure_keys(scope).await?;

    let all_keys = scope
      .packs
      .expect_value()
      .into_iter()
      .flatten()
      .map(|pack| pack.keys.expect_value().to_owned())
      .flatten()
      .collect::<HashSet<_>>();
    assert!(all_keys.contains(
      &format!("key_{}_{}_{}", scope.options.buckets - 1, 2, 9)
        .as_bytes()
        .to_vec()
    ));

    strategy.ensure_contents(scope).await?;

    let all_contents = scope
      .packs
      .expect_value()
      .into_iter()
      .flatten()
      .map(|pack| pack.contents.expect_value().to_owned())
      .flatten()
      .collect::<HashSet<_>>();
    assert!(all_contents.contains(
      &format!("val_{}_{}_{}", scope.options.buckets - 1, 2, 9)
        .as_bytes()
        .to_vec()
    ));

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
  async fn should_read_scope() {
    let fs = Arc::new(PackMemoryFs::default());
    let strategy =
      SplitPackStrategy::new(PathBuf::from("/cache"), PathBuf::from("/temp"), fs.clone());
    let options = Arc::new(PackOptions {
      buckets: 1,
      max_pack_size: 16,
      expires: 60000,
    });
    let mut scope = PackScope::new(PathBuf::from("/cache/test_read_meta"), options.clone());
    clean_scope_path(&scope, &strategy, fs.clone()).await;

    mock_meta(
      &ScopeMeta::get_path(scope.path.as_ref()),
      fs.clone(),
      &scope.options,
    )
    .await
    .expect("should mock meta");

    mock_packs(&scope.path, fs.clone(), &scope.options)
      .await
      .expect("should mock packs");

    let _ = test_read_meta(&mut scope, &strategy).await.map_err(|e| {
      panic!("{}", e);
    });
    let _ = test_read_packs(&mut scope, &strategy).await.map_err(|e| {
      panic!("{}", e);
    });
  }
}
