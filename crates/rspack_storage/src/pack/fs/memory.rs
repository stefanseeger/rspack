use std::{
  io::{BufRead, Cursor, Read, Seek},
  path::PathBuf,
  sync::Arc,
};

use rspack_error::Result;
use rspack_fs::{AsyncWritableFileSystem, MemoryFileSystem, ReadableFileSystem};
use rspack_paths::{AssertUtf8, Utf8PathBuf};

use super::{FileMeta, PackFileReader, PackFileWriter, PackFs, PackFsError, PackFsErrorOpt};

#[derive(Debug, Default)]
pub struct PackMemoryFs(Arc<MemoryFileSystem>);

#[async_trait::async_trait]
impl PackFs for PackMemoryFs {
  async fn exists(&self, path: &PathBuf) -> Result<bool> {
    let path = path.to_owned().assert_utf8();
    match self.0.metadata(&path) {
      Ok(_) => Ok(true),
      Err(e) => {
        if e.to_string().contains("file not exist") {
          Ok(false)
        } else {
          Err(e.into())
        }
      }
    }
  }

  async fn remove_dir(&self, path: &PathBuf) -> Result<()> {
    let path = path.to_owned().assert_utf8();
    self.0.remove_dir_all(&path).await?;
    Ok(())
  }

  async fn ensure_dir(&self, path: &PathBuf) -> Result<()> {
    let path = path.to_owned().assert_utf8();
    self.0.create_dir_all(&path).await?;
    Ok(())
  }

  async fn write_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileWriter>> {
    Ok(Box::new(MemoryFileWriter::new(
      path.to_owned().assert_utf8(),
      self.0.clone(),
    )))
  }

  async fn read_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileReader>> {
    Ok(Box::new(MemoryFileReader::new(
      path.to_owned(),
      self.0.clone(),
    )))
  }

  async fn read_file_meta(&self, path: &PathBuf) -> Result<FileMeta> {
    let path = path.to_owned().assert_utf8();
    let meta_data = self.0.metadata(&path)?;
    Ok(FileMeta {
      size: meta_data.size,
      mtime: meta_data.mtime_ms,
    })
  }

  async fn remove_file(&self, path: &PathBuf) -> Result<()> {
    let path = path.to_owned().assert_utf8();
    self.0.remove_file(&path).await?;
    Ok(())
  }

  async fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()> {
    // TODO: output file system rename method
    let from_path = from.to_owned().assert_utf8();
    let to_path = to.to_owned().assert_utf8();
    let content = self.0.read_file(&from_path).await?;
    self.0.write(&to_path, &content).await?;
    Ok(())
  }
}

#[derive(Debug)]
pub struct MemoryFileWriter {
  path: Utf8PathBuf,
  contents: Vec<u8>,
  fs: Arc<MemoryFileSystem>,
}

impl MemoryFileWriter {
  pub fn new(path: Utf8PathBuf, fs: Arc<MemoryFileSystem>) -> Self {
    Self {
      path,
      contents: vec![],
      fs,
    }
  }
}

#[async_trait::async_trait]
impl PackFileWriter for MemoryFileWriter {
  async fn line(&mut self, line: &str) -> Result<()> {
    let line = format!("{}\n", line);
    self.contents.extend(line.as_bytes().to_vec());
    Ok(())
  }

  async fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
    self.contents.extend(bytes.to_vec());
    Ok(())
  }

  async fn flush(&mut self) -> Result<()> {
    self.fs.write(&self.path, &self.contents).await?;
    Ok(())
  }
}

#[derive(Debug)]
pub struct MemoryFileReader {
  path: PathBuf,
  reader: Option<Cursor<Vec<u8>>>,
  fs: Arc<MemoryFileSystem>,
}

impl MemoryFileReader {
  pub fn new(path: PathBuf, fs: Arc<MemoryFileSystem>) -> Self {
    Self {
      path,
      reader: None,
      fs,
    }
  }
}

impl MemoryFileReader {
  async fn ensure_contents(&mut self) -> Result<()> {
    if self.reader.is_none() {
      let path = self.path.to_owned().assert_utf8();
      let contents = self.fs.read_file(&path).await?;
      self.reader = Some(Cursor::new(contents));
    }
    Ok(())
  }
}

#[async_trait::async_trait]
impl PackFileReader for MemoryFileReader {
  async fn line(&mut self) -> Result<String> {
    self.ensure_contents().await?;

    let reader = self.reader.as_mut().expect("should have reader");
    let mut next_line = String::new();

    reader
      .read_line(&mut next_line)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e))?;

    next_line.pop();

    Ok(next_line)
  }

  async fn bytes(&mut self, len: usize) -> Result<Vec<u8>> {
    self.ensure_contents().await?;

    let mut bytes = vec![0u8; len];
    let reader = self.reader.as_mut().expect("should have reader");

    reader
      .read_exact(&mut bytes)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e))?;

    Ok(bytes)
  }

  async fn skip(&mut self, len: usize) -> Result<()> {
    self.ensure_contents().await?;

    let reader = self.reader.as_mut().expect("should have reader");

    reader
      .seek_relative(len as i64)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e).into())
  }
}

#[cfg(test)]
mod tests {
  use std::{path::PathBuf, sync::Arc};

  use rspack_error::Result;
  use rspack_fs::MemoryFileSystem;

  use super::PackMemoryFs;
  use crate::pack::PackFs;

  async fn test_ensure_dir(fs: &PackMemoryFs) -> Result<()> {
    fs.ensure_dir(&PathBuf::from("/parent/from")).await?;
    fs.ensure_dir(&PathBuf::from("/parent/to")).await?;

    assert!(fs.exists(&PathBuf::from("/parent/from")).await?);
    assert!(fs.exists(&PathBuf::from("/parent/to")).await?);

    Ok(())
  }

  #[tokio::test]
  async fn should_pack_memory_fs_work() {
    let fs = PackMemoryFs(Arc::new(MemoryFileSystem::default()));

    let _ = test_ensure_dir(&fs).await;
  }
}
