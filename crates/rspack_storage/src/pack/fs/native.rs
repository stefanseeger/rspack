use std::{
  fs::{remove_dir_all, File},
  io::{BufRead, BufReader, BufWriter, Read, Write},
  os::unix::fs::MetadataExt,
  path::PathBuf,
};

use rspack_error::Result;

use super::{FileMeta, PackFileReader, PackFileWriter, PackFs, PackFsError, PackFsErrorOpt};

#[derive(Debug, Default)]
pub struct PackNativeFs;

#[async_trait::async_trait]
impl PackFs for PackNativeFs {
  async fn exists(&self, path: &PathBuf) -> Result<bool> {
    Ok(path.exists())
  }

  async fn remove_dir(&self, path: &PathBuf) -> Result<()> {
    if path.exists() {
      remove_dir_all(path).map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Remove, e).into())
    } else {
      Ok(())
    }
  }

  async fn ensure_dir(&self, path: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(path)
      .map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Dir, e).into())
  }

  async fn write_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileWriter>> {
    self
      .ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))
      .await?;
    let file = File::create(path).map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Write, e))?;
    Ok(Box::new(NativeFileWriter::new(
      path.clone(),
      BufWriter::new(file),
    )))
  }

  async fn read_file(&self, path: &PathBuf) -> Result<Box<dyn PackFileReader>> {
    let file = File::open(&path).map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Read, e))?;
    Ok(Box::new(NativeFileReader::new(
      path.clone(),
      BufReader::new(file),
    )))
  }

  async fn read_file_meta(&self, path: &PathBuf) -> Result<FileMeta> {
    let file = File::open(&path).map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Read, e))?;
    let meta_data = file
      .metadata()
      .map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Stat, e))?;
    Ok(FileMeta {
      size: meta_data.size(),
      mtime: meta_data.mtime_nsec() as u64,
    })
  }

  async fn remove_file(&self, path: &PathBuf) -> Result<()> {
    if path.exists() {
      std::fs::remove_file(&path)
        .map_err(|e| PackFsError::new(&path, PackFsErrorOpt::Remove, e).into())
    } else {
      Ok(())
    }
  }

  async fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()> {
    if from.exists() {
      self
        .ensure_dir(&PathBuf::from(to.parent().expect("should have parent")))
        .await?;
      std::fs::rename(&from, &to)
        .map_err(|e| PackFsError::new(&from, PackFsErrorOpt::Move, e).into())
    } else {
      Ok(())
    }
  }
}

#[derive(Debug)]
pub struct NativeFileWriter {
  path: PathBuf,
  inner: BufWriter<File>,
}

impl NativeFileWriter {
  pub fn new(path: PathBuf, inner: BufWriter<File>) -> Self {
    Self { path, inner }
  }
}

#[async_trait::async_trait]
impl PackFileWriter for NativeFileWriter {
  async fn line(&mut self, line: &str) -> Result<()> {
    self
      .inner
      .write_fmt(format_args!("{}\n", line))
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Write, e).into())
  }

  async fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
    self
      .inner
      .write(bytes)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Write, e))?;
    Ok(())
  }

  async fn flush(&mut self) -> Result<()> {
    Ok(())
  }
}

#[derive(Debug)]
pub struct NativeFileReader {
  path: PathBuf,
  inner: BufReader<File>,
}

impl NativeFileReader {
  pub fn new(path: PathBuf, inner: BufReader<File>) -> Self {
    Self { path, inner }
  }
}

#[async_trait::async_trait]
impl PackFileReader for NativeFileReader {
  async fn line(&mut self) -> Result<String> {
    let mut next_line = String::new();
    self
      .inner
      .read_line(&mut next_line)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e))?;

    next_line.pop();

    Ok(next_line)
  }

  async fn bytes(&mut self, len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    self
      .inner
      .read_exact(&mut bytes)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e))?;
    Ok(bytes)
  }

  async fn skip(&mut self, len: usize) -> Result<()> {
    self
      .inner
      .seek_relative(len as i64)
      .map_err(|e| PackFsError::new(&self.path, PackFsErrorOpt::Read, e).into())
  }
}
