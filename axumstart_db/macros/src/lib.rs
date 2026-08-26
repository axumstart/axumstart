use proc_macro::TokenStream;

mod dialect;
mod dsl;
mod insert;
mod repository;
mod update;

#[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
compile_error!(
    "axumstart_db_macros requires exactly one of the `postgres`, `mysql`, or `sqlite` features to be enabled"
);

#[cfg(any(
    all(feature = "postgres", feature = "mysql"),
    all(feature = "postgres", feature = "sqlite"),
    all(feature = "mysql", feature = "sqlite"),
))]
compile_error!(
    "axumstart_db_macros: enable exactly one of `postgres`, `mysql`, `sqlite` — not more than one"
);

#[proc_macro_attribute]
pub fn repository(attr: TokenStream, item: TokenStream) -> TokenStream {
    repository::expand(attr, item)
}

#[proc_macro_derive(SqlxInsert, attributes(key))]
pub fn derive_sqlx_insert(input: TokenStream) -> TokenStream {
    insert::expand(input)
}

#[proc_macro_derive(SqlxUpdate, attributes(key))]
pub fn derive_sqlx_update(input: TokenStream) -> TokenStream {
    update::expand(input)
}
