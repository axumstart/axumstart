---
sidebar_position: 1
---

# axumstart_db

Generates a repository implementation from a trait definition: annotate the trait, name your
methods following a small set of conventions, and the SQL (Postgres, MySQL, or SQLite — one
backend, chosen at compile time) gets written for you.

## Picking a backend

Exactly one of the `postgres` / `mysql` / `sqlite` Cargo features must be enabled. This is a
compile-time choice because SQL placeholder syntax and `RETURNING` availability are baked into
generated queries at macro-expansion time — enabling zero or more than one is a compile error.

```toml
axumstart_db = { path = "...", features = ["postgres"] }
```

This gives you `Db` (the `sqlx::Database` for the chosen backend), plus the aliases `DbPool =
sqlx::Pool<Db>` and `DbConnection = <Db as sqlx::Database>::Connection`.

## `#[repository(table = "...")]`

Annotate a trait to generate:
- the trait itself, wrapped with `#[async_trait]`,
- a `Db{TraitName}` struct with a `pool: DbPool` field and `new(pool: DbPool) -> Self`,
- `impl {TraitName} for Db{TraitName}`, with method bodies generated from method *names*.

```rust
#[repository(table = "user_stats")]
pub trait UserStatsRepository: Send + Sync {
    async fn find_by_user_id(&self, user_id: i32) -> sqlx::Result<Option<UserStatsRow>>;
    async fn insert(&self, values: UserStatsValues) -> sqlx::Result<UserStatsRow>;
    fn db(&self) -> &DbPool;
}

// Generated: DbUserStatsRepository { pool }  +  impl UserStatsRepository for DbUserStatsRepository
```

A common pattern is a type alias for callers: `pub type UserStatsRepositoryImpl =
DbUserStatsRepository;`.

### Method name conventions

| Method name pattern | Generated SQL |
|---|---|
| `find_by_{field}` | `SELECT * FROM "{table}" WHERE {field} = $1` |
| `find_by_{f1}_and_{f2}` | `... WHERE f1 = $1 AND f2 = $2` (any number of fields) |
| `find_all_by_{field}` | same, but always `fetch_all` |
| `find_random_by_{field}` | `... WHERE {field} = $1 ORDER BY RANDOM() LIMIT 1` |
| `exists_by_{field}` | `SELECT EXISTS(SELECT 1 FROM "{table}" WHERE {field} = $1)` |
| `delete_by_{field}` | `DELETE FROM "{table}" WHERE {field} = $1` |
| `insert` | delegates to `SqlxInsert::__sqlx_insert_into` ([values struct needs `#[derive(SqlxInsert)]`](#derivesqlxinsert)) |
| `insert_all` | single multi-row `INSERT` for a whole batch |
| `db` / `pool` | returns `&self.pool` |

**Fetch mode** for `find_by_*` / `find_random_by_*` is inferred from the return type:
`Option<T>` → `fetch_optional`, `Vec<T>` → `fetch_all`, anything else → `fetch_one`.

**`insert`/`insert_all` returning `()`** skip fetching the row back entirely — declare
`-> sqlx::Result<()>` instead of `-> sqlx::Result<Row>` and codegen emits a plain `INSERT`, no
`RETURNING` (Postgres/SQLite) and no follow-up `SELECT` (MySQL):

```rust
async fn insert(&self, values: EventValues) -> sqlx::Result<()>;
async fn insert_all(&self, values: Vec<EventValues>) -> sqlx::Result<()>;
```

Methods with a **default body** are emitted as-is (no codegen applied). **Unknown method
names** compile to `todo!("no codegen rule for `name`")` — a runtime panic, not a compile
error, so double-check new method names against this table.

### `join(table_name)` — cross-table field resolution

Declare related tables so `find_by_*` method names can reference their columns:

```rust
#[repository(table = "completed_challenge", join(challenge))]
pub trait CompletedChallengeRepository: Send + Sync {
    // challenge_uuid → JOIN "challenge" ON "challenge".id = "completed_challenge".challenge_id
    //                   WHERE "challenge".uuid = $1 AND user_id = $2
    async fn find_by_challenge_uuid_and_user_id(
        &self, challenge_uuid: Uuid, user_id: i32,
    ) -> sqlx::Result<Option<CompletedChallengeRow>>;
}
```

Convention: a `{join}_id` foreign key must exist on the main table. Multiple joins:
`join(challenge, user)`. Field resolution is `{join_name}_{col}` → `"{join_name}".{col}` (with
the `JOIN` added); every other field stays unqualified against the main table.

### `_in` — works the same on every backend

`find_by_id_in`, `delete_by_id_in`, `find_all_by_id_in`, etc. take a `Vec<T>` and render an
`IN (...)` list. This is built at runtime via `sqlx::QueryBuilder` (using the exported
`push_in_list` helper) rather than `= ANY($n)`, since neither MySQL nor SQLite let a single
placeholder bind a `Vec<T>` — so the same code path is used on all three backends, Postgres
included.

### Pagination — `Page`

`Page { limit: i64, offset: i64 }`, constructed with `Page::new(limit, offset)` or
`Page::number(page, size)` (zero-based page number, fixed page size). The `#[repository]` macro
detects a `Page`-typed parameter on `find_all*` methods and appends `LIMIT $n OFFSET $m`.

### `#[transactional]`

Marking a method `#[transactional]` generates a `{method}_tx` variant taking an extra
`conn: &mut DbConnection` instead of using `self.pool` — the original pool-based method is still
generated alongside it:

```rust
#[transactional]
async fn update(&self, stats: UserStatsRow) -> sqlx::Result<UserStatsRow>;
// generates both:
//   update(&self, stats)                          — uses the pool
//   update_tx(&self, conn: &mut DbConnection, stats) — uses the given connection
```

**MySQL limitation:** `upsert`, `update`, and the row-returning form of `insert`/`insert_all`
emulate `RETURNING` with a follow-up `SELECT` on the same executor, which requires the executor
to be `Copy` — true for `&DbPool`, never true for `&mut DbConnection`. So `#[transactional]`
combined with any of those does **not** compile under the `mysql` feature; only the pool-based
(non-`_tx`) call works there. The void form of `insert`/`insert_all` (`-> sqlx::Result<()>`) is
exempt, since it's a single statement with nothing to fetch back. Postgres and SQLite have no
such restriction.

### Combining with `mockall`

Add `mock` to the macro to generate `#[cfg_attr(test, ::mockall::automock)]` on the trait:

```rust
#[repository(table = "onboarding_test", mock)]
pub trait OnboardingTestRepository: Send + Sync { /* ... */ }
```

### `_this_week` — cross-dialect caveat

`find_all_by_*_this_week` (and `#[created_at(col)]` to pick a column other than `created_at`)
filters rows from the current ISO week (Monday start), using the backend-specific
`axumstart_db::WEEK_START_SQL` boundary expression:

- **Postgres/MySQL:** works against a native `TIMESTAMP`/`DATETIME` column.
- **SQLite:** has no native timestamp type. `WEEK_START_SQL` produces an ISO8601 date string,
  which only compares correctly against a column storing ISO8601 **TEXT** (e.g. `created_at
  TEXT DEFAULT CURRENT_TIMESTAMP`). If your SQLite schema stores `created_at` as an INTEGER unix
  timestamp instead, the comparison silently returns no rows — SQLite compares INTEGER-affinity
  columns and TEXT literals by type, not value. Use a method body instead in that case.

Exact edge-boundary behavior (timezone handling, week-start inclusivity) is not guaranteed to be
bit-identical across the three backends — pick one backend per deployment and rely on that.

## `#[derive(SqlxInsert)]`

Generates `__sqlx_insert_into(table, pool)`, `__sqlx_insert_ignore_into`,
`__sqlx_insert_all_into`, and `__sqlx_upsert_into` on a named struct, from its fields in
declaration order:

```rust
#[derive(SqlxInsert)]
pub struct UserStatsValues {
    pub user_id: i32,
    pub self_acceptance: i32,
    // ...
}
```

**Postgres/SQLite:** all four build `INSERT ... RETURNING *` (or `ON CONFLICT ... RETURNING *`
for upsert) — one round-trip.

**MySQL has no `RETURNING`.** Every write is followed by a second `SELECT` on the same
connection to hand the row back:
- `insert`: `INSERT` → `last_insert_id()` → `SELECT * WHERE {key} = ?`
- `insert_all`: multi-row `INSERT` → `SELECT * WHERE {key} BETWEEN first_id AND first_id +
  rows.len()`, relying on MySQL's contiguous auto-increment allocation for a single multi-row
  insert (true under the default `innodb_autoinc_lock_mode`) — if the table's key isn't an
  auto-increment column, this won't return the right rows.
- `upsert`: `INSERT ... ON DUPLICATE KEY UPDATE ...` → `SELECT * WHERE {conflict_col} = ?`

`{key}` for `insert`/`insert_all` comes from an optional struct-level attribute, defaulting to
`id`:

```rust
#[derive(SqlxInsert)]
#[key(user_id)]
pub struct UserStatsValues { /* ... */ }
```

All field types must implement `sqlx::Encode + sqlx::Type + Send`; on MySQL every field must
also implement `Clone` (the upsert path keeps a copy of whichever field ends up being the
runtime-selected conflict column, since the `INSERT`'s bind chain already consumes `self` by the
time the follow-up `SELECT` needs it).

## `#[derive(SqlxUpdate)]`

One field marked `#[key]` is the `WHERE` target; every other field is a `SET` column.

```rust
#[derive(SqlxUpdate)]
pub struct UserStatsUpdate {
    #[key]
    pub user_id: i32,
    pub self_acceptance: i32,
}
```

Postgres/SQLite generate `UPDATE ... SET ... WHERE key = $n RETURNING *`. MySQL generates
`UPDATE ... SET ... WHERE key = ?` followed by `SELECT * WHERE key = ?` — the same `Copy`
executor requirement as above, so no `#[transactional]` on MySQL for update either.
