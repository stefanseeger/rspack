use quote::quote;
use syn::{parse::Parser, parse_macro_input, ItemStruct};

pub fn impl_runtime_module(
  _args: proc_macro::TokenStream,
  tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
  let mut input = parse_macro_input!(tokens as ItemStruct);
  let name = &input.ident;
  let generics = &input.generics;
  let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
  let origin_fields = input.fields.clone();

  if let syn::Fields::Named(ref mut fields) = input.fields {
    fields.named.push(
      syn::Field::parse_named
        .parse2(quote! { pub source_map_kind: ::rspack_util::source_map::SourceMapKind })
        .expect("Failed to parse new field for source_map_kind"),
    );
    fields.named.push(
      syn::Field::parse_named
        .parse2(quote! { pub custom_source: Option<::rspack_core::rspack_sources::BoxSource> })
        .expect("Failed to parse new field for custom_source"),
    );
  }

  let field_names = origin_fields
    .iter()
    .map(|field| field.ident.as_ref().expect("Expected named struct"))
    .collect::<Vec<_>>();
  let field_tys: Vec<&syn::Type> = origin_fields.iter().map(|field| &field.ty).collect();
  let with_default = quote! {
    #[allow(clippy::too_many_arguments)]
    fn with_default(#(#field_names: #field_tys,)*) -> Self {
      Self {
        source_map_kind: ::rspack_util::source_map::SourceMapKind::empty(),
        custom_source: None,
        #(#field_names,)*
      }
    }
  };

  quote! {
    #input

    impl #impl_generics #name #ty_generics #where_clause {
      #with_default

      fn get_generated_code(
        &self,
        compilation: &::rspack_core::Compilation,
      ) -> ::rspack_error::Result<std::sync::Arc<dyn ::rspack_core::rspack_sources::Source>> {
        use ::rspack_core::CustomSourceRuntimeModule;
        let source = if let Some(custom_source) = self.get_custom_source() {
          custom_source
        } else {
          let generated_code = self.generate(compilation)?;

          use ::rspack_core::rspack_sources::SourceExt;
          if self.source_map_kind.enabled() {
            use ::rspack_collections::Identifiable;
            ::rspack_core::rspack_sources::OriginalSource::new(generated_code, self.identifier().to_string()).boxed()
          } else {
            ::rspack_core::rspack_sources::RawSource::from(generated_code).boxed()
          }
        };
        Ok(source)
      }
    }

    impl #impl_generics ::rspack_core::CustomSourceRuntimeModule for #name #ty_generics #where_clause {
      fn set_custom_source(&mut self, source: ::rspack_core::rspack_sources::BoxSource) -> () {
        self.custom_source = Some(source);
      }
      fn get_custom_source(&self) -> Option<::rspack_core::rspack_sources::BoxSource> {
        self.custom_source.clone()
      }
      fn get_constructor_name(&self) -> String {
        String::from(stringify!(#name))
      }
    }

    impl #impl_generics ::rspack_collections::Identifiable for #name #ty_generics #where_clause {
      fn identifier(&self) -> ::rspack_collections::Identifier {
        self.name()
      }
    }

    impl #impl_generics ::rspack_core::DependenciesBlock for #name #ty_generics #where_clause {
      fn add_block_id(&mut self, _: ::rspack_core::AsyncDependenciesBlockIdentifier) {
        unreachable!()
      }

      fn get_blocks(&self) -> &[::rspack_core::AsyncDependenciesBlockIdentifier] {
        unreachable!()
      }

      fn add_dependency_id(&mut self, _: ::rspack_core::DependencyId) {
        unreachable!()
      }

      fn get_dependencies(&self) -> &[::rspack_core::DependencyId] {
        unreachable!()
      }
    }

    impl #impl_generics ::rspack_core::Module for #name #ty_generics #where_clause {
      fn module_type(&self) -> &::rspack_core::ModuleType {
        &::rspack_core::ModuleType::Runtime
      }

      fn source_types(&self) -> &[::rspack_core::SourceType] {
        &[::rspack_core::SourceType::JavaScript]
      }

      fn size(&self, _source_type: Option<&::rspack_core::SourceType>, compilation: &::rspack_core::Compilation) -> f64 {
        self.get_generated_code(compilation).ok().map(|source| source.size() as f64).unwrap_or(0f64)
      }

      fn readable_identifier(&self, _context: &::rspack_core::Context) -> std::borrow::Cow<str> {
        self.name().as_str().into()
      }

      fn original_source(&self) -> Option<&dyn ::rspack_core::rspack_sources::Source> {
        None
      }

      fn factory_meta(&self) -> Option<&::rspack_core::FactoryMeta> {
        None
      }

      fn set_factory_meta(&mut self, v: ::rspack_core::FactoryMeta) {}

      fn build_info(&self) -> Option<&::rspack_core::BuildInfo> {
        None
      }

      fn set_build_info(&mut self, v: ::rspack_core::BuildInfo) {}

      fn build_meta(&self) -> Option<&::rspack_core::BuildMeta> {
        None
      }

      fn set_build_meta(&mut self, v: ::rspack_core::BuildMeta) {}

      fn get_diagnostics(&self) -> Vec<::rspack_error::Diagnostic> {
        vec![]
      }

      fn code_generation(
        &self,
        compilation: &::rspack_core::Compilation,
        _runtime: Option<&::rspack_core::RuntimeSpec>,
        _: Option<::rspack_core::ConcatenationScope>,
      ) -> rspack_error::Result<::rspack_core::CodeGenerationResult> {
        let mut result = ::rspack_core::CodeGenerationResult::default();
        result.add(::rspack_core::SourceType::Runtime, self.get_generated_code(compilation)?);
        result.set_hash(
          &compilation.options.output.hash_function,
          &compilation.options.output.hash_digest,
          &compilation.options.output.hash_salt,
        );
        Ok(result)
      }

      fn update_hash(
        &self,
        hasher: &mut dyn std::hash::Hasher,
        compilation: &::rspack_core::Compilation,
        _runtime: Option<&::rspack_core::RuntimeSpec>,
      ) -> ::rspack_error::Result<()> {
        use rspack_util::ext::DynHash;
        self.name().dyn_hash(hasher);
        self.stage().dyn_hash(hasher);
        self.generate(compilation)?.dyn_hash(hasher);
        Ok(())
      }
    }

    impl #impl_generics rspack_error::Diagnosable for #name  #ty_generics #where_clause {}

    impl #impl_generics ::rspack_util::source_map::ModuleSourceMapConfig for #name #ty_generics #where_clause {
      fn get_source_map_kind(&self) -> &::rspack_util::source_map::SourceMapKind {
        &self.source_map_kind
      }

      fn set_source_map_kind(&mut self, source_map_kind: ::rspack_util::source_map::SourceMapKind) {
        self.source_map_kind = source_map_kind;
      }
    }
  }
  .into()
}
