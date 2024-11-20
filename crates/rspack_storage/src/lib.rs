// mod fs;
mod pack;

use std::sync::Arc;

// pub use fs::FsStorage;
pub use pack::{PackFs, PackMemoryFs, PackNativeFs, PackOptions, PackStorage};
use rspack_error::Result;
use tokio::sync::oneshot::Receiver;

#[async_trait::async_trait]
pub trait Storage: std::fmt::Debug + Sync + Send {
  async fn get_all(&self, scope: &'static str) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>>;
  fn set(&self, scope: &'static str, key: Vec<u8>, value: Vec<u8>);
  fn remove(&self, scope: &'static str, key: &[u8]);
  fn idle(&self) -> Receiver<Result<()>>;
}

pub type ArcStorage = Arc<dyn Storage>;
