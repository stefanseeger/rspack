use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use rspack_error::Result;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::SplitPackStrategy;
use crate::{
  pack::{
    strategy::split::util::get_name, Pack, PackContents, PackContentsState, PackFileMeta,
    PackIncrementalResult, PackKeys, PackKeysState, PackWriteStrategy,
  },
  PackOptions,
};

#[async_trait]
impl PackWriteStrategy for SplitPackStrategy {
  async fn update_pack(
    &self,
    dir: PathBuf,
    options: &PackOptions,
    mut packs: HashMap<Arc<PackFileMeta>, Pack>,
    mut updates: HashMap<Arc<Vec<u8>>, Option<Arc<Vec<u8>>>>,
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

    for (pack_meta, _) in packs.iter() {
      if options.max_pack_size as f64 * 0.8_f64 > pack_meta.size as f64 {
        removed_packs.insert(pack_meta.clone());
      }
    }

    for (dirty_key, dirty_value) in updates.iter() {
      if dirty_value.is_some() {
        insert_keys.insert(dirty_key.clone());
        if let Some(pack_meta) = update_to_meta.get(dirty_key) {
          removed_packs.insert(pack_meta.clone());
        }
      } else {
        removed_keys.insert(dirty_key.clone());
        if let Some(pack_meta) = update_to_meta.get(dirty_key) {
          removed_packs.insert(pack_meta.clone());
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
      .filter(|(meta, _)| !removed_packs.contains(meta))
      .map(|(meta, pack)| (meta.clone(), pack.to_owned()))
      .collect::<Vec<_>>();

    let new_packs: Vec<(PackFileMeta, Pack)> = create(&dir, options, &mut wait_items).await;

    PackIncrementalResult {
      new_packs,
      remain_packs,
      removed_files,
    }
  }

  async fn write_pack(
    &self,
    path: &PathBuf,
    keys: &PackKeys,
    contents: &PackContents,
  ) -> Result<()> {
    let path = self.get_temp_path(path)?;
    self
      .fs
      .ensure_dir(&PathBuf::from(path.parent().expect("should have parent")))
      .await?;

    let mut writer = self.fs.write_file(&path).await?;

    // key meta line
    writer
      .line(
        keys
          .iter()
          .map(|key| key.len().to_string())
          .collect::<Vec<_>>()
          .join(" ")
          .as_str(),
      )
      .await?;

    // content meta line
    writer
      .line(
        contents
          .iter()
          .map(|content| content.len().to_string())
          .collect::<Vec<_>>()
          .join(" ")
          .as_str(),
      )
      .await?;

    for key in keys {
      writer.bytes(key).await?;
    }

    for content in contents {
      writer.bytes(content).await?;
    }

    writer.flush().await?;

    Ok(())
  }
}

async fn create(
  dir: &PathBuf,
  options: &PackOptions,
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
        size: new_pack.size(),
        writed: false,
      },
      new_pack,
    )
  }

  loop {
    if items.len() == 0 {
      break;
    }
    let last_item = items.last().expect("should have first item");
    // handle big single cache
    if last_item.0.len() as f64 + last_item.1.len() as f64 > options.max_pack_size as f64 * 0.8_f64
    {
      let (key, value) = items.pop().expect("shoud have first item");
      new_packs.push(create_pack(dir, vec![key], vec![value]));
    } else {
      break;
    }
  }

  items.reverse();

  loop {
    let mut batch_keys: PackKeys = vec![];
    let mut batch_contents: PackContents = vec![];
    let mut batch_size = 0_usize;

    loop {
      if items.len() == 0 {
        break;
      }

      let last_item = items.last().expect("should have first item");

      if batch_size + last_item.0.len() + last_item.1.len() > options.max_pack_size {
        break;
      }

      let (key, value) = items.pop().expect("shoud have first item");
      batch_size += value.len() + key.len();
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

  new_packs
}
