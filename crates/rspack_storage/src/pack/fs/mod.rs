use std::path::PathBuf;

use rspack_error::Result;

mod native;
pub use native::*;

mod error;
pub use error::*;

pub struct FileMeta {
  pub size: u64,
  pub mtime: i64,
}

pub trait PackFileReader: std::fmt::Debug + Sync + Send {
  fn line(&mut self) -> Result<String>;
  fn bytes(&mut self, len: usize) -> Result<Vec<u8>>;
  fn skip(&mut self, len: usize) -> Result<()>;
}

pub trait PackFileWriter: std::fmt::Debug + Sync + Send {
  fn line(&mut self, line: &str) -> Result<()>;
  fn bytes(&mut self, bytes: &[u8]) -> Result<()>;
}

pub trait PackFs: std::fmt::Debug + Sync + Send {
  fn exists(&self, path: &PathBuf) -> Result<bool>;
  fn remove_dir(&self, path: &PathBuf) -> Result<()>;
  fn ensure_dir(&self, path: &PathBuf) -> Result<()>;
  fn write_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileWriter>>;
  fn read_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileReader>>;
  fn read_file_meta(&self, path: &PathBuf) -> Result<FileMeta>;
  fn remove_file(&self, path: &PathBuf) -> Result<()>;
  fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()>;
}
