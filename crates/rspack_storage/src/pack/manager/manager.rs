use std::sync::Arc;

use futures::future::join_all;
use itertools::Itertools;
use rspack_error::{error, Result};
use rustc_hash::FxHashMap as HashMap;
use tokio::sync::oneshot::Receiver;
use tokio::sync::{oneshot, Mutex};

use super::TaskQueue;
use crate::pack::{PackScope, ScopeSaveResult, Strategy, ValidateResult};
use crate::{pack::ScopeUpdates, PackOptions};

type ScopeMap = HashMap<&'static str, PackScope>;

#[derive(Debug)]
pub struct ScopeManager {
  pub strategy: Arc<dyn Strategy>,
  pub options: Arc<PackOptions>,
  pub scopes: Arc<Mutex<ScopeMap>>,
  pub queue: TaskQueue,
}

impl ScopeManager {
  pub fn new(options: PackOptions, strategy: Arc<dyn Strategy>) -> Self {
    ScopeManager {
      strategy,
      options: Arc::new(options),
      scopes: Default::default(),
      queue: TaskQueue::new(),
    }
  }
  pub fn save(&self, updates: ScopeUpdates) -> Receiver<Result<()>> {
    let scopes = self.scopes.clone();
    let options = self.options.clone();
    let strategy = self.strategy.clone();

    let (tx, rx) = oneshot::channel();
    self.queue.add_task(Box::pin(async move {
      let scopes = std::mem::take(&mut *scopes.lock().await);
      let _ = match save_scopes(scopes, updates, options, strategy).await {
        Ok(_) => tx.send(Ok(())),
        Err(e) => tx.send(Err(e)),
      };
    }));

    rx
  }

  pub async fn get_all(&self, name: &'static str) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>> {
    let mut scopes = self.scopes.lock().await;
    let scope = scopes
      .entry(name)
      .or_insert_with(|| PackScope::new(name, self.options.clone(), self.strategy.clone()));

    match scope.validate(&self.options).await {
      Ok(validate) => match validate {
        ValidateResult::Valid => scope.get_contents().await,
        ValidateResult::Invalid(reason) => {
          scopes.clear();
          Err(error!("cache is not validate: {}", reason))
        }
      },
      Err(e) => {
        scopes.clear();
        Err(error!("cache is not validate: {}", e))
      }
    }
  }
}

async fn save_scopes(
  mut scopes: ScopeMap,
  mut updates: ScopeUpdates,
  options: Arc<PackOptions>,
  strategy: Arc<dyn Strategy>,
) -> Result<ScopeMap> {
  strategy.before_save().await?;

  for (scope_name, _) in updates.iter() {
    scopes
      .entry(scope_name)
      .or_insert_with(|| PackScope::empty(scope_name, options.clone(), strategy.clone()));
  }

  let update_tasks = join_all(
    scopes
      .iter_mut()
      .map(|(name, scope)| (scope, updates.remove(name).unwrap_or_default()))
      .collect_vec()
      .into_iter()
      .map(|(scope, updates)| scope.update(updates)),
  );

  update_tasks.await.into_iter().collect::<Result<()>>()?;

  let mut scopes = scopes.into_iter().collect_vec();
  let save_tasks = join_all(
    scopes
      .iter_mut()
      .map(|(_, scope)| scope.save())
      .collect_vec(),
  );
  let (writed_files, removed_files) = save_tasks
    .await
    .into_iter()
    .collect::<Result<Vec<ScopeSaveResult>>>()?
    .into_iter()
    .fold((vec![], vec![]), |mut acc, s| {
      acc.0.extend(s.writed_files);
      acc.1.extend(s.removed_files);
      acc
    });

  strategy.after_save(writed_files, removed_files).await?;

  Ok(scopes.into_iter().collect())
}
