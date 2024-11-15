use std::{
  hash::Hasher,
  path::PathBuf,
  sync::Arc,
  time::{SystemTime, UNIX_EPOCH},
};

use futures::executor::block_on;
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rspack_error::{error, Result};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};
use tokio::task::unconstrained;

use crate::pack::{
  batch_read_contents, batch_read_keys, batch_validate, batch_write_packs, PackKeysState, Strategy,
};
use crate::pack::{
  Pack, PackContentsState, PackFileMeta, PackStorageOptions, PackValidateCandidate, ScopeMeta,
};

#[derive(Debug)]
pub enum ScopeValidateResult {
  Valid,
  Invalid(String),
}

#[derive(Debug, Default, Clone)]
pub enum ScopeMetaState {
  #[default]
  Pending,
  Value(ScopeMeta),
}

impl ScopeMetaState {
  pub fn expect_value(&self) -> &ScopeMeta {
    match self {
      ScopeMetaState::Value(v) => v,
      ScopeMetaState::Pending => panic!("should have scope meta"),
    }
  }
  pub fn expect_value_mut(&mut self) -> &mut ScopeMeta {
    match self {
      ScopeMetaState::Value(ref mut v) => v,
      ScopeMetaState::Pending => panic!("should have scope meta"),
    }
  }
  pub fn take_value(&mut self) -> Option<ScopeMeta> {
    match self {
      ScopeMetaState::Value(v) => Some(std::mem::take(&mut *v)),
      _ => None,
    }
  }
}

type ScopePacks = Vec<Vec<Pack>>;

#[derive(Debug, Default, Clone)]
pub enum ScopePacksState {
  #[default]
  Pending,
  Value(ScopePacks),
}

impl ScopePacksState {
  pub fn expect_value(&self) -> &ScopePacks {
    match self {
      ScopePacksState::Value(v) => v,
      ScopePacksState::Pending => panic!("scope meta is not ready"),
    }
  }
  pub fn expect_value_mut(&mut self) -> &mut ScopePacks {
    match self {
      ScopePacksState::Value(v) => v,
      ScopePacksState::Pending => panic!("scope meta is not ready"),
    }
  }
  pub fn take_value(&mut self) -> Option<ScopePacks> {
    match self {
      ScopePacksState::Value(v) => Some(std::mem::take(&mut *v)),
      _ => None,
    }
  }
}

#[derive(Debug, Clone)]
pub struct PackScope {
  pub name: &'static str,
  pub path: Arc<PathBuf>,
  pub options: Arc<PackStorageOptions>,
  pub meta: ScopeMetaState,
  pub packs: ScopePacksState,
  pub removed: HashSet<PathBuf>,
  pub strategy: Arc<dyn Strategy>,
}

impl PackScope {
  pub fn new(
    name: &'static str,
    options: Arc<PackStorageOptions>,
    strategy: Arc<dyn Strategy>,
  ) -> Self {
    let path = strategy.get_path(name);
    Self {
      name,
      path: Arc::new(path),
      options,
      meta: ScopeMetaState::Pending,
      packs: ScopePacksState::Pending,
      removed: HashSet::default(),
      strategy,
    }
  }

  pub fn empty(
    name: &'static str,
    options: Arc<PackStorageOptions>,
    strategy: Arc<dyn Strategy>,
  ) -> Self {
    let scope_path = strategy.get_path(name);
    let meta = ScopeMeta::new(&scope_path, options.clone());
    let packs = vec![vec![]; options.buckets];

    Self {
      name,
      path: Arc::new(scope_path),
      options,
      meta: ScopeMetaState::Value(meta),
      packs: ScopePacksState::Value(packs),
      removed: HashSet::default(),
      strategy,
    }
  }

  pub fn loaded(&self) -> bool {
    matches!(self.meta, ScopeMetaState::Value(_))
      && matches!(self.packs, ScopePacksState::Value(_))
      && self
        .packs
        .expect_value()
        .iter()
        .flatten()
        .all(|pack| pack.loaded())
  }

  pub fn get_contents(&mut self) -> Result<Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>> {
    self.ensure_pack_contents()?;

    let packs = self.packs.expect_value();
    let contents = packs
      .iter()
      .flatten()
      .filter_map(|pack| {
        if let (PackKeysState::Value(keys), PackContentsState::Value(contents)) =
          (&pack.keys, &pack.contents)
        {
          if keys.len() == contents.len() {
            return Some(
              keys
                .iter()
                .enumerate()
                .map(|(index, key)| (key.clone(), contents[index].clone()))
                .collect_vec(),
            );
          }
        }
        None
      })
      .flatten()
      .collect_vec();

    Ok(contents)
  }

  pub fn validate(&mut self, options: &PackStorageOptions) -> Result<ScopeValidateResult> {
    self.ensure_meta()?;

    // validate meta
    let meta = self.meta.expect_value();
    if meta.buckets != options.buckets || meta.max_pack_size != options.max_pack_size {
      return Ok(ScopeValidateResult::Invalid(
        "scope options changed".to_string(),
      ));
    }

    let current_time = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("get current time failed")
      .as_secs();

    if current_time - meta.last_modified > options.expires {
      return Ok(ScopeValidateResult::Invalid("scope expired".to_string()));
    }

    // validate packs
    let validate = self.validate_packs()?;
    if validate {
      Ok(ScopeValidateResult::Valid)
    } else {
      Ok(ScopeValidateResult::Invalid(
        "scope cache files changed".to_string(),
      ))
    }
  }

  fn validate_packs(&mut self) -> Result<bool> {
    self.ensure_pack_keys()?;
    let candidates = {
      let packs = self.get_pack_meta_pairs()?;
      packs
        .iter()
        .map(|(bucket_id, _, pack_meta, pack)| PackValidateCandidate {
          path: self.path.join(bucket_id.to_string()).join(&pack_meta.name),
          hash: pack_meta.hash.to_owned(),
          keys: pack.keys.expect_value().to_owned(),
          contents: pack.keys.expect_value().to_owned(),
        })
        .collect_vec()
    };
    let validate_results = block_on(unconstrained(batch_validate(
      candidates,
      self.strategy.clone(),
    )))?;
    Ok(validate_results.into_iter().all(|v| v))
  }

  fn ensure_pack_keys(&mut self) -> Result<()> {
    self.ensure_packs()?;
    let candidates = {
      let packs_pairs = self.get_pack_meta_pairs()?;
      let pack_infos = packs_pairs
        .into_iter()
        .filter(|(_, _, _, pack)| matches!(pack.keys, PackKeysState::Pending))
        .collect_vec();
      pack_infos
        .iter()
        .map(|args| (args.0, args.1, args.3.path.to_owned()))
        .collect_vec()
    };
    let read_key_results = block_on(unconstrained(batch_read_keys(
      candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
      self.strategy.clone(),
    )))?;
    let packs = self.packs.expect_value_mut();
    for (index, keys) in read_key_results.into_iter().enumerate() {
      let (bucket_id, pack_pos, _) = candidates.get(index).expect("should have pack info");
      if let Some(pack) = packs
        .get_mut(*bucket_id)
        .and_then(|packs| packs.get_mut(*pack_pos))
      {
        pack.keys = PackKeysState::Value(keys);
      }
    }
    Ok(())
  }

  fn ensure_pack_contents(&mut self) -> Result<()> {
    self.ensure_pack_keys()?;

    let candidates = {
      let packs_pairs = self.get_pack_meta_pairs()?;
      let pack_infos = packs_pairs
        .into_iter()
        .filter(|(_, _, _, pack)| {
          matches!(pack.contents, PackContentsState::Pending)
            && matches!(pack.keys, PackKeysState::Value(_))
        })
        .collect_vec();
      pack_infos
        .iter()
        .map(|args| (args.0, args.1, args.3.path.to_owned()))
        .collect_vec()
    };

    let read_contents_result: Vec<Vec<Arc<Vec<u8>>>> =
      block_on(unconstrained(batch_read_contents(
        candidates.iter().map(|i| i.2.to_owned()).collect_vec(),
        self.strategy.clone(),
      )))?;

    let packs = self.packs.expect_value_mut();
    for (index, contents) in read_contents_result.into_iter().enumerate() {
      let (bucket_id, pack_pos, _) = candidates.get(index).expect("should have pack info");
      if let Some(pack) = packs
        .get_mut(*bucket_id)
        .and_then(|packs| packs.get_mut(*pack_pos))
      {
        pack.contents = PackContentsState::Value(contents);
      }
    }

    Ok(())
  }

  fn ensure_meta(&mut self) -> Result<()> {
    if matches!(self.meta, ScopeMetaState::Pending) {
      let scope_path = ScopeMeta::get_path(&self.path);
      let meta = self.strategy.read_scope_meta(&scope_path)?;
      if let Some(meta) = meta {
        self.meta = ScopeMetaState::Value(meta);
      } else {
        self.meta = ScopeMetaState::Value(ScopeMeta::new(&self.path, self.options.clone()));
      }
    }
    Ok(())
  }

  fn ensure_packs(&mut self) -> Result<()> {
    self.ensure_meta()?;

    let meta = self.meta.expect_value();

    if matches!(self.packs, ScopePacksState::Value(_)) {
      return Ok(());
    }

    self.packs = ScopePacksState::Value(
      meta
        .packs
        .iter()
        .enumerate()
        .map(|(bucket_id, pack_meta_list)| {
          let bucket_dir = self.path.join(bucket_id.to_string());
          pack_meta_list
            .iter()
            .map(|pack_meta| Pack::new(bucket_dir.join(&pack_meta.name)))
            .collect_vec()
        })
        .collect_vec(),
    );

    Ok(())
  }

  fn get_pack_meta_pairs(&self) -> Result<Vec<(usize, usize, Arc<PackFileMeta>, &Pack)>> {
    let meta = self.meta.expect_value();
    let packs = self.packs.expect_value();

    Ok(
      meta
        .packs
        .iter()
        .enumerate()
        .map(|(bucket_id, pack_meta_list)| {
          let bucket_packs = packs.get(bucket_id).expect("should have bucket packs");
          pack_meta_list
            .iter()
            .enumerate()
            .map(|(pack_pos, pack_meta)| {
              (
                bucket_id,
                pack_pos,
                pack_meta.clone(),
                bucket_packs.get(pack_pos).expect("should have bucket pack"),
              )
            })
            .collect_vec()
        })
        .flatten()
        .collect_vec(),
    )
  }

  pub fn update(&mut self, updates: HashMap<Vec<u8>, Option<Vec<u8>>>) {
    let mut scope_meta = self.meta.take_value().expect("shoud have scope meta");
    let mut scope_packs = self.packs.take_value().expect("shoud have scope packs");

    // get changed buckets
    let bucket_updates = updates
      .into_par_iter()
      .map(|(key, value)| {
        let bucket_id = choose_bucket(&key, self.options.buckets);
        (bucket_id, key, value)
      })
      .collect::<Vec<_>>()
      .into_iter()
      .fold(
        HashMap::<usize, HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>>::default(),
        |mut res, (bucket_id, key, value)| {
          res
            .entry(bucket_id)
            .or_default()
            .insert(Arc::new(key.to_owned()), value.to_owned().map(Arc::new));
          res
        },
      );

    // get dirty buckets
    let mut bucket_tasks = vec![];
    for (dirty_bucket_id, dirty_items) in bucket_updates.into_iter() {
      let dirty_bucket_packs = {
        let mut packs = HashMap::default();

        let old_dirty_bucket_metas = std::mem::take(
          scope_meta
            .packs
            .get_mut(dirty_bucket_id)
            .expect("should have bucket pack metas"),
        )
        .into_iter()
        .enumerate()
        .collect::<HashMap<_, _>>();

        let mut old_dirty_bucket_packs = std::mem::take(
          scope_packs
            .get_mut(dirty_bucket_id)
            .expect("should have bucket packs"),
        )
        .into_iter()
        .enumerate()
        .collect::<HashMap<_, _>>();

        for (key, pack_meta) in old_dirty_bucket_metas.into_iter() {
          let pack = old_dirty_bucket_packs
            .remove(&key)
            .expect("should have bucket pack");
          packs.insert(pack_meta, pack);
        }
        packs
      };

      bucket_tasks.push((dirty_bucket_id, dirty_bucket_packs, dirty_items));
    }

    // generate dirty buckets
    let dirty_bucket_results = bucket_tasks
      .into_par_iter()
      .map(|(bucket_id, mut bucket_packs, mut bucket_updates)| {
        let bucket_res = self.strategy.incremental(
          self.path.join(bucket_id.to_string()),
          self.options.clone(),
          &mut bucket_packs,
          &mut bucket_updates,
        );
        (bucket_id, bucket_res)
      })
      .collect::<HashMap<_, _>>();

    let mut total_files = HashSet::default();
    // link remain packs to scope
    for (bucket_id, bucket_result) in dirty_bucket_results {
      for (pack_meta, pack) in bucket_result.remain_packs {
        total_files.insert(pack.path.clone());
        scope_packs[bucket_id].push(pack);
        scope_meta.packs[bucket_id].push(pack_meta);
      }

      for (pack_meta, pack) in bucket_result.new_packs {
        total_files.insert(pack.path.clone());
        scope_packs[bucket_id].push(pack);
        scope_meta.packs[bucket_id].push(Arc::new(pack_meta));
      }

      self.removed.extend(bucket_result.removed_files);
    }

    // should not remove pack files
    self.removed = std::mem::take(&mut self.removed)
      .into_iter()
      .filter(|r| total_files.contains(r))
      .collect();

    self.packs = ScopePacksState::Value(scope_packs);
    self.meta = ScopeMetaState::Value(scope_meta);
  }

  pub async fn save(&mut self) -> Result<SavedScopeResult> {
    if !self.loaded() {
      return Err(error!("scope not loaded, run `get_all` first"));
    }
    let removed_files = std::mem::take(&mut self.removed);
    let packs = self.packs.expect_value();
    let meta = self.meta.expect_value_mut();

    let mut writed_files = HashSet::default();

    let mut candidates = packs
      .iter()
      .flatten()
      .zip(meta.packs.iter_mut().flatten())
      .filter(|(_, meta)| !meta.writed)
      .collect_vec();

    let write_results = batch_write_packs(
      candidates.iter().map(|i| i.0.clone()).collect_vec(),
      self.strategy.clone(),
    )
    .await?;

    for ((_, meta), (hash, path)) in candidates.iter_mut().zip(write_results.into_iter()) {
      std::mem::replace(
        *meta,
        Arc::new(PackFileMeta {
          hash,
          name: meta.name.clone(),
          writed: true,
        }),
      );
      writed_files.insert(path);
    }

    let meta = self.meta.expect_value();
    writed_files.insert(meta.path.clone());
    self.strategy.write_scope_meta(&meta)?;

    Ok(SavedScopeResult {
      writed_files,
      removed_files,
    })
  }
}

pub struct SavedScopeResult {
  pub writed_files: HashSet<PathBuf>,
  pub removed_files: HashSet<PathBuf>,
}

pub fn choose_bucket(key: &Vec<u8>, total: usize) -> usize {
  let mut hasher = FxHasher::default();
  hasher.write(key);
  let bucket_id = usize::try_from(hasher.finish() % total as u64).expect("should get bucket id");
  bucket_id
}
