use std::{
  path::PathBuf,
  sync::Arc,
  time::{SystemTime, UNIX_EPOCH},
};

use crate::PackOptions;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackFileMeta {
  pub hash: String,
  pub name: String,
  pub size: usize,
  pub writed: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ScopeMeta {
  pub path: PathBuf,
  pub buckets: usize,
  pub max_pack_size: usize,
  pub last_modified: u64,
  pub packs: Vec<Vec<Arc<PackFileMeta>>>,
}
impl ScopeMeta {
  pub fn new(dir: &PathBuf, options: &PackOptions) -> Self {
    Self {
      path: Self::get_path(dir),
      buckets: options.buckets.clone(),
      max_pack_size: options.max_pack_size.clone(),
      last_modified: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("should get current time")
        .as_secs(),
      packs: vec![vec![]; options.buckets],
    }
  }

  pub fn get_path(dir: &PathBuf) -> PathBuf {
    dir.join("cache_meta")
  }
}
