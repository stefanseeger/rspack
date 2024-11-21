#[cfg(test)]
pub mod test_pack_utils {
  use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
  };

  use itertools::Itertools;
  use rspack_error::Result;

  use crate::{pack::PackScope, PackFs, PackOptions};

  pub async fn mock_pack_file(
    path: &PathBuf,
    unique_id: &str,
    item_count: usize,
    fs: Arc<dyn PackFs>,
  ) -> Result<()> {
    fs.ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))
      .await?;
    let mut writer = fs.write_file(&path).await?;
    let mut keys = vec![];
    let mut contents = vec![];
    for i in 0..item_count {
      keys.push(format!("key_{}_{}", unique_id, i).as_bytes().to_vec());
      contents.push(format!("val_{}_{}", unique_id, i).as_bytes().to_vec());
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
    Ok(())
  }

  pub async fn mock_meta_file(
    path: &PathBuf,
    fs: Arc<dyn PackFs>,
    options: &PackOptions,
    pack_count: usize,
  ) -> Result<()> {
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
      for pack_no in 0..pack_count {
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

  pub fn count_scope_packs(scope: &PackScope) -> usize {
    scope.packs.expect_value().iter().flatten().count()
  }
  pub fn count_bucket_packs(scope: &PackScope) -> Vec<usize> {
    scope
      .packs
      .expect_value()
      .iter()
      .map(|i| i.len())
      .collect_vec()
  }
  pub fn get_bucket_pack_sizes(scope: &PackScope) -> Vec<usize> {
    let mut res = scope
      .meta
      .expect_value()
      .packs
      .iter()
      .flatten()
      .map(|c| c.size)
      .collect_vec();
    res.sort_unstable();
    res
  }
}
