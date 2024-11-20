use std::path::PathBuf;

use rspack_error::{
  miette::{self},
  thiserror::{self, Error},
};

#[derive(Debug)]
pub enum PackFsErrorOpt {
  Read,
  Write,
  Dir,
  Remove,
  Stat,
  Move,
}

impl std::fmt::Display for PackFsErrorOpt {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Read => write!(f, "read"),
      Self::Write => write!(f, "write"),
      Self::Dir => write!(f, "create dir"),
      Self::Remove => write!(f, "remove"),
      Self::Stat => write!(f, "stat"),
      Self::Move => write!(f, "move"),
    }
  }
}

#[derive(Debug, Error)]
#[error(r#"Rspack Storage FS Error: {opt} `{file}` failed with `{inner}`"#)]
pub struct PackFsError {
  file: String,
  inner: String,
  opt: PackFsErrorOpt,
}

impl PackFsError {
  pub fn from_io_error(file: &PathBuf, opt: PackFsErrorOpt, error: std::io::Error) -> Self {
    Self {
      file: file.to_string_lossy().to_string(),
      inner: error.to_string(),
      opt,
    }
  }
  pub fn from_fs_error(file: &PathBuf, opt: PackFsErrorOpt, error: rspack_fs::Error) -> Self {
    Self {
      file: file.to_string_lossy().to_string(),
      inner: error.to_string(),
      opt,
    }
  }
}

impl miette::Diagnostic for PackFsError {
  fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
    Some(Box::new("PackFsError"))
  }
}
