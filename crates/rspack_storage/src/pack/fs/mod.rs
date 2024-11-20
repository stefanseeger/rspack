use std::path::PathBuf;

use rspack_error::Result;

mod native;
pub use native::*;

mod memory;
pub use memory::*;

mod error;
pub use error::*;

pub struct FileMeta {
  pub size: u64,
  pub mtime: u64,
  pub is_file: bool,
  pub is_dir: bool,
}

#[async_trait::async_trait]
pub trait PackFileReader: std::fmt::Debug + Sync + Send {
  async fn line(&mut self) -> Result<String>;
  async fn bytes(&mut self, len: usize) -> Result<Vec<u8>>;
  async fn skip(&mut self, len: usize) -> Result<()>;
}

#[async_trait::async_trait]
pub trait PackFileWriter: std::fmt::Debug + Sync + Send {
  async fn line(&mut self, line: &str) -> Result<()>;
  async fn bytes(&mut self, bytes: &[u8]) -> Result<()>;
  async fn flush(&mut self) -> Result<()>;
}

#[async_trait::async_trait]
pub trait PackFs: std::fmt::Debug + Sync + Send {
  async fn exists(&self, path: &PathBuf) -> Result<bool>;
  async fn remove_dir(&self, path: &PathBuf) -> Result<()>;
  async fn ensure_dir(&self, path: &PathBuf) -> Result<()>;
  async fn write_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileWriter>>;
  async fn read_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileReader>>;
  async fn metadata(&self, path: &PathBuf) -> Result<FileMeta>;
  async fn remove_file(&self, path: &PathBuf) -> Result<()>;
  async fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()>;
}
