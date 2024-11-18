use std::{path::PathBuf, sync::Arc};

use itertools::Itertools;
use rspack_error::{error, Result};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::{
  read_contents, read_keys, read_meta, read_packs, save_scope, update_scope, validate_meta,
  validate_packs, ScopeSaveResult, ValidateResult,
};
use crate::pack::{Pack, PackContentsState, PackStorageOptions, ScopeMeta};
use crate::pack::{PackKeysState, Strategy};

#[derive(Debug, Default, Clone)]
pub enum ScopeMetaState {
  #[default]
  Pending,
  Value(ScopeMeta),
}

impl ScopeMetaState {
  pub fn expect_value(&self) -> &ScopeMeta {
    match self {
      ScopeMetaState::Value(v) => v,
      ScopeMetaState::Pending => panic!("should have scope meta"),
    }
  }
  pub fn expect_value_mut(&mut self) -> &mut ScopeMeta {
    match self {
      ScopeMetaState::Value(ref mut v) => v,
      ScopeMetaState::Pending => panic!("should have scope meta"),
    }
  }
  pub fn take_value(&mut self) -> Option<ScopeMeta> {
    match self {
      ScopeMetaState::Value(v) => Some(std::mem::take(&mut *v)),
      _ => None,
    }
  }
}

pub type ScopePacks = Vec<Vec<Pack>>;

#[derive(Debug, Default, Clone)]
pub enum ScopePacksState {
  #[default]
  Pending,
  Value(ScopePacks),
}

impl ScopePacksState {
  pub fn expect_value(&self) -> &ScopePacks {
    match self {
      ScopePacksState::Value(v) => v,
      ScopePacksState::Pending => panic!("scope meta is not ready"),
    }
  }
  pub fn expect_value_mut(&mut self) -> &mut ScopePacks {
    match self {
      ScopePacksState::Value(v) => v,
      ScopePacksState::Pending => panic!("scope meta is not ready"),
    }
  }
  pub fn take_value(&mut self) -> Option<ScopePacks> {
    match self {
      ScopePacksState::Value(v) => Some(std::mem::take(&mut *v)),
      _ => None,
    }
  }
}

#[derive(Debug, Clone)]
pub struct PackScope {
  pub path: Arc<PathBuf>,
  pub options: Arc<PackStorageOptions>,
  pub meta: ScopeMetaState,
  pub packs: ScopePacksState,
  pub removed: HashSet<PathBuf>,
  pub strategy: Arc<dyn Strategy>,
}

impl PackScope {
  pub fn new(
    name: &'static str,
    options: Arc<PackStorageOptions>,
    strategy: Arc<dyn Strategy>,
  ) -> Self {
    Self {
      path: Arc::new(strategy.get_path(name)),
      options,
      meta: ScopeMetaState::Pending,
      packs: ScopePacksState::Pending,
      removed: HashSet::default(),
      strategy,
    }
  }

  pub fn empty(
    name: &'static str,
    options: Arc<PackStorageOptions>,
    strategy: Arc<dyn Strategy>,
  ) -> Self {
    let scope_path = strategy.get_path(name);
    let meta = ScopeMeta::new(&scope_path, options.clone());
    let packs = vec![vec![]; options.buckets];

    Self {
      path: Arc::new(scope_path),
      options,
      meta: ScopeMetaState::Value(meta),
      packs: ScopePacksState::Value(packs),
      removed: HashSet::default(),
      strategy,
    }
  }

  pub fn loaded(&self) -> bool {
    matches!(self.meta, ScopeMetaState::Value(_))
      && matches!(self.packs, ScopePacksState::Value(_))
      && self
        .packs
        .expect_value()
        .iter()
        .flatten()
        .all(|pack| pack.loaded())
  }

  pub fn get_contents(&mut self) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>> {
    self.ensure_pack_contents()?;

    Ok(
      self
        .packs
        .expect_value()
        .iter()
        .flatten()
        .filter_map(|pack| {
          if let (PackKeysState::Value(keys), PackContentsState::Value(contents)) =
            (&pack.keys, &pack.contents)
          {
            if keys.len() == contents.len() {
              return Some(
                keys
                  .iter()
                  .enumerate()
                  .map(|(index, key)| (key.clone(), contents[index].clone()))
                  .collect_vec(),
              );
            }
          }
          None
        })
        .flatten()
        .collect_vec(),
    )
  }

  pub fn validate(&mut self, options: &PackStorageOptions) -> Result<ValidateResult> {
    self.ensure_meta()?;

    let is_meta_valid = validate_meta(&self, &options)?;

    if matches!(is_meta_valid, ValidateResult::Valid) {
      self.ensure_pack_keys()?;
      validate_packs(&self)
    } else {
      Ok(is_meta_valid)
    }
  }

  fn ensure_meta(&mut self) -> Result<()> {
    if matches!(self.meta, ScopeMetaState::Pending) {
      self.meta = ScopeMetaState::Value(read_meta(&self)?);
    }
    Ok(())
  }

  fn ensure_packs(&mut self) -> Result<()> {
    self.ensure_meta()?;
    if matches!(self.packs, ScopePacksState::Pending) {
      self.packs = ScopePacksState::Value(read_packs(&self)?);
    }
    Ok(())
  }

  fn ensure_pack_keys(&mut self) -> Result<()> {
    self.ensure_packs()?;

    let packs_results = read_keys(&self)?;
    let packs = self.packs.expect_value_mut();
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

  fn ensure_pack_contents(&mut self) -> Result<()> {
    self.ensure_pack_keys()?;

    let packs_results = read_contents(&self)?;
    let packs = self.packs.expect_value_mut();
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

  pub fn update(&mut self, updates: HashMap<Vec<u8>, Option<Vec<u8>>>) -> Result<()> {
    if !self.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
    update_scope(self, updates)
  }

  pub async fn save(&mut self) -> Result<ScopeSaveResult> {
    if !self.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
    save_scope(self).await
  }
}
