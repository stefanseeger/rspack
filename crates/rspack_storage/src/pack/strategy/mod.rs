use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;
use rustc_hash::FxHashMap as HashMap;

use crate::{
  pack::{Pack, PackContents, PackFileMeta, PackKeys, ScopeMeta},
  PackOptions,
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

  async fn get_hash(
    &self,
    path: &PathBuf,
    keys: &PackKeys,
    contents: &PackContents,
  ) -> Result<String>;
  async fn create(
    &self,
    dir: &PathBuf,
    options: Arc<PackOptions>,
    items: &mut Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>,
  ) -> Vec<(PackFileMeta, Pack)>;
  async fn incremental(
    &self,
    dir: PathBuf,
    options: Arc<PackOptions>,
    packs: HashMap<Arc<PackFileMeta>, Pack>,
    updates: HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
  ) -> PackIncrementalResult;
  async fn write(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<()>;
  async fn read_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>>;
  async fn read_contents(&self, path: &PathBuf) -> Result<Option<PackContents>>;

  async fn before_save(&self) -> Result<()>;
  async fn after_save(&self, writed_files: Vec<PathBuf>, removed_files: Vec<PathBuf>)
    -> Result<()>;
  async fn write_scope_meta(&self, meta: &ScopeMeta) -> Result<()>;
  async fn read_scope_meta(&self, path: &PathBuf) -> Result<Option<ScopeMeta>>;
}

mod base;
pub use base::*;
