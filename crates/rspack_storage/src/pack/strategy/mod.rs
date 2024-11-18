use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;
use rustc_hash::FxHashMap as HashMap;

use crate::{
  pack::{Pack, PackContents, PackFileMeta, PackKeys, ScopeMeta},
  PackStorageOptions,
};

pub struct PackIncrementalResult {
  pub new_packs: Vec<(PackFileMeta, Pack)>,
  pub remain_packs: Vec<(Arc<PackFileMeta>, Pack)>,
  pub removed_files: Vec<PathBuf>,
}

#[async_trait]
pub trait Strategy: std::fmt::Debug + Sync + Send {
  fn get_path(&self, sub: &str) -> PathBuf;
  fn get_temp_path(&self, path: &PathBuf) -> Result<PathBuf>;

  fn get_hash(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<String>;
  fn create(
    &self,
    dir: &PathBuf,
    options: Arc<PackStorageOptions>,
    items: &mut Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>,
  ) -> Vec<(PackFileMeta, Pack)>;
  fn incremental(
    &self,
    dir: PathBuf,
    options: Arc<PackStorageOptions>,
    packs: &mut HashMap<Arc<PackFileMeta>, Pack>,
    updates: &mut HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
  ) -> PackIncrementalResult;
  fn write(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<()>;
  fn read_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>>;
  fn read_contents(&self, path: &PathBuf) -> Result<Option<PackContents>>;

  async fn before_save(&self) -> Result<()>;
  async fn after_save(&self, writed_files: Vec<PathBuf>, removed_files: Vec<PathBuf>)
    -> Result<()>;
  fn write_scope_meta(&self, meta: &ScopeMeta) -> Result<()>;
  fn read_scope_meta(&self, path: &PathBuf) -> Result<Option<ScopeMeta>>;
}

mod base;
pub use base::*;
