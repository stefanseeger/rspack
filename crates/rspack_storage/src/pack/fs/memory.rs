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
    self
      .0
      .remove_dir_all(&path.to_owned().assert_utf8())
      .await
      .map_err(|e| PackFsError::from_fs_error(&path, PackFsErrorOpt::Remove, e))?;
    Ok(())
  }

  async fn ensure_dir(&self, path: &PathBuf) -> Result<()> {
    self
      .0
      .create_dir_all(&path.to_owned().assert_utf8())
      .await
      .map_err(|e| PackFsError::from_fs_error(&path, PackFsErrorOpt::Dir, e))?;
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

  async fn metadata(&self, path: &PathBuf) -> Result<FileMeta> {
    let meta_data = self
      .0
      .metadata(&path.to_owned().assert_utf8())
      .map_err(|e| PackFsError::from_fs_error(&path, PackFsErrorOpt::Stat, e))?;
    Ok(FileMeta {
      size: meta_data.size,
      mtime: meta_data.mtime_ms,
      is_file: meta_data.is_file,
      is_dir: meta_data.is_directory,
    })
  }

  async fn remove_file(&self, path: &PathBuf) -> Result<()> {
    self
      .0
      .remove_file(&path.to_owned().assert_utf8())
      .await
      .map_err(|e| PackFsError::from_fs_error(&path, PackFsErrorOpt::Remove, e))?;
    Ok(())
  }

  async fn move_file(&self, from: &PathBuf, to: &PathBuf) -> Result<()> {
    // TODO: output file system rename method
    let from_path = from.to_owned().assert_utf8();
    let to_path = to.to_owned().assert_utf8();
    let content = self
      .0
      .read_file(&from_path)
      .await
      .map_err(|e| PackFsError::from_fs_error(from, PackFsErrorOpt::Move, e))?;
    self
      .0
      .write(&to_path, &content)
      .await
      .map_err(|e| PackFsError::from_fs_error(from, PackFsErrorOpt::Move, e))?;
    self
      .0
      .remove_file(&from_path)
      .await
      .map_err(|e| PackFsError::from_fs_error(from, PackFsErrorOpt::Move, e))?;
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
      .map_err(|e| PackFsError::from_io_error(&self.path, PackFsErrorOpt::Read, e))?;

    next_line.pop();

    Ok(next_line)
  }

  async fn bytes(&mut self, len: usize) -> Result<Vec<u8>> {
    self.ensure_contents().await?;

    let mut bytes = vec![0u8; len];
    let reader = self.reader.as_mut().expect("should have reader");

    reader
      .read_exact(&mut bytes)
      .map_err(|e| PackFsError::from_io_error(&self.path, PackFsErrorOpt::Read, e))?;

    Ok(bytes)
  }

  async fn skip(&mut self, len: usize) -> Result<()> {
    self.ensure_contents().await?;

    let reader = self.reader.as_mut().expect("should have reader");

    reader
      .seek_relative(len as i64)
      .map_err(|e| PackFsError::from_io_error(&self.path, PackFsErrorOpt::Read, e).into())
  }
}

#[cfg(test)]
mod tests {
  use std::{path::PathBuf, sync::Arc};

  use rspack_error::Result;
  use rspack_fs::MemoryFileSystem;

  use super::PackMemoryFs;
  use crate::pack::PackFs;

  async fn test_create_dir(fs: &PackMemoryFs) -> Result<()> {
    fs.ensure_dir(&PathBuf::from("/parent/from")).await?;
    fs.ensure_dir(&PathBuf::from("/parent/to")).await?;

    assert!(fs.exists(&PathBuf::from("/parent/from")).await?);
    assert!(fs.exists(&PathBuf::from("/parent/to")).await?);

    assert!(fs.metadata(&PathBuf::from("/parent/from")).await?.is_dir);
    assert!(fs.metadata(&PathBuf::from("/parent/to")).await?.is_dir);

    Ok(())
  }

  async fn test_write_file(fs: &PackMemoryFs) -> Result<()> {
    let mut writer = fs
      .write_file(&PathBuf::from("/parent/from/file.txt"))
      .await?;

    writer.line("hello").await?;
    writer.bytes(b" world").await?;
    writer.flush().await?;

    assert!(fs.exists(&PathBuf::from("/parent/from/file.txt")).await?);
    assert!(
      fs.metadata(&PathBuf::from("/parent/from/file.txt"))
        .await?
        .is_file
    );

    Ok(())
  }

  async fn test_read_file(fs: &PackMemoryFs) -> Result<()> {
    let mut reader = fs
      .read_file(&PathBuf::from("/parent/from/file.txt"))
      .await?;

    assert_eq!(reader.line().await?, "hello");
    assert_eq!(reader.bytes(b" world".len()).await?, b" world");

    Ok(())
  }

  async fn test_move_file(fs: &PackMemoryFs) -> Result<()> {
    fs.move_file(
      &PathBuf::from("/parent/from/file.txt"),
      &PathBuf::from("/parent/to/file.txt"),
    )
    .await?;
    assert!(!fs.exists(&PathBuf::from("/parent/from/file.txt")).await?);
    assert!(fs.exists(&PathBuf::from("/parent/to/file.txt")).await?);
    assert!(
      fs.metadata(&PathBuf::from("/parent/to/file.txt"))
        .await?
        .is_file
    );

    Ok(())
  }

  async fn test_remove_file(fs: &PackMemoryFs) -> Result<()> {
    fs.remove_file(&PathBuf::from("/parent/to/file.txt"))
      .await?;
    assert!(!fs.exists(&PathBuf::from("/parent/to/file.txt")).await?);
    Ok(())
  }

  async fn test_remove_dir(fs: &PackMemoryFs) -> Result<()> {
    fs.remove_dir(&PathBuf::from("/parent/from")).await?;
    fs.remove_dir(&PathBuf::from("/parent/to")).await?;
    assert!(!fs.exists(&PathBuf::from("/parent/from")).await?);
    assert!(!fs.exists(&PathBuf::from("/parent/to")).await?);
    Ok(())
  }

  async fn test_error(fs: &PackMemoryFs) -> Result<()> {
    match fs
      .metadata(&PathBuf::from("/parent/from/not_exist.txt"))
      .await
    {
      Ok(_) => panic!("should error"),
      Err(e) => assert_eq!(
        e.to_string(),
        r#"Rspack Storage FS Error: stat `/parent/from/not_exist.txt` failed with `IO error: file not exist`"#
      ),
    };

    Ok(())
  }

  async fn test_memory_fs(fs: &PackMemoryFs) -> Result<()> {
    test_create_dir(&fs).await?;
    test_write_file(&fs).await?;
    test_read_file(&fs).await?;
    test_move_file(&fs).await?;
    test_remove_file(&fs).await?;
    test_remove_dir(&fs).await?;
    test_error(&fs).await?;

    Ok(())
  }

  #[tokio::test]
  async fn should_pack_memory_fs_work() {
    let fs = PackMemoryFs(Arc::new(MemoryFileSystem::default()));

    let _ = test_memory_fs(&fs).await.map_err(|e| {
      panic!("{}", e);
    });
  }
}
