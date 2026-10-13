use proc_macro::TokenStream;
use quote::quote;
use syn::{Expr, ItemFn, LitStr, parse_macro_input};

mod krate;

const METHODS: &[&str] = &[
  "get", "post", "put", "delete", "patch", "head", "options", "trace", "connect",
];

enum ThisAttr {
  Method(&'static str, LitStr),
  Layer(Expr),
}

macro_rules! method_attr {
  ($name:ident) => {
    #[proc_macro_attribute]
    pub fn $name(attr: TokenStream, item: TokenStream) -> TokenStream {
      let path = parse_macro_input!(attr as LitStr);
      expand(ThisAttr::Method(stringify!($name), path), item)
    }
  };
}

method_attr!(get);
method_attr!(post);
method_attr!(put);
method_attr!(delete);
method_attr!(patch);
method_attr!(head);
method_attr!(options);
method_attr!(trace);
method_attr!(connect);

/// `#[layer(expr)]` — attaches a `tower::Layer` to this one route (and only this route),
/// via `MethodRouter::layer`, independent of any other auto-registered route. `expr` is either
/// a ready expression (`TraceLayer::new_for_http()`) or a bare function name, which gets called
/// for us — optionally with `&Components`, if the function takes it (see
/// `axumstart_router::LayerFactoryProbe`). Stack multiple `#[layer(...)]` to chain them, applied
/// in the order written. Must be combined with exactly one `#[get]`/`#[post]`/etc. on the same
/// function — order between `#[layer(...)]` and the method attribute doesn't matter.
#[proc_macro_attribute]
pub fn layer(attr: TokenStream, item: TokenStream) -> TokenStream {
  let expr = parse_macro_input!(attr as Expr);
  expand(ThisAttr::Layer(expr), item)
}

// Rust expands stacked attribute macros outside-in: whichever of #[get]/.../#[layer] is
// topmost on a given function is the one actually invoked, and it receives the rest still
// attached as plain (unexpanded) attributes on the parsed item. So every entry point above
// funnels into this one function, which reconstructs the whole picture — the method+path and
// every #[layer(...)] in source order — by combining its own value with whatever's left on
// `item_fn.attrs`, then strips all of it so the compiler doesn't try to re-expand anything.
fn expand(this: ThisAttr, item: TokenStream) -> TokenStream {
  let mut item_fn = parse_macro_input!(item as ItemFn);
  let ax = krate::path();

  let mut method: Option<(&'static str, LitStr)> = None;
  let mut layers: Vec<Expr> = Vec::new();

  match this {
    ThisAttr::Method(name, path) => method = Some((name, path)),
    ThisAttr::Layer(expr) => layers.push(expr),
  }

  let mut remaining_attrs = Vec::with_capacity(item_fn.attrs.len());
  for attr in item_fn.attrs.drain(..) {
    if let Some(name) = METHODS.iter().find(|m| attr.path().is_ident(m)) {
      if method.is_some() {
        return syn::Error::new_spanned(
          &attr,
          "only one HTTP method attribute (#[get]/#[post]/etc.) is allowed per handler",
        )
        .to_compile_error()
        .into();
      }
      let path: LitStr = match attr.parse_args() {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
      };
      method = Some((name, path));
    } else if attr.path().is_ident("layer") {
      let expr: Expr = match attr.parse_args() {
        Ok(e) => e,
        Err(e) => return e.to_compile_error().into(),
      };
      layers.push(expr);
    } else {
      remaining_attrs.push(attr);
    }
  }
  item_fn.attrs = remaining_attrs;

  let Some((method_name, path)) = method else {
    return syn::Error::new_spanned(
      &item_fn.sig,
      "#[layer(...)] requires exactly one #[get]/#[post]/#[put]/#[delete]/#[patch]/#[head]/\
       #[options]/#[trace]/#[connect] attribute on the same function",
    )
    .to_compile_error()
    .into();
  };

  let fn_name = &item_fn.sig.ident;
  let method_ident = syn::Ident::new(method_name, proc_macro2::Span::call_site());

  let layer_calls = layers.iter().map(|expr| {
    if matches!(expr, Expr::Path(_)) {
      quote! { .layer(#ax::LayerFactoryProbe::new(#expr).call(ctx)) }
    } else {
      quote! { .layer(#expr) }
    }
  });

  let registration = quote! {
    #ax::inventory::submit! {
      #ax::RouteRegistration(|router: ::axum::Router<#ax::Components>, ctx: &#ax::Components| {
        router.route(#path, ::axum::routing::#method_ident(#fn_name) #(#layer_calls)*)
      })
    }
  };

  quote! {
    #item_fn
    #registration
  }
  .into()
}
