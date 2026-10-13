use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

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
        ));
      }
    },
    _ => {
      return Err(syn::Error::new_spanned(
        &input.ident,
        "SqlxInsert can only be derived for structs",
      ));
    }
  };

  let field_names: Vec<_> = fields.iter().map(|f| f.ident.as_ref().unwrap()).collect();

  if field_names.is_empty() {
    return Err(syn::Error::new_spanned(
      &input.ident,
      "SqlxInsert requires at least one field",
    ));
  }

  if dialect::is_mysql() {
    let key_col = extract_key_col(input)?;
    let key_field = field_names
      .iter()
      .find(|field| field.to_string() == key_col)
      .copied();
    if key_col != "id" && key_field.is_none() {
      return Err(syn::Error::new_spanned(
        input,
        format!("MySQL `#[key({key_col})]` requires a matching field on the values struct"),
      ));
    }
    mysql_impl(name, &field_names, &key_col, key_field)
  } else {
    returning_impl(name, &field_names)
  }
}

/// Postgres/SQLite: `RETURNING *` gets the written row back in one round-trip.
fn returning_impl(name: &syn::Ident, field_names: &[&syn::Ident]) -> syn::Result<TokenStream2> {
  let db = crate::krate::db_path();
  let cols = field_names
    .iter()
    .map(|f| dialect::quote_ident(&f.to_string()))
    .collect::<Vec<_>>()
    .join(", ");
  let placeholders = (1..=field_names.len())
    .map(dialect::placeholder)
    .collect::<Vec<_>>()
    .join(", ");

  let sql_template = format!("INSERT INTO {{}} ({cols}) VALUES ({placeholders}) RETURNING *");
  let void_sql_template = format!("INSERT INTO {{}} ({cols}) VALUES ({placeholders})");
  let ignore_sql_template =
    format!("INSERT INTO {{}} ({cols}) VALUES ({placeholders}) ON CONFLICT DO NOTHING");
  let insert_all_prefix_template = format!("INSERT INTO {{}} ({cols}) ");

  let bind_calls: Vec<TokenStream2> = field_names.iter().map(|f| quote!(.bind(self.#f))).collect();
  let push_bind_calls: Vec<TokenStream2> = field_names
    .iter()
    .map(|f| quote!(__b.push_bind(__row.#f);))
    .collect();
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              let sql = ::std::format!(#sql_template, #db::quote_identifier(table));
              ::sqlx::query_as::<_, Row>(::sqlx::AssertSqlSafe(sql))
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              let sql = ::std::format!(#void_sql_template, #db::quote_identifier(table));
              ::sqlx::query(::sqlx::AssertSqlSafe(sql))
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              let sql = ::std::format!(#ignore_sql_template, #db::quote_identifier(table));
              ::sqlx::query(::sqlx::AssertSqlSafe(sql))
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(::std::vec::Vec::new());
              }
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(());
              }
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              static __COLS: &[&str] = &[#(#col_strs),*];
              let col_list = __COLS.iter().map(|c| #db::quote_identifier(c)).collect::<::std::vec::Vec<_>>().join(", ");
              let set_clause = __COLS.iter().filter(|&&c| c != conflict_col)
                  .map(|c| ::std::format!("{} = EXCLUDED.{}", #db::quote_identifier(c), #db::quote_identifier(c)))
                  .collect::<::std::vec::Vec<_>>().join(", ");
              let sql = ::std::format!(
                  "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING *",
                  #db::quote_identifier(table), col_list, #upsert_placeholders,
                  #db::quote_identifier(conflict_col), set_clause,
              );
              ::sqlx::query_as::<_, Row>(::sqlx::AssertSqlSafe(sql))
                  #(#bind_calls)*
                  .fetch_one(executor)
                  .await
          }

          // Batch INSERT ... ON CONFLICT (conflict_col) DO UPDATE ... RETURNING *. The row
          // count varies per call. Identifiers and the conflict clause are rebuilt per
          // call so values types can safely be reused across repositories.
          pub async fn __sqlx_upsert_all_into<'e, E, Row>(
              rows: ::std::vec::Vec<Self>,
              table: &'static str,
              conflict_col: &'static str,
              executor: E,
          ) -> ::sqlx::Result<::std::vec::Vec<Row>>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db>,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(::std::vec::Vec::new());
              }
              static __COLS: &[&str] = &[#(#col_strs),*];
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
              );
              __qb.push_values(rows, |mut __b, __row| {
                  #(#push_bind_calls)*
              });
              let __set_clause = __COLS
                  .iter()
                  .filter(|&&c| c != conflict_col)
                  .map(|c| ::std::format!("{} = EXCLUDED.{}", #db::quote_identifier(c), #db::quote_identifier(c)))
                  .collect::<::std::vec::Vec<_>>()
                  .join(", ");
              __qb.push(::std::format!(
                  " ON CONFLICT ({}) DO UPDATE SET {__set_clause} RETURNING *",
                  #db::quote_identifier(conflict_col),
              ));
              __qb.build_query_as::<Row>().fetch_all(executor).await
          }

          // Same batch upsert, no RETURNING — for callers that don't want the rows back.
          pub async fn __sqlx_upsert_all_into_void<'e, E>(
              rows: ::std::vec::Vec<Self>,
              table: &'static str,
              conflict_col: &'static str,
              executor: E,
          ) -> ::sqlx::Result<()>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(());
              }
              static __COLS: &[&str] = &[#(#col_strs),*];
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
              );
              __qb.push_values(rows, |mut __b, __row| {
                  #(#push_bind_calls)*
              });
              let __set_clause = __COLS
                  .iter()
                  .filter(|&&c| c != conflict_col)
                  .map(|c| ::std::format!("{} = EXCLUDED.{}", #db::quote_identifier(c), #db::quote_identifier(c)))
                  .collect::<::std::vec::Vec<_>>()
                  .join(", ");
              __qb.push(::std::format!(" ON CONFLICT ({}) DO UPDATE SET {__set_clause}", #db::quote_identifier(conflict_col)));
              __qb.build().execute(executor).await.map(|_| ())
          }
      }
  })
}

/// MySQL: no RETURNING. Every write is followed by a `SELECT ... WHERE {key} = ?` on the
/// same executor to hand back the row, so `E` must be `Copy` (references are — this rules
/// out combining these methods with `#[transactional]`'s `&mut DbConnection`, which isn't
/// `Copy`; only pool-based calls work for insert/insert_all/upsert on MySQL).
fn mysql_impl(
  name: &syn::Ident,
  field_names: &[&syn::Ident],
  key_col: &str,
  key_field: Option<&syn::Ident>,
) -> syn::Result<TokenStream2> {
  let db = crate::krate::db_path();
  let cols = field_names
    .iter()
    .map(|f| dialect::quote_ident(&f.to_string()))
    .collect::<Vec<_>>()
    .join(", ");
  let placeholders = (1..=field_names.len())
    .map(dialect::placeholder)
    .collect::<Vec<_>>()
    .join(", ");

  let insert_sql_template = format!("INSERT INTO {{}} ({cols}) VALUES ({placeholders})");
  let ignore_sql_template = format!("INSERT IGNORE INTO {{}} ({cols}) VALUES ({placeholders})");
  let insert_all_prefix_template = format!("INSERT INTO {{}} ({cols}) ");
  let key = dialect::quote_ident(key_col);
  let select_by_key_template = format!("SELECT * FROM {{}} WHERE {key} = ?");
  let select_in_template = format!("SELECT * FROM {{}} WHERE {key} IN ");

  let bind_calls: Vec<TokenStream2> = field_names.iter().map(|f| quote!(.bind(self.#f))).collect();
  let push_bind_calls: Vec<TokenStream2> = field_names
    .iter()
    .map(|f| quote!(__b.push_bind(__row.#f);))
    .collect();
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
  // Batch upsert's ON DUPLICATE KEY UPDATE always sets every column (MySQL doesn't mind
  // re-setting the conflict column to its own value, unlike Postgres's EXCLUDED clause).
  let on_duplicate_suffix = format!(
    " ON DUPLICATE KEY UPDATE {}",
    col_strs
      .iter()
      .map(|c| format!(
        "{} = VALUES({})",
        dialect::quote_ident(c),
        dialect::quote_ident(c)
      ))
      .collect::<Vec<_>>()
      .join(", ")
  );
  // One fully-assembled match arm per field, built here rather than relying on lockstep
  // repetition inside `quote!` — each arm needs its own select-back template and field
  // access, so it's simpler to hand `quote!` one TokenStream2 per arm than to juggle
  // several parallel Vecs inside a single `#(...)*`.
  let upsert_all_arms: Vec<TokenStream2> = field_names
    .iter()
    .zip(&col_strs)
    .map(|(f, col)| {
      let select_template = format!("SELECT * FROM {{}} WHERE {} IN ", dialect::quote_ident(col));
      quote! {
          #col => {
              let __keys: ::std::vec::Vec<_> = rows.iter().map(|__row| __row.#f.clone()).collect();
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
              );
              __qb.push_values(rows, |mut __b, __row| {
                  #(#push_bind_calls)*
              });
              __qb.push(#on_duplicate_suffix);
              __qb.build().execute(executor).await?;

              let mut __sqb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#select_template, #db::quote_identifier(table)),
              );
              #db::push_in_list(&mut __sqb, __keys);
              __sqb.build_query_as::<Row>().fetch_all(executor).await
          }
      }
    })
    .collect();

  let single_insert = match key_field {
    Some(field) => quote! {
        let __key = self.#field.clone();
        ::sqlx::query(::sqlx::AssertSqlSafe(insert_sql))
            #(#bind_calls)*
            .execute(executor)
            .await?;
        ::sqlx::query_as::<_, Row>(::sqlx::AssertSqlSafe(select_sql))
            .bind(__key)
            .fetch_one(executor)
            .await
    },
    None => quote! {
        let __generated_key: bool = ::sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? AND column_name = ? AND extra LIKE '%auto_increment%')",
        )
        .bind(table)
        .bind(#key_col)
        .fetch_one(executor)
        .await?;
        if !__generated_key {
            return Err(::sqlx::Error::Protocol("row-returning MySQL insert requires a supplied key or an auto-increment key column".into()));
        }
        let __id = ::sqlx::query(::sqlx::AssertSqlSafe(insert_sql))
            #(#bind_calls)*
            .execute(executor)
            .await?
            .last_insert_id();
        if __id == 0 {
            return Err(::sqlx::Error::Protocol("row-returning MySQL insert requires a supplied key or an auto-increment id".into()));
        }
        ::sqlx::query_as::<_, Row>(::sqlx::AssertSqlSafe(select_sql))
            .bind(__id)
            .fetch_one(executor)
            .await
    },
  };

  let batch_insert = match key_field {
    Some(field) => quote! {
        let __keys: ::std::vec::Vec<_> = rows.iter().map(|__row| __row.#field.clone()).collect();
        let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
            ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
        );
        __qb.push_values(rows, |mut __b, __row| {
            #(#push_bind_calls)*
        });
        __qb.build().execute(executor).await?;
        let mut __sqb = ::sqlx::QueryBuilder::<#db::Db>::new(
            ::std::format!(#select_in_template, #db::quote_identifier(table)),
        );
        #db::push_in_list(&mut __sqb, __keys);
        __sqb.build_query_as::<Row>().fetch_all(executor).await
    },
    None => quote! {
        let _ = (table, executor);
        Err(::sqlx::Error::Protocol("row-returning MySQL insert_all requires the key field in the values struct; use the ()-returning form for generated keys".into()))
    },
  };

  Ok(quote! {
      impl #name {
          pub async fn __sqlx_insert_into<'e, E, Row>(
              self,
              table: &'static str,
              executor: E,
          ) -> ::sqlx::Result<Row>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db> + ::std::marker::Copy,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              let insert_sql = ::std::format!(#insert_sql_template, #db::quote_identifier(table));
              let select_sql = ::std::format!(#select_by_key_template, #db::quote_identifier(table));

              #single_insert
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              let insert_sql = ::std::format!(#insert_sql_template, #db::quote_identifier(table));
              ::sqlx::query(::sqlx::AssertSqlSafe(insert_sql))
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              let sql = ::std::format!(#ignore_sql_template, #db::quote_identifier(table));
              ::sqlx::query(::sqlx::AssertSqlSafe(sql))
                  #(#bind_calls)*
                  .execute(executor)
                  .await
                  .map(|_| ())
          }

          // Select back using supplied keys. Without keys, fail before writing rather
          // than guessing an auto-increment range (increments need not be contiguous).
          pub async fn __sqlx_insert_all_into<'e, E, Row>(
              rows: ::std::vec::Vec<Self>,
              table: &'static str,
              executor: E,
          ) -> ::sqlx::Result<::std::vec::Vec<Row>>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db> + ::std::marker::Copy,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(::std::vec::Vec::new());
              }
              #batch_insert
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
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(());
              }
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
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
              E: ::sqlx::Executor<'e, Database = #db::Db> + ::std::marker::Copy,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              static __COLS: &[&str] = &[#(#col_strs),*];
              let col_list = __COLS.iter().map(|c| #db::quote_identifier(c)).collect::<::std::vec::Vec<_>>().join(", ");
              let placeholders = (1..=__COLS.len()).map(|_| "?").collect::<::std::vec::Vec<_>>().join(", ");
              let set_clause = __COLS.iter()
                  .map(|c| ::std::format!("{} = VALUES({})", #db::quote_identifier(c), #db::quote_identifier(c)))
                  .collect::<::std::vec::Vec<_>>().join(", ");
              let sql = ::std::format!(
                  "INSERT INTO {} ({}) VALUES ({}) ON DUPLICATE KEY UPDATE {}",
                  #db::quote_identifier(table), col_list, placeholders, set_clause,
              );
              let select_sql = ::std::format!(
                  "SELECT * FROM {} WHERE {} = ?",
                  #db::quote_identifier(table), #db::quote_identifier(conflict_col),
              );

              #(#clone_stmts)*
              ::sqlx::query(::sqlx::AssertSqlSafe(sql))
                  #(#bind_calls)*
                  .execute(executor)
                  .await?;

              match conflict_col {
                  #( #col_strs => return ::sqlx::query_as::<_, Row>(::sqlx::AssertSqlSafe(select_sql)).bind(#clone_idents).fetch_one(executor).await, )*
                  other => ::std::panic!("__sqlx_upsert_into: `{}` is not a field of this values struct", other),
              }
          }

          // Batch INSERT ... ON DUPLICATE KEY UPDATE, then SELECT-back by
          // `{conflict_col} IN (...)`. Unlike `__sqlx_insert_all_into`'s auto-increment
          // range trick, this works for a mixed insert/update batch because the conflict
          // column's values are already known before the write (they're struct fields),
          // not DB-assigned — so the values to extract are collected up front, per the
          // (compile-time-known-per-arm) field matching the runtime `conflict_col`.
          pub async fn __sqlx_upsert_all_into<'e, E, Row>(
              rows: ::std::vec::Vec<Self>,
              table: &'static str,
              conflict_col: &'static str,
              executor: E,
          ) -> ::sqlx::Result<::std::vec::Vec<Row>>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db> + ::std::marker::Copy,
              Row: for<'r> ::sqlx::FromRow<'r, <#db::Db as ::sqlx::Database>::Row> + Send + Unpin,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(::std::vec::Vec::new());
              }
              match conflict_col {
                  #(#upsert_all_arms)*
                  other => ::std::panic!("__sqlx_upsert_all_into: `{}` is not a field of this values struct", other),
              }
          }

          // Same batch upsert, no SELECT-back — for callers that don't want the rows back.
          // Doesn't need `E: Copy` (a single statement), so it also works with `#[transactional]`.
          pub async fn __sqlx_upsert_all_into_void<'e, E>(
              rows: ::std::vec::Vec<Self>,
              table: &'static str,
              conflict_col: &'static str,
              executor: E,
          ) -> ::sqlx::Result<()>
          where
              E: ::sqlx::Executor<'e, Database = #db::Db>,
          {
              if rows.is_empty() {
                  return ::std::result::Result::Ok(());
              }
              static __COLS: &[&str] = &[#(#col_strs),*];
              if !__COLS.contains(&conflict_col) {
                  ::std::panic!("__sqlx_upsert_all_into_void: `{}` is not a field of this values struct", conflict_col);
              }
              let mut __qb = ::sqlx::QueryBuilder::<#db::Db>::new(
                  ::std::format!(#insert_all_prefix_template, #db::quote_identifier(table)),
              );
              __qb.push_values(rows, |mut __b, __row| {
                  #(#push_bind_calls)*
              });
              __qb.push(#on_duplicate_suffix);
              __qb.build().execute(executor).await.map(|_| ())
          }
      }
  })
}
