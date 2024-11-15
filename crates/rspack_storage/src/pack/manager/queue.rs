use std::{
  fmt::Debug,
  future::Future,
  sync::mpsc::{channel, Sender},
};

use futures::{
  channel::oneshot::{self, Receiver},
  future::BoxFuture,
  FutureExt,
};
use rspack_error::{error, Result};

pub struct TaskQueue(Sender<BoxFuture<'static, ()>>);

impl Debug for TaskQueue {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "TaskQueue {{... }}")
  }
}

impl TaskQueue {
  pub fn new() -> Self {
    let (tx, rx) = channel();
    tokio::spawn(async move {
      while let Ok(future) = rx.recv() {
        future.await
      }
    });

    Self(tx)
  }

  pub fn add_task<F: Send + 'static>(
    &self,
    task: impl Future<Output = F> + Send + 'static,
  ) -> Result<Receiver<F>> {
    let (tx, rx) = oneshot::channel();
    self
      .0
      .send(
        async move {
          let res = task.await;
          let _ = tx.send(res);
        }
        .boxed(),
      )
      .map_err(|e| error!("{}", e))?;
    Ok(rx)
  }
}
