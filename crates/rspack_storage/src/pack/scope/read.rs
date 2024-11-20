use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use rspack_error::{error, Result};

use super::{PackFileMeta, PackScope, ScopeMeta, ScopePacks};
use crate::pack::{Pack, PackContents, PackContentsState, PackKeys, PackKeysState, Strategy};

#[derive(Debug)]
pub struct ReadKeysResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub keys: PackKeys,
}

pub async fn read_keys(scope: &PackScope) -> Result<Vec<ReadKeysResult>> {
  let candidates = get_pack_meta_pairs(scope)?
    .into_iter()
    .filter(|(_, _, _, pack)| matches!(pack.keys, PackKeysState::Pending))
    .map(|args| (args.0, args.1, args.3.path.to_owned()))
    .collect_vec();
  let readed = batch_read_keys(
    candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
    scope.strategy.clone(),
  )
  .await?;

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
    tokio::spawn(async move { strategy.read_keys(&path).await }).map_err(|e| error!("{}", e))
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

#[derive(Debug)]
pub struct ReadContentsResult {
  pub bucket_id: usize,
  pub pack_pos: usize,
  pub contents: PackContents,
}

pub async fn read_contents(scope: &PackScope) -> Result<Vec<ReadContentsResult>> {
  let candidates = get_pack_meta_pairs(scope)?
    .into_iter()
    .filter(|(_, _, _, pack)| matches!(pack.contents, PackContentsState::Pending))
    .map(|args| (args.0, args.1, args.3.path.to_owned()))
    .collect_vec();

  let readed: Vec<Vec<Arc<Vec<u8>>>> = batch_read_contents(
    candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
    scope.strategy.clone(),
  )
  .await?;

  Ok(
    readed
      .into_iter()
      .zip(candidates.into_iter())
      .map(|(contents, (bucket_id, pack_pos, _))| ReadContentsResult {
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
    tokio::spawn(async move { strategy.read_contents(&path).await }).map_err(|e| error!("{}", e))
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

pub async fn read_meta(scope: &PackScope) -> Result<ScopeMeta> {
  let scope_path = ScopeMeta::get_path(&scope.path);
  let meta = scope.strategy.read_scope_meta(&scope_path).await?;
  if let Some(meta) = meta {
    Ok(meta)
  } else {
    Ok(ScopeMeta::new(&scope.path, &scope.options))
  }
}

pub async fn read_packs(scope: &PackScope) -> Result<ScopePacks> {
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

  use super::{read_keys, read_meta, read_packs};
  use crate::{
    pack::{
      read_contents, PackFileMeta, PackFs, PackMemoryFs, PackScope, ScopeMeta, ScopeMetaState,
      ScopePacksState, SplitPackStrategy, Strategy,
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

  async fn test_read_meta(scope: &mut PackScope) -> Result<()> {
    let meta = read_meta(scope).await?;
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

    scope.meta = ScopeMetaState::Value(meta);

    Ok(())
  }

  async fn test_read_packs(scope: &mut PackScope) -> Result<()> {
    let packs = read_packs(scope).await?;
    assert_eq!(packs.len(), scope.options.buckets);
    scope.packs = ScopePacksState::Value(packs);

    let keys = read_keys(scope).await?;
    assert_eq!(
      keys.len(),
      scope.packs.expect_value().iter().flatten().count()
    );

    let all_keys = keys
      .into_iter()
      .map(|item| item.keys)
      .flatten()
      .collect::<HashSet<_>>();
    assert!(all_keys.contains(
      &format!("key_{}_{}_{}", scope.options.buckets - 1, 2, 9)
        .as_bytes()
        .to_vec()
    ));

    let contents = read_contents(scope).await?;
    assert_eq!(
      contents.len(),
      scope.packs.expect_value().iter().flatten().count()
    );

    let all_contents = contents
      .into_iter()
      .map(|item| item.contents)
      .flatten()
      .collect::<HashSet<_>>();
    assert!(all_contents.contains(
      &format!("val_{}_{}_{}", scope.options.buckets - 1, 2, 9)
        .as_bytes()
        .to_vec()
    ));

    Ok(())
  }

  async fn clean_scope_path(scope: &PackScope, strategy: Arc<dyn Strategy>, fs: Arc<dyn PackFs>) {
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
    let strategy = Arc::new(SplitPackStrategy::new(
      PathBuf::from("/cache"),
      PathBuf::from("/temp"),
      fs.clone(),
    ));
    let options = Arc::new(PackOptions {
      buckets: 1,
      max_pack_size: 16,
      expires: 60000,
    });
    let mut scope = PackScope::new("test_read_meta", options.clone(), strategy.clone());
    clean_scope_path(&scope, strategy.clone(), fs.clone()).await;

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

    let _ = test_read_meta(&mut scope).await.map_err(|e| {
      panic!("{}", e);
    });
    let _ = test_read_packs(&mut scope).await.map_err(|e| {
      panic!("{}", e);
    });
  }
}
