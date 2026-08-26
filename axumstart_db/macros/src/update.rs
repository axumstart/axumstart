use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

use crate::dialect;

pub fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_inner(&input) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(e) => TokenStream::from(e.to_compile_error()),
    }
}

fn expand_inner(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "SqlxUpdate requires named struct fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "SqlxUpdate can only be derived for structs",
            ))
        }
    };

    let key_fields: Vec<_> = fields
        .iter()
        .filter(|f| f.attrs.iter().any(|a| a.path().is_ident("key")))
        .collect();
    let key_field = match key_fields.as_slice() {
        [one] => *one,
        [] => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "SqlxUpdate requires exactly one field marked with #[key]",
            ))
        }
        [_, second, ..] => {
            return Err(syn::Error::new_spanned(
                second.ident.as_ref().unwrap(),
                "SqlxUpdate allows only one #[key] field",
            ))
        }
    };
    let key_name = key_field.ident.as_ref().unwrap();
    let key_str = key_name.to_string();

    let non_key_fields: Vec<_> = fields
        .iter()
        .filter(|f| f.ident.as_ref().unwrap() != key_name)
        .map(|f| f.ident.as_ref().unwrap())
        .collect();

    if non_key_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "SqlxUpdate requires at least one non-key field to update",
        ));
    }

    if dialect::is_mysql() {
        mysql_impl(name, key_name, &key_str, &non_key_fields)
    } else {
        returning_impl(name, key_name, &key_str, &non_key_fields)
    }
}

/// Postgres/SQLite: `RETURNING *` gets the updated row back in one round-trip.
fn returning_impl(
    name: &syn::Ident,
    key_name: &syn::Ident,
    key_str: &str,
    non_key_fields: &[&syn::Ident],
) -> syn::Result<TokenStream2> {
    let set_clause = non_key_fields
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{col} = {}", dialect::placeholder(i + 1)))
        .collect::<Vec<_>>()
        .join(", ");

    let where_idx = dialect::placeholder(non_key_fields.len() + 1);
    let sql_template = format!("UPDATE \"{{}}\" SET {set_clause} WHERE {key_str} = {where_idx} RETURNING *");

    let bind_calls: Vec<TokenStream2> = non_key_fields
        .iter()
        .map(|f| quote!(.bind(self.#f)))
        .chain(std::iter::once(quote!(.bind(self.#key_name))))
        .collect();

    Ok(quote! {
        impl #name {
            pub async fn __sqlx_update_in<'e, E, Row>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __SQL_UPDATE: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_UPDATE
                    .get_or_init(|| ::std::format!(#sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query_as::<_, Row>(sql)
                    #(#bind_calls)*
                    .fetch_one(executor)
                    .await
            }
        }
    })
}

/// MySQL: no RETURNING — `UPDATE ... WHERE key = ?` then `SELECT ... WHERE key = ?`. The
/// key's value has to be cloned before the UPDATE's bind chain moves it out of `self`.
/// `E` must be `Copy` (references are — this rules out `#[transactional]`'s
/// `&mut DbConnection`, which isn't `Copy`; only pool-based calls work here on MySQL).
fn mysql_impl(
    name: &syn::Ident,
    key_name: &syn::Ident,
    key_str: &str,
    non_key_fields: &[&syn::Ident],
) -> syn::Result<TokenStream2> {
    let set_clause = non_key_fields
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{col} = {}", dialect::placeholder(i + 1)))
        .collect::<Vec<_>>()
        .join(", ");

    let update_sql_template = format!("UPDATE `{{}}` SET {set_clause} WHERE {key_str} = ?");
    let select_sql_template = format!("SELECT * FROM `{{}}` WHERE {key_str} = ?");

    let bind_calls: Vec<TokenStream2> = non_key_fields.iter().map(|f| quote!(.bind(self.#f))).collect();

    Ok(quote! {
        impl #name {
            pub async fn __sqlx_update_in<'e, E, Row>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db> + ::std::marker::Copy,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __SQL_UPDATE: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let update_sql: &'static str = __SQL_UPDATE
                    .get_or_init(|| ::std::format!(#update_sql_template, table).into_boxed_str())
                    .as_ref();
                static __SELECT_SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let select_sql: &'static str = __SELECT_SQL
                    .get_or_init(|| ::std::format!(#select_sql_template, table).into_boxed_str())
                    .as_ref();

                let __key = self.#key_name.clone();
                ::sqlx::query(update_sql)
                    #(#bind_calls)*
                    .bind(__key.clone())
                    .execute(executor)
                    .await?;
                ::sqlx::query_as::<_, Row>(select_sql)
                    .bind(__key)
                    .fetch_one(executor)
                    .await
            }
        }
    })
}
