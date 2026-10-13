//! Compile-time backend selection. Exactly one of the crate's `postgres`/`mysql`/`sqlite`
//! features is enabled by the downstream consumer (mirrored automatically from
//! `axumstart_db`'s own features — see its `Cargo.toml`). These helpers decide the few
//! things that must be baked into generated SQL at macro-expansion time: placeholder
//! syntax, RETURNING availability, and a couple of dialect-specific SQL keywords.
//! Everything else (types, week-boundary SQL) is resolved at the call site via
//! `::axumstart_db::...` token paths instead, so this module stays small.

pub fn placeholder(idx: usize) -> String {
  if cfg!(feature = "postgres") {
    format!("${idx}")
  } else {
    "?".to_string()
  }
}

/// Quotes a SQL identifier for the selected backend.
pub fn quote_ident(identifier: &str) -> String {
  if cfg!(feature = "mysql") {
    format!("`{}`", identifier.replace('`', "``"))
  } else {
    format!("\"{}\"", identifier.replace('"', "\"\""))
  }
}

pub fn order_columns(table: &str, columns: &[String]) -> String {
  columns
    .iter()
    .map(|column| {
      let (column, direction) = column
        .strip_suffix(" DESC")
        .map(|c| (c, " DESC"))
        .or_else(|| column.strip_suffix(" ASC").map(|c| (c, " ASC")))
        .unwrap_or((column.as_str(), ""));
      format!("{}.{}{direction}", quote_ident(table), quote_ident(column))
    })
    .collect::<Vec<_>>()
    .join(", ")
}

/// Postgres and SQLite both support `RETURNING *` (including after `ON CONFLICT`).
/// MySQL has no RETURNING at all — callers emulate it with a follow-up SELECT
/// (see `insert.rs`/`update.rs`'s `is_mysql()` branches).
pub fn is_mysql() -> bool {
  cfg!(feature = "mysql")
}

pub fn random_order_sql() -> &'static str {
  if cfg!(feature = "mysql") {
    "ORDER BY RAND()"
  } else {
    "ORDER BY RANDOM()"
  }
}
