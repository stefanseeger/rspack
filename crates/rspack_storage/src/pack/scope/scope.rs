use std::{path::PathBuf, sync::Arc};

use itertools::Itertools;
use rspack_error::Result;
use rustc_hash::FxHashSet as HashSet;

use crate::pack::PackKeysState;
use crate::pack::{Pack, PackContentsState, PackOptions, ScopeMeta};

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
  pub options: Arc<PackOptions>,
  pub meta: ScopeMetaState,
  pub packs: ScopePacksState,
  pub removed: HashSet<PathBuf>,
}

impl PackScope {
  pub fn new(path: PathBuf, options: Arc<PackOptions>) -> Self {
    Self {
      path: Arc::new(path),
      options,
      meta: ScopeMetaState::Pending,
      packs: ScopePacksState::Pending,
      removed: HashSet::default(),
    }
  }

  pub fn empty(path: PathBuf, options: Arc<PackOptions>) -> Self {
    let meta = ScopeMeta::new(&path, &options);
    let packs = vec![vec![]; options.buckets];

    Self {
      path: Arc::new(path),
      options,
      meta: ScopeMetaState::Value(meta),
      packs: ScopePacksState::Value(packs),
      removed: HashSet::default(),
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

  pub async fn get_contents(&self) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>> {
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
}
