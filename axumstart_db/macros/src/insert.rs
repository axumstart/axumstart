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

/// Struct-level `#[key(col)]` — names the column `__sqlx_insert_into`/`__sqlx_upsert_into`/
/// `__sqlx_insert_all_into` select back on after a write, on backends with no RETURNING
/// (MySQL). Defaults to `id` if omitted. Has no effect on Postgres/SQLite, which use
/// `RETURNING *` directly and never need this.
fn extract_key_col(input: &DeriveInput) -> syn::Result<String> {
    for attr in &input.attrs {
        if attr.path().is_ident("key") {
            let ident: syn::Ident = attr.parse_args()?;
            return Ok(ident.to_string());
        }
    }
    Ok("id".to_string())
}

fn expand_inner(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "SqlxInsert requires named struct fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "SqlxInsert can only be derived for structs",
            ))
        }
    };

    let field_names: Vec<_> = fields
        .iter()
        .map(|f| f.ident.as_ref().unwrap())
        .collect();

    if field_names.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "SqlxInsert requires at least one field",
        ));
    }

    if dialect::is_mysql() {
        mysql_impl(name, &field_names, &extract_key_col(input)?)
    } else {
        returning_impl(name, &field_names)
    }
}

/// Postgres/SQLite: `RETURNING *` gets the written row back in one round-trip.
fn returning_impl(name: &syn::Ident, field_names: &[&syn::Ident]) -> syn::Result<TokenStream2> {
    let cols = field_names.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(", ");
    let placeholders =
        (1..=field_names.len()).map(dialect::placeholder).collect::<Vec<_>>().join(", ");

    let sql_template = format!("INSERT INTO \"{{}}\" ({cols}) VALUES ({placeholders}) RETURNING *");
    let void_sql_template = format!("INSERT INTO \"{{}}\" ({cols}) VALUES ({placeholders})");
    let ignore_sql_template =
        format!("INSERT INTO \"{{}}\" ({cols}) VALUES ({placeholders}) ON CONFLICT DO NOTHING");
    let insert_all_prefix_template = format!("INSERT INTO \"{{}}\" ({cols}) ");

    let bind_calls: Vec<TokenStream2> = field_names.iter().map(|f| quote!(.bind(self.#f))).collect();
    let push_bind_calls: Vec<TokenStream2> =
        field_names.iter().map(|f| quote!(__b.push_bind(__row.#f);)).collect();
    let col_strs: Vec<String> = field_names.iter().map(|f| f.to_string()).collect();
    let upsert_placeholders = placeholders;

    Ok(quote! {
        impl #name {
            // Accepts any sqlx Executor (DbPool, &mut DbConnection, etc.)
            pub async fn __sqlx_insert_into<'e, E, Row>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL
                    .get_or_init(|| ::std::format!(#sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query_as::<_, Row>(sql)
                    #(#bind_calls)*
                    .fetch_one(executor)
                    .await
            }

            // Same INSERT, no RETURNING — for callers that don't want the row back.
            pub async fn __sqlx_insert_into_void<'e, E>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                static __SQL_VOID: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_VOID
                    .get_or_init(|| ::std::format!(#void_sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query(sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await
                    .map(|_| ())
            }

            // INSERT ... ON CONFLICT DO NOTHING — returns () on success
            pub async fn __sqlx_insert_ignore_into<'e, E>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                static __SQL_IGNORE: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_IGNORE
                    .get_or_init(|| ::std::format!(#ignore_sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query(sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await
                    .map(|_| ())
            }

            // Single multi-row INSERT ... RETURNING *. One round-trip for the whole batch.
            pub async fn __sqlx_insert_all_into<'e, E, Row>(
                rows: ::std::vec::Vec<Self>,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<::std::vec::Vec<Row>>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                if rows.is_empty() {
                    return ::std::result::Result::Ok(::std::vec::Vec::new());
                }
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(
                    ::std::format!(#insert_all_prefix_template, table),
                );
                __qb.push_values(rows, |mut __b, __row| {
                    #(#push_bind_calls)*
                });
                __qb.push(" RETURNING *");
                __qb.build_query_as::<Row>().fetch_all(executor).await
            }

            // Same multi-row INSERT, no RETURNING — for callers that don't want the rows back.
            pub async fn __sqlx_insert_all_into_void<'e, E>(
                rows: ::std::vec::Vec<Self>,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                if rows.is_empty() {
                    return ::std::result::Result::Ok(());
                }
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(
                    ::std::format!(#insert_all_prefix_template, table),
                );
                __qb.push_values(rows, |mut __b, __row| {
                    #(#push_bind_calls)*
                });
                __qb.build().execute(executor).await.map(|_| ())
            }

            // INSERT ... ON CONFLICT (conflict_col) DO UPDATE SET all_other_cols = EXCLUDED.col RETURNING *
            pub async fn __sqlx_upsert_into<'e, E, Row>(
                self,
                table: &'static str,
                conflict_col: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __SQL_UPSERT: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_UPSERT
                    .get_or_init(|| {
                        static __COLS: &[&str] = &[#(#col_strs),*];
                        let col_list = __COLS.join(", ");
                        let set_clause = __COLS
                            .iter()
                            .filter(|&&c| c != conflict_col)
                            .map(|c| ::std::format!("{c} = EXCLUDED.{c}"))
                            .collect::<::std::vec::Vec<_>>()
                            .join(", ");
                        ::std::format!(
                            "INSERT INTO \"{table}\" ({col_list}) VALUES ({}) ON CONFLICT ({conflict_col}) DO UPDATE SET {set_clause} RETURNING *",
                            #upsert_placeholders,
                        )
                        .into_boxed_str()
                    })
                    .as_ref();
                ::sqlx::query_as::<_, Row>(sql)
                    #(#bind_calls)*
                    .fetch_one(executor)
                    .await
            }
        }
    })
}

/// MySQL: no RETURNING. Every write is followed by a `SELECT ... WHERE {key} = ?` on the
/// same executor to hand back the row, so `E` must be `Copy` (references are — this rules
/// out combining these methods with `#[transactional]`'s `&mut DbConnection`, which isn't
/// `Copy`; only pool-based calls work for insert/insert_all/upsert on MySQL).
fn mysql_impl(name: &syn::Ident, field_names: &[&syn::Ident], key_col: &str) -> syn::Result<TokenStream2> {
    let cols = field_names.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(", ");
    let placeholders = (1..=field_names.len()).map(dialect::placeholder).collect::<Vec<_>>().join(", ");

    let insert_sql_template = format!("INSERT INTO `{{}}` ({cols}) VALUES ({placeholders})");
    let ignore_sql_template =
        format!("INSERT IGNORE INTO `{{}}` ({cols}) VALUES ({placeholders})");
    let insert_all_prefix_template = format!("INSERT INTO `{{}}` ({cols}) ");
    let select_by_key_template = format!("SELECT * FROM `{{}}` WHERE {key_col} = ?");
    let select_range_template =
        format!("SELECT * FROM `{{}}` WHERE {key_col} >= ? AND {key_col} < ? ORDER BY {key_col}");

    let bind_calls: Vec<TokenStream2> = field_names.iter().map(|f| quote!(.bind(self.#f))).collect();
    let push_bind_calls: Vec<TokenStream2> =
        field_names.iter().map(|f| quote!(__b.push_bind(__row.#f);)).collect();
    let col_strs: Vec<String> = field_names.iter().map(|f| f.to_string()).collect();
    // The INSERT statement's bind_calls move every field out of `self`, so the upsert's
    // follow-up SELECT (which binds whichever field matches the runtime `conflict_col`)
    // needs its own pre-`clone()`d copy of each field to bind from instead.
    let clone_idents: Vec<syn::Ident> = (0..field_names.len())
        .map(|i| quote::format_ident!("__clone_{i}"))
        .collect();
    let clone_stmts: Vec<TokenStream2> = field_names
        .iter()
        .zip(&clone_idents)
        .map(|(f, c)| quote!(let #c = self.#f.clone();))
        .collect();

    Ok(quote! {
        impl #name {
            pub async fn __sqlx_insert_into<'e, E, Row>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db> + ::std::marker::Copy,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __INSERT_SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let insert_sql: &'static str = __INSERT_SQL
                    .get_or_init(|| ::std::format!(#insert_sql_template, table).into_boxed_str())
                    .as_ref();
                static __SELECT_SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let select_sql: &'static str = __SELECT_SQL
                    .get_or_init(|| ::std::format!(#select_by_key_template, table).into_boxed_str())
                    .as_ref();

                let __id = ::sqlx::query(insert_sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await?
                    .last_insert_id();
                ::sqlx::query_as::<_, Row>(select_sql)
                    .bind(__id)
                    .fetch_one(executor)
                    .await
            }

            // Same INSERT, no follow-up SELECT — for callers that don't want the row back.
            // Unlike the Row-returning version above, this doesn't need `E: Copy` (only one
            // statement), so it also works with `#[transactional]`'s `&mut DbConnection`.
            pub async fn __sqlx_insert_into_void<'e, E>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                static __INSERT_SQL_VOID: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let insert_sql: &'static str = __INSERT_SQL_VOID
                    .get_or_init(|| ::std::format!(#insert_sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query(insert_sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await
                    .map(|_| ())
            }

            // INSERT IGNORE — single statement, no follow-up SELECT needed.
            pub async fn __sqlx_insert_ignore_into<'e, E>(
                self,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                static __SQL_IGNORE: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_IGNORE
                    .get_or_init(|| ::std::format!(#ignore_sql_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query(sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await
                    .map(|_| ())
            }

            // Multi-row INSERT, then re-SELECT the batch by primary key range. Relies on
            // MySQL's contiguous auto-increment allocation for a single multi-row insert
            // statement (true under the default innodb_autoinc_lock_mode) — if the table
            // doesn't use an auto-increment `{key_col}`, this will not return the right rows.
            pub async fn __sqlx_insert_all_into<'e, E, Row>(
                rows: ::std::vec::Vec<Self>,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<::std::vec::Vec<Row>>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db> + ::std::marker::Copy,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                if rows.is_empty() {
                    return ::std::result::Result::Ok(::std::vec::Vec::new());
                }
                let __count = rows.len() as u64;
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(
                    ::std::format!(#insert_all_prefix_template, table),
                );
                __qb.push_values(rows, |mut __b, __row| {
                    #(#push_bind_calls)*
                });
                let __first_id = __qb.build().execute(executor).await?.last_insert_id();

                static __SELECT_RANGE_SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let select_sql: &'static str = __SELECT_RANGE_SQL
                    .get_or_init(|| ::std::format!(#select_range_template, table).into_boxed_str())
                    .as_ref();
                ::sqlx::query_as::<_, Row>(select_sql)
                    .bind(__first_id)
                    .bind(__first_id + __count)
                    .fetch_all(executor)
                    .await
            }

            // Same multi-row INSERT, no follow-up SELECT — for callers that don't want the
            // rows back. Unlike the Row-returning version above, this doesn't need `E: Copy`
            // (only one statement), so it also works with `#[transactional]`.
            pub async fn __sqlx_insert_all_into_void<'e, E>(
                rows: ::std::vec::Vec<Self>,
                table: &'static str,
                executor: E,
            ) -> ::sqlx::Result<()>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db>,
            {
                if rows.is_empty() {
                    return ::std::result::Result::Ok(());
                }
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(
                    ::std::format!(#insert_all_prefix_template, table),
                );
                __qb.push_values(rows, |mut __b, __row| {
                    #(#push_bind_calls)*
                });
                __qb.build().execute(executor).await.map(|_| ())
            }

            // INSERT ... ON DUPLICATE KEY UPDATE col = VALUES(col), ... then SELECT-back by
            // the conflict column (MySQL has no RETURNING and infers the conflicting unique
            // key automatically — it isn't named the way Postgres/SQLite's ON CONFLICT is).
            pub async fn __sqlx_upsert_into<'e, E, Row>(
                self,
                table: &'static str,
                conflict_col: &'static str,
                executor: E,
            ) -> ::sqlx::Result<Row>
            where
                E: ::sqlx::Executor<'e, Database = ::axumstart_db::Db> + ::std::marker::Copy,
                Row: for<'r> ::sqlx::FromRow<'r, <::axumstart_db::Db as ::sqlx::Database>::Row> + Send + Unpin,
            {
                static __SQL_UPSERT: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let sql: &'static str = __SQL_UPSERT
                    .get_or_init(|| {
                        static __COLS: &[&str] = &[#(#col_strs),*];
                        let col_list = __COLS.join(", ");
                        let placeholders = (1..=__COLS.len())
                            .map(|_| "?")
                            .collect::<::std::vec::Vec<_>>()
                            .join(", ");
                        let set_clause = __COLS
                            .iter()
                            .map(|c| ::std::format!("{c} = VALUES({c})"))
                            .collect::<::std::vec::Vec<_>>()
                            .join(", ");
                        ::std::format!(
                            "INSERT INTO `{table}` ({col_list}) VALUES ({placeholders}) ON DUPLICATE KEY UPDATE {set_clause}"
                        )
                        .into_boxed_str()
                    })
                    .as_ref();
                static __SELECT_SQL: ::std::sync::OnceLock<::std::boxed::Box<str>> =
                    ::std::sync::OnceLock::new();
                let select_sql: &'static str = __SELECT_SQL
                    .get_or_init(|| ::std::format!("SELECT * FROM `{}` WHERE {} = ?", table, conflict_col).into_boxed_str())
                    .as_ref();

                #(#clone_stmts)*
                ::sqlx::query(sql)
                    #(#bind_calls)*
                    .execute(executor)
                    .await?;

                match conflict_col {
                    #( #col_strs => return ::sqlx::query_as::<_, Row>(select_sql).bind(#clone_idents).fetch_one(executor).await, )*
                    other => ::std::panic!("__sqlx_upsert_into: `{}` is not a field of this values struct", other),
                }
            }
        }
    })
}
