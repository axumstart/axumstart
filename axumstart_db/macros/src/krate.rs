//! Resolves the token path generated code should use to reach `axumstart_db`'s and
//! `axumstart_components`'s public API — through the `axumstart` meta-crate re-export
//! (`axumstart::db`, `axumstart::components`) if the consumer depends on that, or directly
//! otherwise. Checked at macro-expansion time against the *consuming* crate's `Cargo.toml`.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;

pub fn db_path() -> TokenStream {
  resolve("db", "axumstart_db")
}

pub fn components_path() -> TokenStream {
  resolve("components", "axumstart_components")
}

fn resolve(submodule: &str, direct_crate: &str) -> TokenStream {
  if let Ok(FoundCrate::Name(name)) = crate_name("axumstart") {
    let meta = Ident::new(&name, Span::call_site());
    let sub = Ident::new(submodule, Span::call_site());
    return quote!(::#meta::#sub);
  }

  match crate_name(direct_crate) {
    Ok(FoundCrate::Itself) => quote!(crate),
    Ok(FoundCrate::Name(name)) => {
      let ident = Ident::new(&name, Span::call_site());
      quote!(::#ident)
    }
    Err(_) => {
      let ident = Ident::new(direct_crate, Span::call_site());
      quote!(::#ident)
    }
  }
}
