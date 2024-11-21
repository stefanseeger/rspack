use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;

use super::SplitPackStrategy;
use crate::pack::{PackContents, PackKeys, PackReadStrategy};

#[async_trait]
impl PackReadStrategy for SplitPackStrategy {
  async fn read_pack_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>> {
    if !self.fs.exists(path).await? {
      return Ok(None);
    }

    let mut reader = self.fs.read_file(path).await?;
    let key_meta_list: Vec<usize> = reader
      .line()
      .await?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .collect();

    reader.line().await?;

    let mut keys = vec![];
    for len in key_meta_list {
      keys.push(Arc::new(reader.bytes(len).await?));
    }
    Ok(Some(keys))
  }

  async fn read_pack_contents(&self, path: &PathBuf) -> Result<Option<PackContents>> {
    if !self.fs.exists(path).await? {
      return Ok(None);
    }

    let mut reader = self.fs.read_file(path).await?;
    let total_key_size = reader
      .line()
      .await?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .fold(0_usize, |acc, key| acc + key);

    let content_meta_list: Vec<usize> = reader
      .line()
      .await?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .collect();

    reader.skip(total_key_size).await?;

    let mut res = vec![];
    for len in content_meta_list {
      res.push(Arc::new(reader.bytes(len).await?));
    }

    Ok(Some(res))
  }
}

#[cfg(test)]
mod tests {

  use std::{path::PathBuf, sync::Arc};

  use rspack_error::Result;
  use rustc_hash::FxHashSet as HashSet;

  use crate::{
    pack::{
      strategy::split::test::test_pack_utils::mock_pack_file, PackReadStrategy, SplitPackStrategy,
    },
    PackFs, PackMemoryFs,
  };

  async fn test_read_keys_non_exists(strategy: &SplitPackStrategy) -> Result<()> {
    let non_exists_keys = strategy
      .read_pack_keys(&PathBuf::from("/non_exists_path"))
      .await?;
    assert!(non_exists_keys.is_none());
    Ok(())
  }

  async fn test_read_contents_non_exists(strategy: &SplitPackStrategy) -> Result<()> {
    let non_exists_contents = strategy
      .read_pack_contents(&PathBuf::from("/non_exists_path"))
      .await?;
    assert!(non_exists_contents.is_none());
    Ok(())
  }

  async fn test_read_keys(path: &PathBuf, strategy: &SplitPackStrategy) -> Result<()> {
    let keys = strategy
      .read_pack_keys(path)
      .await?
      .unwrap_or_default()
      .into_iter()
      .collect::<HashSet<_>>();
    assert!(keys.contains(&"key_mock_0".as_bytes().to_vec()));
    assert!(keys.contains(&"key_mock_19".as_bytes().to_vec()));
    Ok(())
  }

  async fn test_read_contents(path: &PathBuf, strategy: &SplitPackStrategy) -> Result<()> {
    let contents = strategy
      .read_pack_contents(path)
      .await?
      .unwrap_or_default()
      .into_iter()
      .collect::<HashSet<_>>();
    assert!(contents.contains(&"val_mock_0".as_bytes().to_vec()));
    assert!(contents.contains(&"val_mock_19".as_bytes().to_vec()));
    Ok(())
  }

  #[tokio::test]
  async fn should_read_pack() {
    let dir = PathBuf::from("/cache/test_read_pack");
    let fs = Arc::new(PackMemoryFs::default());
    fs.remove_dir(&dir).await.expect("should clean dir");
    let strategy = SplitPackStrategy::new(dir.clone(), PathBuf::from("/temp"), fs.clone());
    mock_pack_file(&dir.join("./mock_pack"), "mock", 20, fs)
      .await
      .expect("should mock pack file");
    let _ = test_read_keys(&dir.join("./mock_pack"), &strategy)
      .await
      .map_err(|e| panic!("{:?}", e));
    let _ = test_read_contents(&dir.join("./mock_pack"), &strategy)
      .await
      .map_err(|e| panic!("{:?}", e));
    let _ = test_read_keys_non_exists(&strategy)
      .await
      .map_err(|e| panic!("{:?}", e));
    let _ = test_read_contents_non_exists(&strategy)
      .await
      .map_err(|e| panic!("{:?}", e));
  }
}
