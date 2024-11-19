use std::{
  fmt::Debug,
  future::Future,
  sync::mpsc::{channel, Sender},
};

use futures::{future::BoxFuture, FutureExt};

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

  pub fn add_task(&self, task: impl Future<Output = ()> + Send + 'static) {
    self
      .0
      .send(async move { task.await }.boxed())
      .expect("should add task");
  }
}
