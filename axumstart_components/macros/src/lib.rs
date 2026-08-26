use proc_macro::TokenStream;
use quote::quote;
use syn::{
  Expr, Fields, FnArg, Item, ItemFn, ItemStruct, LitStr, Path, ReturnType, Token, Type,
  parse_macro_input,
};

#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
  if !attr.is_empty() {
    return syn::Error::new(
      proc_macro2::Span::call_site(),
      "#[component] does not take arguments; for structs, use `#[derive(Component)]` \
       with `#[as_trait(Trait)]` for trait registration",
    )
    .to_compile_error()
    .into();
  }

  let item = parse_macro_input!(item as Item);

  match item {
    Item::Fn(f) => component_fn(f),
    Item::Struct(s) => syn::Error::new_spanned(
      &s,
      "#[component] no longer supports structs — use `#[derive(Component)]` instead \
       (add `#[as_trait(Trait)]` for trait registration)",
    )
    .to_compile_error()
    .into(),
    other => syn::Error::new_spanned(other, "#[component] only supports functions")
      .to_compile_error()
      .into(),
  }
}

/// `#[derive(Component)]` — registers a struct as a component. Fields resolve their
/// dependencies via `ComponentProvider` (or `Default::default()` for `#[default]`-marked
/// fields). Add `#[as_trait(Trait)]` on the struct to also register it as `Arc<dyn Trait>`.
#[proc_macro_derive(Component, attributes(default, as_trait))]
pub fn derive_component(input: TokenStream) -> TokenStream {
  let input = parse_macro_input!(input as ItemStruct);
  let as_trait = match extract_as_trait(&input) {
    Ok(t) => t,
    Err(e) => return e.to_compile_error().into(),
  };
  component_struct(as_trait, input)
}

fn extract_as_trait(input: &ItemStruct) -> syn::Result<Option<Path>> {
  match input.attrs.iter().find(|a| a.path().is_ident("as_trait")) {
    Some(attr) => attr.parse_args::<Path>().map(Some),
    None => Ok(None),
  }
}

fn component_struct(as_trait: Option<Path>, input: ItemStruct) -> TokenStream {
  let name = &input.ident;

  let fields_init = match &input.fields {
    Fields::Named(fields) => {
      let inits = fields.named.iter().map(|f| {
        let field_name = &f.ident;
        let init = if has_default_attr(f) { quote! { Default::default() } } else { field_init(&f.ty) };
        quote! { #field_name: #init }
      });
      quote! { Self { #(#inits),* } }
    }
    Fields::Unit => quote! { Self },
    Fields::Unnamed(fields) => {
      let inits = fields.unnamed.iter().map(|f| {
        if has_default_attr(f) { quote! { Default::default() } } else { field_init(&f.ty) }
      });
      quote! { Self(#(#inits),*) }
    }
  };

  let blueprint = quote! {
    #[::axumstart_components::async_trait]
    impl ::axumstart_components::ComponentBlueprint for #name {
      async fn new(ctx: &::axumstart_components::ComponentProvider) -> Self {
        #fields_init
      }
    }
  };

  let dyn_blueprint = match &as_trait {
    Some(trait_path) => quote! {
      impl ::axumstart_components::DynComponentBlueprint for #name {
        type Dyn = dyn #trait_path;
        fn upcast(arc: ::std::sync::Arc<Self>) -> ::std::sync::Arc<dyn #trait_path> {
          arc
        }
      }
    },
    None => quote! {},
  };

  let registration = quote! {
    ::axumstart_components::inventory::submit! {
      ::axumstart_components::ComponentRegistration(|ctx: &::axumstart_components::ComponentProvider| {
        ::axumstart_components::RegisterProbe::<#name>::new().register(ctx);
      })
    }
  };

  quote! {
    #blueprint
    #dyn_blueprint
    #registration
  }
  .into()
}

fn component_fn(input: ItemFn) -> TokenStream {
  let sig = &input.sig;
  let fn_name = &sig.ident;
  let is_async = sig.asyncness.is_some();

  let return_ty = match &sig.output {
    ReturnType::Type(_, ty) => ty,
    ReturnType::Default => {
      return syn::Error::new_spanned(
        sig,
        "#[component] functions must return the constructed type",
      )
      .to_compile_error()
      .into();
    }
  };

  if sig.inputs.len() > 1 {
    return syn::Error::new_spanned(
      &sig.inputs,
      "#[component] functions take at most one parameter, `&ComponentProvider`",
    )
    .to_compile_error()
    .into();
  }

  let has_ctx_param = match sig.inputs.first() {
    None => false,
    Some(FnArg::Receiver(receiver)) => {
      return syn::Error::new_spanned(receiver, "#[component] does not support methods")
        .to_compile_error()
        .into();
    }
    Some(FnArg::Typed(pat_type)) => {
      if !is_component_provider_ref(&pat_type.ty) {
        return syn::Error::new_spanned(
          &pat_type.ty,
          "#[component] function parameter must be `&ComponentProvider`",
        )
        .to_compile_error()
        .into();
      }
      true
    }
  };

  let call_expr = match (has_ctx_param, is_async) {
    (true, true) => quote! { #fn_name(&ctx).await },
    (true, false) => quote! { #fn_name(&ctx) },
    (false, true) => quote! { #fn_name().await },
    (false, false) => quote! { #fn_name() },
  };

  let registration = quote! {
    ::axumstart_components::inventory::submit! {
      ::axumstart_components::ComponentRegistration(|ctx: &::axumstart_components::ComponentProvider| {
        ::axumstart_components::FactoryProbe::<#return_ty>::new().register(
          ctx,
          |ctx: ::axumstart_components::ComponentProvider| async move { #call_expr },
        );
      })
    }
  };

  quote! {
    #input
    #registration
  }
  .into()
}

/// `#[derive(ComponentConfig)]` — a config struct whose every field is read from an
/// environment variable, either required (`#[env_var("VAR")]`) or with a fallback
/// (`#[env_var("VAR", default_expr)]`). Registers via `ComponentBlueprint` + `inventory`
/// exactly like `#[component]` (plain `register`, via `RegisterProbe`), so it's resolved
/// once via `register_all()` and shared through `ctx.get_cloned::<Config>()` rather than
/// re-read from the environment at every call site.
#[proc_macro_derive(ComponentConfig, attributes(env_var))]
pub fn derive_component_config(input: TokenStream) -> TokenStream {
  let input = parse_macro_input!(input as ItemStruct);
  component_config_struct(input)
}

struct EnvVarArgs {
  name: LitStr,
  default: Option<Expr>,
}

impl syn::parse::Parse for EnvVarArgs {
  fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
    let name: LitStr = input.parse()?;
    let default = if input.peek(Token![,]) {
      input.parse::<Token![,]>()?;
      Some(input.parse()?)
    } else {
      None
    };
    Ok(EnvVarArgs { name, default })
  }
}

fn component_config_struct(input: ItemStruct) -> TokenStream {
  let name = &input.ident;

  let fields = match &input.fields {
    Fields::Named(fields) => fields,
    other => {
      return syn::Error::new_spanned(
        other,
        "#[derive(ComponentConfig)] only supports structs with named fields",
      )
      .to_compile_error()
      .into();
    }
  };

  let mut field_inits = Vec::with_capacity(fields.named.len());
  for f in &fields.named {
    let field_name = f.ident.as_ref().unwrap();
    let field_ty = &f.ty;

    let env_attr = f.attrs.iter().find(|a| a.path().is_ident("env_var"));

    let init = match env_attr {
      Some(attr) => {
        let args: EnvVarArgs = match attr.parse_args() {
          Ok(a) => a,
          Err(e) => return e.to_compile_error().into(),
        };
        let name_lit = &args.name;
        match &args.default {
          Some(default_expr) => quote! {
            {
              let __name = #name_lit;
              match ::std::env::var(__name) {
                Ok(v) => v.parse::<#field_ty>().unwrap_or_else(|e| panic!("Environment variable {} invalid: {:?}", __name, e)),
                Err(_) => (#default_expr),
              }
            }
          },
          None => quote! {
            {
              let __name = #name_lit;
              ::std::env::var(__name)
                .unwrap_or_else(|_| panic!("Environment variable {} not set", __name))
                .parse::<#field_ty>()
                .unwrap_or_else(|e| panic!("Environment variable {} invalid: {:?}", __name, e))
            }
          },
        }
      }
      None => {
        return syn::Error::new_spanned(
          f,
          "field must have `#[env_var(\"VAR\")]` or `#[env_var(\"VAR\", default)]`",
        )
        .to_compile_error()
        .into();
      }
    };

    field_inits.push(quote! { #field_name: #init });
  }

  let blueprint = quote! {
    #[::axumstart_components::async_trait]
    impl ::axumstart_components::ComponentBlueprint for #name {
      async fn new(_ctx: &::axumstart_components::ComponentProvider) -> Self {
        Self { #(#field_inits),* }
      }
    }
  };

  let registration = quote! {
    ::axumstart_components::inventory::submit! {
      ::axumstart_components::ComponentRegistration(|ctx: &::axumstart_components::ComponentProvider| {
        ::axumstart_components::RegisterProbe::<#name>::new().register(ctx);
      })
    }
  };

  quote! {
    #blueprint
    #registration
  }
  .into()
}

fn is_component_provider_ref(ty: &Type) -> bool {
  if let Type::Reference(r) = ty {
    if let Type::Path(p) = &*r.elem {
      return p.path.segments.last().map(|s| s.ident == "ComponentProvider").unwrap_or(false);
    }
  }
  false
}

fn has_default_attr(field: &syn::Field) -> bool {
  field.attrs.iter().any(|a| a.path().is_ident("default"))
}

fn field_init(ty: &Type) -> proc_macro2::TokenStream {
  if is_arc(ty) {
    quote! { ctx.get().await }
  } else {
    quote! { ctx.get_cloned().await }
  }
}

fn is_arc(ty: &Type) -> bool {
  if let Type::Path(tp) = ty {
    tp.path.segments.last().map(|s| s.ident == "Arc").unwrap_or(false)
  } else {
    false
  }
}
