//! Runtime support types for the `axumstart_db_macros` proc-macro crate.
//!
//! Exactly one of the `postgres` / `mysql` / `sqlite` features must be enabled — the
//! target backend is a compile-time choice, since SQL placeholder syntax and RETURNING
//! availability are baked into generated queries at macro-expansion time.

pub use axumstart_db_macros::{SqlxInsert, SqlxUpdate, repository};

#[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
compile_error!(
  "axumstart_db requires exactly one of the `postgres`, `mysql`, or `sqlite` features to be enabled"
);

#[cfg(any(
  all(feature = "postgres", feature = "mysql"),
  all(feature = "postgres", feature = "sqlite"),
  all(feature = "mysql", feature = "sqlite"),
))]
compile_error!(
  "axumstart_db: enable exactly one of `postgres`, `mysql`, `sqlite` — not more than one"
);

#[cfg(feature = "postgres")]
pub type Db = sqlx::Postgres;
#[cfg(feature = "mysql")]
pub type Db = sqlx::MySql;
#[cfg(feature = "sqlite")]
pub type Db = sqlx::Sqlite;

pub type DbPool = sqlx::Pool<Db>;
pub type DbConnection = <Db as sqlx::Database>::Connection;

/// Quotes a runtime SQL identifier for the selected backend.
pub fn quote_identifier(identifier: &str) -> String {
  if cfg!(feature = "mysql") {
    format!("`{}`", identifier.replace('`', "``"))
  } else {
    format!("\"{}\"", identifier.replace('"', "\"\""))
  }
}

/// Best-effort Monday-start week boundary, one per backend. Exact edge behavior around
/// the boundary (timezone handling, `_this_week` inclusivity) is not guaranteed to be
/// bit-identical across backends — pick one backend per deployment and rely on that.
#[cfg(feature = "postgres")]
pub const WEEK_START_SQL: &str = "date_trunc('week', CURRENT_TIMESTAMP)";
#[cfg(feature = "mysql")]
pub const WEEK_START_SQL: &str = "DATE_SUB(CURRENT_DATE, INTERVAL WEEKDAY(CURRENT_DATE) DAY)";
#[cfg(feature = "sqlite")]
pub const WEEK_START_SQL: &str =
  "date(CURRENT_TIMESTAMP, '-' || ((strftime('%w', CURRENT_TIMESTAMP) + 6) % 7) || ' days')";

/// LIMIT/OFFSET pagination. The `#[repository]` macro detects a `Page`-typed
/// parameter on `find_all*` methods and appends `LIMIT $n OFFSET $m`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
  pub limit: i64,
  pub offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
  NegativeLimit,
  NegativeOffset,
  Overflow,
}

impl std::fmt::Display for PageError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(match self {
      Self::NegativeLimit => "page limit must be non-negative",
      Self::NegativeOffset => "page offset must be non-negative",
      Self::Overflow => "page offset overflowed",
    })
  }
}

impl std::error::Error for PageError {}

pub trait Repository: Send + Sync {
  fn pool(&self) -> &DbPool;
}

impl Page {
  /// Unchecked constructor retained for compatibility; prefer `try_new` for user input.
  pub fn new(limit: i64, offset: i64) -> Self {
    Self { limit, offset }
  }

  /// Zero-based page number with fixed page size.
  /// Panics for invalid values or overflow; prefer `try_number` for user input.
  pub fn number(page: i64, size: i64) -> Self {
    Self::try_number(page, size).expect("invalid page number or size")
  }

  pub fn try_new(limit: i64, offset: i64) -> Result<Self, PageError> {
    if limit < 0 {
      return Err(PageError::NegativeLimit);
    }
    if offset < 0 {
      return Err(PageError::NegativeOffset);
    }
    Ok(Self { limit, offset })
  }

  pub fn try_number(page: i64, size: i64) -> Result<Self, PageError> {
    if page < 0 || size < 0 {
      return Err(if size < 0 {
        PageError::NegativeLimit
      } else {
        PageError::NegativeOffset
      });
    }
    let offset = page.checked_mul(size).ok_or(PageError::Overflow)?;
    Ok(Self {
      limit: size,
      offset,
    })
  }

  pub fn validate(self) -> Result<(), PageError> {
    Self::try_new(self.limit, self.offset).map(|_| ())
  }
}

/// Appends a parenthesized, dialect-correct `IN (...)` value list to `qb` via bound
/// placeholders — used by the `#[repository]` macro's codegen for the `_in` DSL suffix
/// so the same code path works across Postgres/MySQL/SQLite (none of which let a single
/// placeholder bind a `Vec<T>` the way Postgres's `= ANY($n)` does).
pub fn push_in_list<'t, T>(qb: &mut sqlx::QueryBuilder<Db>, items: impl IntoIterator<Item = T>)
where
  T: sqlx::Encode<'t, Db> + sqlx::Type<Db>,
{
  qb.push("(");
  let mut sep = qb.separated(", ");
  let mut has_items = false;
  for item in items {
    has_items = true;
    sep.push_bind(item);
  }
  if !has_items {
    qb.push("NULL");
  }
  qb.push(")");
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pagination_rejects_invalid_values() {
    assert_eq!(Page::try_new(-1, 0), Err(PageError::NegativeLimit));
    assert_eq!(Page::try_new(1, -1), Err(PageError::NegativeOffset));
    assert_eq!(Page::try_number(-1, 5), Err(PageError::NegativeOffset));
    assert_eq!(Page::try_number(1, -1), Err(PageError::NegativeLimit));
    assert_eq!(Page::try_number(i64::MAX, 2), Err(PageError::Overflow));
    assert_eq!(Page::try_number(2, 5), Ok(Page::new(5, 10)));
    assert_eq!(Page::try_new(0, 0), Ok(Page::new(0, 0)));
  }

  #[test]
  fn identifier_quotes_are_escaped() {
    #[cfg(feature = "mysql")]
    assert_eq!(quote_identifier("a`b"), "`a``b`");
    #[cfg(not(feature = "mysql"))]
    assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
  }
}
