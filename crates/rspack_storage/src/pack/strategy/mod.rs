use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::{
  pack::{Pack, PackContents, PackFileMeta, PackKeys},
  PackOptions,
};

pub struct UpdatePacksResult {
  pub new_packs: Vec<(PackFileMeta, Pack)>,
  pub remain_packs: Vec<(Arc<PackFileMeta>, Pack)>,
  pub removed_files: Vec<PathBuf>,
}

#[async_trait]
pub trait Strategy: PackStrategy + ScopeStrategy + std::fmt::Debug + Sync + Send {}

#[async_trait]
pub trait ScopeStrategy: ScopeReadStrategy + ScopeWriteStrategy {}

#[async_trait]
pub trait PackStrategy: PackReadStrategy + PackWriteStrategy + ScopeValidateStrategy {}

#[async_trait]
pub trait PackReadStrategy {
  async fn read_pack_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>>;
  async fn read_pack_contents(&self, path: &PathBuf) -> Result<Option<PackContents>>;
}

#[async_trait]
pub trait PackWriteStrategy {
  async fn update_packs(
    &self,
    dir: PathBuf,
    options: &PackOptions,
    packs: HashMap<Arc<PackFileMeta>, Pack>,
    updates: HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
  ) -> UpdatePacksResult;
  async fn write_pack(&self, pack: &Pack) -> Result<()>;
}

// #[async_trait]
// pub trait ScopeStrategy {
// fn get_path(&self, sub: &str) -> PathBuf;
//   // fn get_temp_path(&self, path: &PathBuf) -> Result<PathBuf>;

// async fn before_save(&self) -> Result<()>;
// async fn after_save(&self, writed_files: Vec<PathBuf>, removed_files: Vec<PathBuf>)
//     -> Result<()>;
//   async fn write_scope_meta(&self, meta: &ScopeMeta) -> Result<()>;
//   async fn read_scope_meta(&self, path: &PathBuf) -> Result<Option<ScopeMeta>>;
// }

#[async_trait]
pub trait ScopeReadStrategy {
  fn get_path(&self, sub: &str) -> PathBuf;
  async fn ensure_meta(&self, scope: &mut PackScope) -> Result<()>;
  async fn ensure_packs(&self, scope: &mut PackScope) -> Result<()>;
  async fn ensure_keys(&self, scope: &mut PackScope) -> Result<()>;
  async fn ensure_contents(&self, scope: &mut PackScope) -> Result<()>;
}

#[derive(Debug)]
pub enum ValidateResult {
  Valid,
  Invalid(String),
}

#[async_trait]
pub trait ScopeValidateStrategy {
  async fn validate_meta(&self, scope: &mut PackScope) -> Result<ValidateResult>;
  async fn validate_packs(&self, scope: &mut PackScope) -> Result<ValidateResult>;
}

#[derive(Debug, Default)]
pub struct WriteScopeResult {
  pub writed_files: HashSet<PathBuf>,
  pub removed_files: HashSet<PathBuf>,
}

impl WriteScopeResult {
  pub fn extend(&mut self, other: Self) {
    self.writed_files.extend(other.writed_files);
    self.removed_files.extend(other.removed_files);
  }
}

#[async_trait]
pub trait ScopeWriteStrategy {
  async fn before_save(&self) -> Result<()>;
  async fn after_save(&self, writed_files: Vec<PathBuf>, removed_files: Vec<PathBuf>)
    -> Result<()>;
  async fn update_scope(
    &self,
    scope: &mut PackScope,
    updates: HashMap<Vec<u8>, Option<Vec<u8>>>,
  ) -> Result<()>;
  async fn write_scope(&self, scope: &mut PackScope) -> Result<WriteScopeResult>;
  async fn write_packs(&self, scope: &mut PackScope) -> Result<WriteScopeResult>;
  async fn write_meta(&self, scope: &mut PackScope) -> Result<WriteScopeResult>;
}

mod split;
pub use split::*;

use super::PackScope;
