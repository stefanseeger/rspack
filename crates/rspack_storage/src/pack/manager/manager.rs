use std::{
  borrow::BorrowMut,
  sync::{Arc, Mutex},
};

use futures::{channel::oneshot::Receiver, future::join_all};
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rspack_error::{error, Result};
use rustc_hash::FxHashMap as HashMap;

use super::TaskQueue;
use crate::pack::{PackScope, ScopeSaveResult, Strategy, ValidateResult};
use crate::{pack::ScopeUpdates, PackStorageOptions};

type ScopeMap = HashMap<&'static str, PackScope>;

#[derive(Debug)]
pub struct ScopeManager {
  strategy: Arc<dyn Strategy>,
  options: Arc<PackStorageOptions>,
  scopes: Arc<Mutex<ScopeMap>>,
  queue: TaskQueue,
}

impl ScopeManager {
  pub fn new(options: PackStorageOptions, strategy: Arc<dyn Strategy>) -> Self {
    ScopeManager {
      strategy,
      options: Arc::new(options),
      scopes: Default::default(),
      queue: TaskQueue::new(),
    }
  }
  pub fn update(&mut self, updates: &mut ScopeUpdates) -> Result<Receiver<()>> {
    update_scopes(
      self.options.clone(),
      self.strategy.clone(),
      self.scopes.lock().unwrap().borrow_mut(),
      updates,
    )?;
    self.start()
  }

  fn start(&mut self) -> Result<Receiver<()>> {
    let scopes_mutex = self.scopes.clone();
    let strategy = self.strategy.clone();

    self.queue.add_task(Box::pin(async move {
      let scopes = std::mem::take(&mut *scopes_mutex.lock().unwrap());
      match save_scopes(scopes, strategy).await {
        Ok(new_scopes) => {
          *scopes_mutex.lock().unwrap() = new_scopes;
        }
        Err(e) => println!("{}", e),
      };
    }))
  }

  pub fn get_all(&mut self, name: &'static str) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>> {
    let mut scopes = self.scopes.lock().unwrap();
    let scope = scopes
      .entry(name)
      .or_insert_with(|| PackScope::new(name, self.options.clone(), self.strategy.clone()));

    match scope.validate(&self.options) {
      Ok(validate) => match validate {
        ValidateResult::Valid => scope.get_contents(),
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

fn update_scopes(
  options: Arc<PackStorageOptions>,
  strategy: Arc<dyn Strategy>,
  scopes: &mut ScopeMap,
  updates: &mut ScopeUpdates,
) -> Result<()> {
  let scopes = scopes;

  for (scope_name, _) in updates.iter() {
    scopes
      .entry(scope_name)
      .or_insert_with(|| PackScope::empty(scope_name, options.clone(), strategy.clone()));
  }

  scopes
    .iter_mut()
    .map(|(name, scope)| (scope, updates.remove(name).unwrap_or_default()))
    .collect_vec()
    .into_par_iter()
    .map(|(scope, updates)| scope.update(updates))
    .collect::<Result<()>>()
}

async fn save_scopes(scopes: ScopeMap, strategy: Arc<dyn Strategy>) -> Result<ScopeMap> {
  strategy.before_save().await?;

  let mut scopes = scopes.into_iter().collect_vec();
  let tasks = join_all(
    scopes
      .iter_mut()
      .map(|(_, scope)| scope.save())
      .collect_vec(),
  );
  let (writed_files, removed_files) = tasks
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
