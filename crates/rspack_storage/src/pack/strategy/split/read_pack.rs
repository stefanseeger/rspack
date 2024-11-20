use std::{hash::Hasher, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;
use rustc_hash::FxHasher;

use super::{util::get_name, SplitPackStrategy};
use crate::pack::{PackContents, PackKeys, PackReadStrategy};

#[async_trait]
impl PackReadStrategy for SplitPackStrategy {
  async fn get_pack_hash(
    &self,
    path: &PathBuf,
    keys: &PackKeys,
    contents: &PackContents,
  ) -> Result<String> {
    let mut hasher = FxHasher::default();
    let file_name = get_name(keys, contents);
    hasher.write(file_name.as_bytes());

    let meta = self.fs.metadata(path).await?;
    hasher.write_u64(meta.size);
    hasher.write_u64(meta.mtime);

    Ok(format!("{:016x}", hasher.finish()))
  }

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
