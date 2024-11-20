use std::{path::PathBuf, sync::Arc};

use futures::{future::join_all, TryFutureExt};
use rspack_error::{error, Result};

use crate::pack::{PackFs, PackStrategy, ScopeStrategy, Strategy};

#[derive(Debug, Clone)]
pub struct SplitPackStrategy {
  pub fs: Arc<dyn PackFs>,
  pub root: Arc<PathBuf>,
  pub temp_root: Arc<PathBuf>,
}

impl SplitPackStrategy {
  pub fn new(root: PathBuf, temp_root: PathBuf, fs: Arc<dyn PackFs>) -> Self {
    Self {
      fs,
      root: Arc::new(root),
      temp_root: Arc::new(temp_root),
    }
  }

  pub async fn move_temp_files(&self, files: Vec<PathBuf>) -> Result<()> {
    let mut candidates = vec![];
    for to in files {
      let from = self.get_temp_path(&to)?;
      candidates.push((from, to));
    }

    let tasks = candidates.into_iter().map(|(from, to)| {
      let fs = self.fs.clone();
      tokio::spawn(async move { fs.move_file(&from, &to).await }).map_err(|e| error!("{}", e))
    });

    join_all(tasks)
      .await
      .into_iter()
      .collect::<Result<Vec<Result<()>>>>()?;

    Ok(())
  }

  pub async fn remove_files(&self, files: Vec<PathBuf>) -> Result<()> {
    let tasks = files.into_iter().map(|path| {
      let fs = self.fs.to_owned();
      tokio::spawn(async move { fs.remove_file(&path).await }).map_err(|e| error!("{}", e))
    });

    join_all(tasks)
      .await
      .into_iter()
      .collect::<Result<Vec<Result<()>>>>()?;

    Ok(())
  }

  pub fn get_temp_path(&self, path: &PathBuf) -> Result<PathBuf> {
    let relative_path = path
      .strip_prefix(&*self.root)
      .map_err(|e| error!("failed to get relative path: {}", e))?;
    Ok(self.temp_root.join(relative_path))
  }
}

impl Strategy for SplitPackStrategy {}
impl PackStrategy for SplitPackStrategy {}
impl ScopeStrategy for SplitPackStrategy {}
