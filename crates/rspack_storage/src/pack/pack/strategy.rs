use std::{hash::Hasher, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures::{future::join_all, TryFutureExt};
use itertools::Itertools;
use rspack_error::{error, Result};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};

use super::{Pack, PackKeysState};
use crate::{
  pack::{PackContents, PackContentsState, PackFileMeta, PackKeys, PackStorageFs, ScopeMeta},
  PackStorageOptions,
};

#[async_trait]
pub trait Strategy: std::fmt::Debug + Sync + Send {
  fn get_path(&self, sub: &str) -> PathBuf;
  fn get_temp_path(&self, path: &PathBuf) -> Result<PathBuf>;
  fn ensure_root(&self) -> Result<()>;

  fn get_hash(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<String>;
  fn create(
    &self,
    dir: &PathBuf,
    options: Arc<PackStorageOptions>,
    items: &mut Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>,
  ) -> Vec<(PackFileMeta, Pack)>;
  fn incremental(
    &self,
    dir: PathBuf,
    options: Arc<PackStorageOptions>,
    packs: &mut HashMap<Arc<PackFileMeta>, Pack>,
    updates: &mut HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
  ) -> PackIncrementalResult;
  fn write(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<()>;
  fn read_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>>;
  fn read_contents(&self, path: &PathBuf) -> Result<Option<PackContents>>;

  async fn before_save(&self) -> Result<()>;
  async fn after_save(&self, writed_files: Vec<PathBuf>, removed_files: Vec<PathBuf>)
    -> Result<()>;
  fn write_scope_meta(&self, meta: &ScopeMeta) -> Result<()>;
  fn read_scope_meta(&self, path: &PathBuf) -> Result<Option<ScopeMeta>>;
}

pub struct PackIncrementalResult {
  pub new_packs: Vec<(PackFileMeta, Pack)>,
  pub remain_packs: Vec<(Arc<PackFileMeta>, Pack)>,
  pub removed_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PackStrategy {
  fs: Arc<PackStorageFs>,
  root: Arc<PathBuf>,
  temp_root: Arc<PathBuf>,
}

impl PackStrategy {
  pub fn new(root: PathBuf, temp_root: PathBuf, fs: Arc<PackStorageFs>) -> Self {
    Self {
      fs,
      root: Arc::new(root),
      temp_root: Arc::new(temp_root),
    }
  }

  async fn move_temp_files(&self, files: Vec<PathBuf>) -> Result<()> {
    let mut candidates = vec![];
    for to in files {
      let from = self.get_temp_path(&to)?;
      candidates.push((from, to));
    }

    let tasks = candidates.into_iter().map(|(from, to)| {
      let fs = self.fs.to_owned();
      tokio::spawn(async move { fs.move_file(&from, &to).await }).map_err(|e| error!("{}", e))
    });

    join_all(tasks)
      .await
      .into_iter()
      .collect::<Result<Vec<Result<()>>>>()?;

    Ok(())
  }

  async fn remove_files(&self, files: Vec<PathBuf>) -> Result<()> {
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
}

#[async_trait]
impl Strategy for PackStrategy {
  fn read_scope_meta(&self, path: &PathBuf) -> Result<Option<ScopeMeta>> {
    if !self.fs.exists(path)? {
      return Ok(None);
    }

    let mut reader = self.fs.read_file(path)?;

    let meta_options = reader
      .line()?
      .split(" ")
      .map(|item| {
        item
          .parse::<u64>()
          .map_err(|e| error!("parse meta file failed: {}", e))
      })
      .collect::<Result<Vec<u64>>>()?;

    if meta_options.len() < 3 {
      return Err(error!("meta broken"));
    }

    let buckets = meta_options[0] as usize;
    let max_pack_size = meta_options[1] as usize;
    let last_modified = meta_options[2];

    let mut bucket_id = 0;
    let mut packs = vec![];
    while bucket_id < buckets {
      packs.push(
        reader
          .line()?
          .split(" ")
          .filter(|x| x.contains(","))
          .map(|x| x.split(",").collect::<Vec<_>>())
          .map(|info| {
            if info.len() < 2 {
              Err(error!("parse pack file info failed"))
            } else {
              Ok(Arc::new(PackFileMeta {
                name: info[0].to_owned(),
                hash: info[1].to_owned(),
                writed: true,
              }))
            }
          })
          .collect::<Result<Vec<Arc<PackFileMeta>>>>()?,
      );
      bucket_id += 1;
    }

    if packs.len() < buckets {
      return Err(error!("parse meta buckets failed"));
    }

    Ok(Some(ScopeMeta {
      path: path.clone(),
      buckets,
      max_pack_size,
      last_modified,
      packs,
    }))
  }

  fn write_scope_meta(&self, meta: &ScopeMeta) -> Result<()> {
    let path = self.get_temp_path(&meta.path)?;
    self
      .fs
      .ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))?;

    let mut writer = self.fs.write_file(&path)?;

    writer.line(
      format!(
        "{} {} {}",
        meta.buckets, meta.max_pack_size, meta.last_modified
      )
      .as_str(),
    )?;

    for bucket_id in 0..meta.buckets {
      let line = meta
        .packs
        .get(bucket_id)
        .map(|packs| {
          packs
            .iter()
            .map(|meta| format!("{},{}", meta.name, meta.hash))
            .join(" ")
        })
        .unwrap_or_default();
      writer.line(&line)?;
    }

    Ok(())
  }

  async fn before_save(&self) -> Result<()> {
    self.fs.remove_dir(&self.temp_root)?;
    self.fs.ensure_dir(&self.temp_root)?;
    self.fs.ensure_dir(&self.root)?;
    Ok(())
  }

  async fn after_save(
    &self,
    writed_files: Vec<PathBuf>,
    removed_files: Vec<PathBuf>,
  ) -> Result<()> {
    self.move_temp_files(writed_files).await?;
    self.remove_files(removed_files).await?;

    self.fs.remove_dir(&self.temp_root)?;
    // self.fs.clean_temporary()?;
    Ok(())
  }

  fn get_path(&self, str: &str) -> PathBuf {
    self.root.join(str)
  }

  fn get_temp_path(&self, path: &PathBuf) -> Result<PathBuf> {
    let relative_path = path
      .strip_prefix(&*self.root)
      .map_err(|e| error!("failed to get relative path: {}", e))?;
    Ok(self.temp_root.join(relative_path))
  }

  fn ensure_root(&self) -> Result<()> {
    self.fs.ensure_dir(&self.root)?;
    self.fs.ensure_dir(&self.temp_root)?;
    Ok(())
  }

  fn get_hash(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<String> {
    let mut hasher = FxHasher::default();
    let file_name = get_name(keys, contents);
    hasher.write(file_name.as_bytes());

    let meta = self.fs.read_file_meta(path)?;
    hasher.write_u64(meta.size);
    hasher.write_i64(meta.mtime);

    Ok(format!("{:016x}", hasher.finish()))
  }

  fn create(
    &self,
    dir: &PathBuf,
    options: Arc<PackStorageOptions>,
    items: &mut Vec<(Arc<Vec<u8>>, Arc<Vec<u8>>)>,
  ) -> Vec<(PackFileMeta, Pack)> {
    items.sort_unstable_by(|a, b| a.1.len().cmp(&b.1.len()));

    let mut new_packs = vec![];

    fn create_pack(dir: &PathBuf, keys: PackKeys, contents: PackContents) -> (PackFileMeta, Pack) {
      let file_name = get_name(&keys, &contents);
      let mut new_pack = Pack::new(dir.join(file_name.clone()));
      new_pack.keys = PackKeysState::Value(keys);
      new_pack.contents = PackContentsState::Value(contents);
      (
        PackFileMeta {
          name: file_name,
          hash: Default::default(),
          writed: false,
        },
        new_pack,
      )
    }

    loop {
      if items.len() == 0 {
        break;
      }
      // handle big single cache
      if items.last().expect("should have first item").1.len() as f64
        > options.max_pack_size as f64 * 0.5_f64
      {
        let (key, value) = items.pop().expect("shoud have first item");
        new_packs.push(create_pack(dir, vec![key], vec![value]));
      } else {
        break;
      }
    }

    loop {
      let mut batch_keys: PackKeys = vec![];
      let mut batch_contents: PackContents = vec![];
      let mut batch_size = 0_usize;

      loop {
        if items.len() == 0 {
          break;
        }

        if batch_size + items.last().expect("should have first item").1.len()
          > options.max_pack_size
        {
          break;
        }

        let (key, value) = items.pop().expect("shoud have first item");
        batch_size += value.len();
        batch_keys.push(key);
        batch_contents.push(value);
      }

      if !batch_keys.is_empty() {
        new_packs.push(create_pack(dir, batch_keys, batch_contents));
      }

      if items.len() == 0 {
        break;
      }
    }

    println!("new packs: {:?}", new_packs.len());
    new_packs
  }

  fn incremental(
    &self,
    dir: PathBuf,
    options: Arc<PackStorageOptions>,
    packs: &mut HashMap<Arc<PackFileMeta>, Pack>,
    updates: &mut HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
  ) -> PackIncrementalResult {
    let update_to_meta = packs
      .iter()
      .fold(HashMap::default(), |mut acc, (pack_meta, pack)| {
        let PackKeysState::Value(keys) = &pack.keys else {
          return acc;
        };
        for key in keys {
          acc.insert(key.clone(), pack_meta.clone());
        }
        acc
      });

    let mut removed_packs = HashSet::default();
    let mut insert_keys = HashSet::default();
    let mut removed_keys = HashSet::default();

    let mut removed_files = vec![];

    // TODO: try to update pack
    // let mut updated_packs = HashSet::default();
    // let mut updated_keys = HashSet::default();

    for (dirty_key, dirty_value) in updates.iter() {
      if dirty_value.is_some() {
        if let Some(pack_meta) = update_to_meta.get(dirty_key) {
          // update
          // updated_packs.insert(pack_meta);
          // updated_keys.insert(dirty_key)
          insert_keys.insert(dirty_key.clone());
          removed_packs.insert(pack_meta.clone());
        } else {
          // insert
          insert_keys.insert(dirty_key.clone());
        }
      } else {
        if let Some(pack_meta) = update_to_meta.get(dirty_key) {
          // remove
          removed_keys.insert(dirty_key.clone());
          removed_packs.insert(pack_meta.clone());
        } else {
          // not exists, do nothing
        }
      }
    }

    // pour out items from removed packs
    let mut wait_items = removed_packs
      .iter()
      .fold(vec![], |mut acc, pack_meta| {
        let old_pack = packs.remove(pack_meta).expect("should have bucket pack");

        removed_files.push(old_pack.path.clone());

        let (PackKeysState::Value(keys), PackContentsState::Value(contents)) =
          (old_pack.keys, old_pack.contents)
        else {
          return acc;
        };
        if keys.len() != contents.len() {
          return acc;
        }
        for (content_pos, content) in keys.iter().enumerate() {
          acc.push((
            content.to_owned(),
            contents
              .get(content_pos)
              .expect("should have content")
              .to_owned(),
          ));
        }
        acc
      })
      .into_iter()
      .filter(|(key, _)| !removed_keys.contains(key))
      .filter(|(key, _)| !insert_keys.contains(key))
      .collect::<Vec<_>>();

    // add insert items
    wait_items.extend(
      insert_keys
        .iter()
        .filter_map(|key| {
          updates
            .remove(key)
            .expect("should have insert item")
            .map(|val| (key.clone(), val))
        })
        .collect::<Vec<_>>(),
    );

    let remain_packs = packs
      .into_iter()
      .filter(|(meta, _)| !removed_packs.contains(*meta))
      .map(|(meta, pack)| (meta.clone(), pack.to_owned()))
      .collect::<Vec<_>>();

    let new_packs: Vec<(PackFileMeta, Pack)> = self.create(&dir, options, &mut wait_items);

    PackIncrementalResult {
      new_packs,
      remain_packs,
      removed_files,
    }
  }

  fn write(&self, path: &PathBuf, keys: &PackKeys, contents: &PackContents) -> Result<()> {
    let path = self.get_temp_path(path)?;
    self
      .fs
      .ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))?;

    let mut writer = self.fs.write_file(&path)?;

    // key meta line
    writer.line(
      keys
        .iter()
        .map(|key| key.len().to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .as_str(),
    )?;

    // content meta line
    writer.line(
      contents
        .iter()
        .map(|content| content.len().to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .as_str(),
    )?;

    for key in keys {
      writer.bytes(key)?;
    }

    for content in contents {
      writer.bytes(content)?;
    }

    Ok(())
  }

  fn read_keys(&self, path: &PathBuf) -> Result<Option<PackKeys>> {
    if !self.fs.exists(path)? {
      return Ok(None);
    }

    let mut reader = self.fs.read_file(path)?;
    let key_meta_list: Vec<usize> = reader
      .line()?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .collect();

    reader.line()?;

    let mut keys = vec![];
    for len in key_meta_list {
      keys.push(Arc::new(reader.bytes(len)?));
    }
    Ok(Some(keys))
  }

  fn read_contents(&self, path: &PathBuf) -> Result<Option<PackContents>> {
    if !self.fs.exists(path)? {
      return Ok(None);
    }

    let mut reader = self.fs.read_file(path)?;
    let total_key_size = reader
      .line()?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .fold(0_usize, |acc, key| acc + key);

    let content_meta_list: Vec<usize> = reader
      .line()?
      .split(" ")
      .map(|item| item.parse::<usize>().expect("should have meta info"))
      .collect();

    reader.skip(total_key_size)?;

    let mut res = vec![];
    for len in content_meta_list {
      res.push(Arc::new(reader.bytes(len)?));
    }

    Ok(Some(res))
  }
}

fn get_name(keys: &PackKeys, _: &PackContents) -> String {
  let mut hasher = FxHasher::default();
  for k in keys {
    hasher.write(k);
  }
  hasher.write_usize(keys.len());

  format!("{:016x}", hasher.finish())
}
