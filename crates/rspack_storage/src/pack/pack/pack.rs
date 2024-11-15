use std::{hash::Hasher, path::PathBuf, sync::Arc};

use rspack_error::Result;
use rustc_hash::FxHasher;

use crate::pack::{PackFileMeta, PackStorageFs, PackStorageOptions};

pub type PackKeys = Vec<Arc<Vec<u8>>>;

#[derive(Debug, Default, Clone)]
pub enum PackKeysState {
  #[default]
  Pending,
  Value(PackKeys),
}

impl PackKeysState {
  pub fn expect_value(&self) -> &PackKeys {
    match self {
      PackKeysState::Value(v) => v,
      PackKeysState::Pending => panic!("pack key is not ready"),
    }
  }
}

pub type PackContents = Vec<Arc<Vec<u8>>>;

#[derive(Debug, Default, Clone)]
pub enum PackContentsState {
  #[default]
  Pending,
  Value(PackContents),
}

impl PackContentsState {
  pub fn expect_value(&self) -> &PackContents {
    match self {
      PackContentsState::Value(v) => v,
      PackContentsState::Pending => panic!("pack content is not ready"),
    }
  }
}

#[derive(Debug, Clone)]
pub struct Pack {
  pub path: PathBuf,
  pub keys: PackKeysState,
  pub contents: PackContentsState,
}

impl Pack {
  pub fn new(path: PathBuf) -> Self {
    Self {
      path,
      keys: Default::default(),
      contents: Default::default(),
    }
  }

  pub fn loaded(&self) -> bool {
    matches!(self.keys, PackKeysState::Value(_))
      && matches!(self.contents, PackContentsState::Value(_))
  }
}
