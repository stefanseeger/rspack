use rspack_collections::{Identifiable, Identifier};
use rspack_core::{
  impl_runtime_module,
  rspack_sources::{BoxSource, OriginalSource, RawSource, SourceExt},
  ChunkUkey, Compilation, DependenciesBlock, RuntimeModule, RuntimeModuleStage, SourceType,
};
use rustc_hash::FxHashMap;
use serde::Serialize;

use super::remote_module::RemoteModule;
use crate::utils::json_stringify;

#[impl_runtime_module]
#[derive(Debug)]
pub struct RemoteRuntimeModule {
  id: Identifier,
  chunk: Option<ChunkUkey>,
  enhanced: bool,
}

impl RemoteRuntimeModule {
  pub fn new(enhanced: bool) -> Self {
    Self::with_default(
      Identifier::from("webpack/runtime/remotes_loading"),
      None,
      enhanced,
    )
  }
}

impl RuntimeModule for RemoteRuntimeModule {
  fn name(&self) -> Identifier {
    self.id
  }

  fn stage(&self) -> RuntimeModuleStage {
    RuntimeModuleStage::Attach
  }

  fn attach(&mut self, chunk: ChunkUkey) {
    self.chunk = Some(chunk);
  }

  fn generate(&self, compilation: &Compilation) -> rspack_error::Result<BoxSource> {
    let chunk_ukey = self
      .chunk
      .expect("should have chunk in <RemoteRuntimeModule as RuntimeModule>::generate");
    let chunk = compilation.chunk_by_ukey.expect_get(&chunk_ukey);
    let mut chunk_to_remotes_mapping = FxHashMap::default();
    let mut id_to_remote_data_mapping = FxHashMap::default();
    let module_graph = compilation.get_module_graph();
    for chunk in chunk.get_all_async_chunks(&compilation.chunk_group_by_ukey) {
      let modules = compilation
        .chunk_graph
        .get_chunk_modules_iterable_by_source_type(&chunk, SourceType::Remote, &module_graph);
      let mut remotes = Vec::new();
      for m in modules {
        let Some(m) = m.downcast_ref::<RemoteModule>() else {
          continue;
        };
        let name = m.internal_request.as_str();
        let id = compilation
          .chunk_graph
          .get_module_id(m.identifier())
          .expect("should have module_id at <RemoteRuntimeModule as RuntimeModule>::generate");
        let share_scope = m.share_scope.as_str();
        let dep = m.get_dependencies()[0];
        let external_module = module_graph
          .get_module_by_dependency_id(&dep)
          .expect("should have module");
        let external_module_id = compilation
          .chunk_graph
          .get_module_id(external_module.identifier())
          .expect("should have module_id at <RemoteRuntimeModule as RuntimeModule>::generate");
        remotes.push(id.to_string());
        id_to_remote_data_mapping.insert(
          id,
          RemoteData {
            share_scope,
            name,
            external_module_id,
            remote_name: &m.remote_key,
          },
        );
      }
      if remotes.is_empty() {
        continue;
      }
      let chunk = compilation.chunk_by_ukey.expect_get(&chunk);
      chunk_to_remotes_mapping.insert(
        chunk
          .id
          .as_ref()
          .expect("should have chunkId at <RemoteRuntimeModule as RuntimeModule>::generate"),
        remotes,
      );
    }
    let remotes_loading_impl = if self.enhanced {
      "__webpack_require__.f.remotes = __webpack_require__.f.remotes || function() { throw new Error(\"should have __webpack_require__.f.remotes\"); }"
    } else {
      include_str!("./remotesLoading.js")
    };

    let generated_code = format!(
      r#"
__webpack_require__.remotesLoadingData = {{ chunkMapping: {chunk_mapping}, moduleIdToRemoteDataMapping: {id_to_remote_data_mapping} }};
{remotes_loading_impl}
"#,
      chunk_mapping = json_stringify(&chunk_to_remotes_mapping),
      id_to_remote_data_mapping = json_stringify(&id_to_remote_data_mapping),
      remotes_loading_impl = remotes_loading_impl,
    );
    let source = if self.source_map_kind.enabled() {
      OriginalSource::new(generated_code, self.identifier().to_string()).boxed()
    } else {
      RawSource::from(generated_code).boxed()
    };
    Ok(source)
  }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteData<'a> {
  share_scope: &'a str,
  name: &'a str,
  external_module_id: &'a str,
  remote_name: &'a str,
}
