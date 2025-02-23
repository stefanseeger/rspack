use std::marker::PhantomData;

use napi::Env;
use rspack_fs::{Error, Result, WritableFileSystem};
use rspack_paths::Utf8Path;

use crate::node::{NodeFS, NodeFSRef, TryIntoNodeFSRef};

pub struct NodeWritableFileSystem {
  env: Env,
  fs_ref: NodeFSRef,
  _data: PhantomData<*mut ()>,
}

impl std::fmt::Debug for NodeWritableFileSystem {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("NodeWritableFileSystem").finish()
  }
}

impl NodeWritableFileSystem {
  pub fn new(env: Env, fs: NodeFS) -> napi::Result<Self> {
    Ok(Self {
      env,
      fs_ref: fs.try_into_node_fs_ref(&env)?,
      _data: PhantomData,
    })
  }
}

impl WritableFileSystem for NodeWritableFileSystem {
  fn create_dir(&self, dir: &Utf8Path) -> Result<()> {
    let dir = dir.as_str();
    let mkdir = self.fs_ref.mkdir.get().expect("Failed to get mkdir");
    mkdir
      .call(
        None,
        &[self
          .env
          .create_string(dir)
          .expect("Failed to create string")],
      )
      .map_err(|err| {
        Error::Io(std::io::Error::new(
          std::io::ErrorKind::Other,
          err.to_string(),
        ))
      })?;

    Ok(())
  }

  fn create_dir_all(&self, dir: &Utf8Path) -> Result<()> {
    let dir = dir.as_str();
    let mkdirp = self.fs_ref.mkdirp.get().expect("Failed to get mkdirp");
    mkdirp
      .call(
        None,
        &[self
          .env
          .create_string(dir)
          .expect("Failed to create string")],
      )
      .map_err(|err| {
        Error::Io(std::io::Error::new(
          std::io::ErrorKind::Other,
          err.to_string(),
        ))
      })?;

    Ok(())
  }

  fn write(&self, file: &Utf8Path, data: &[u8]) -> Result<()> {
    let file = file.as_str();
    let buf = data.to_vec();
    let write_file = self
      .fs_ref
      .write_file
      .get()
      .expect("Failed to get write_file");

    write_file
      .call(
        None,
        &[
          self
            .env
            .create_string(file)
            .expect("Failed to create string")
            .into_unknown(),
          self
            .env
            .create_buffer_with_data(buf)
            .expect("Failed to create buffer")
            .into_unknown(),
        ],
      )
      .map_err(|err| {
        Error::Io(std::io::Error::new(
          std::io::ErrorKind::Other,
          err.to_string(),
        ))
      })?;

    Ok(())
  }
}
