use std::{
  fs::{remove_dir_all, File},
  io::{BufRead, BufReader, BufWriter, Read, Write},
  os::unix::fs::MetadataExt,
  path::PathBuf,
};

use futures::TryFutureExt;
use rspack_error::{error, Result};

#[derive(Debug, Clone)]
pub struct PackStorageFs {
  // TODO: input/output file system
}

impl PackStorageFs {
  pub fn new() -> Self {
    Self {}
  }

  pub fn exists(&self, path: &PathBuf) -> Result<bool> {
    Ok(path.exists())
  }

  pub fn remove_dir(&self, path: &PathBuf) -> Result<()> {
    if path.exists() {
      remove_dir_all(path).map_err(|e| error!("{}", e))
    } else {
      Ok(())
    }
  }

  pub fn ensure_dir(&self, path: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| error!("{}", e))
  }

  pub fn write_file(&self, path: &PathBuf) -> Result<FileWriter> {
    self.ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))?;
    let file = File::create(path).expect("should create file");
    Ok((FileWriter(BufWriter::new(file))))
  }

  pub fn read_file(&self, path: &PathBuf) -> Result<FileReader> {
    let file = File::open(&path).map_err(|e| error!("open pack file failed: {}", e))?;
    Ok(FileReader(BufReader::new(file)))
  }

  pub fn read_file_meta(&self, path: &PathBuf) -> Result<FileMeta> {
    let file = File::open(&path).map_err(|e| error!("open pack file failed: {}", e))?;
    let meta_data = file
      .metadata()
      .map_err(|e| error!("open pack file failed: {}", e))?;
    Ok(FileMeta {
      size: meta_data.size(),
      mtime: meta_data.mtime_nsec(),
    })
  }

  pub async fn remove_file(&self, path: &PathBuf) -> Result<()> {
    if path.exists() {
      tokio::fs::remove_file(&path)
        .await
        .map_err(|e| error!("{}", e))
    } else {
      Ok(())
    }
  }

  pub async fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()> {
    if from.exists() {
      self.ensure_dir(&PathBuf::from(to.parent().expect("should have parent")))?;
      tokio::fs::rename(&from, &to)
        .await
        .map_err(|e| error!("{}", e))
    } else {
      Ok(())
    }
  }
}

pub struct FileWriter(BufWriter<File>);

impl FileWriter {
  pub fn line(&mut self, line: &str) -> Result<()> {
    self
      .0
      .write_fmt(format_args!("{}\n", line))
      .map_err(|e| error!("{}", e))
  }

  pub fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
    self.0.write(bytes).map_err(|e| error!("{}", e))?;
    Ok(())
  }
}

pub struct FileReader(BufReader<File>);

impl FileReader {
  pub fn line(&mut self) -> Result<String> {
    let mut next_line = String::new();
    self
      .0
      .read_line(&mut next_line)
      .map_err(|e| error!("{}", e))?;

    next_line.pop();

    Ok(next_line)
  }

  pub fn bytes(&mut self, len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    self.0.read_exact(&mut bytes).map_err(|e| error!("{}", e))?;
    Ok(bytes)
  }

  pub fn skip(&mut self, len: usize) -> Result<()> {
    self
      .0
      .seek_relative(len as i64)
      .map_err(|e| error!("{}", e))
  }
}

pub struct FileMeta {
  pub size: u64,
  pub mtime: i64,
}
