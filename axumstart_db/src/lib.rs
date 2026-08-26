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

/// Best-effort Monday-start week boundary, one per backend. Exact edge behavior around
/// the boundary (timezone handling, `_this_week` inclusivity) is not guaranteed to be
/// bit-identical across backends — pick one backend per deployment and rely on that.
#[cfg(feature = "postgres")]
pub const WEEK_START_SQL: &str = "date_trunc('week', CURRENT_TIMESTAMP)";
#[cfg(feature = "mysql")]
pub const WEEK_START_SQL: &str = "DATE_SUB(CURRENT_DATE, INTERVAL WEEKDAY(CURRENT_DATE) DAY)";
#[cfg(feature = "sqlite")]
pub const WEEK_START_SQL: &str = "date(CURRENT_TIMESTAMP, 'weekday 1', '-7 days')";

/// LIMIT/OFFSET pagination. The `#[repository]` macro detects a `Page`-typed
/// parameter on `find_all*` methods and appends `LIMIT $n OFFSET $m`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
}

pub trait Repository: Send + Sync {
    fn pool(&self) -> &DbPool;
}

impl Page {
    pub fn new(limit: i64, offset: i64) -> Self {
        Self { limit, offset }
    }

    /// Zero-based page number with fixed page size.
    pub fn number(page: i64, size: i64) -> Self {
        Self { limit: size, offset: page * size }
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
    {
        let mut sep = qb.separated(", ");
        for item in items {
            sep.push_bind(item);
        }
    }
    qb.push(")");
}
