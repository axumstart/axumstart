# axumstart_db

Proc-macro crate (`axumstart_db_macros`, re-exported here) that generates a repository
struct from a trait definition, plus the runtime types it depends on (`Page`,
`Repository`, `Db`/`DbPool`/`DbConnection` aliases).

## Picking a backend

Exactly one of the `postgres` / `mysql` / `sqlite` Cargo features must be enabled —
the target backend is a compile-time choice, since SQL placeholder syntax and RETURNING
availability are baked into generated queries at macro-expansion time. Enabling zero or
more than one is a compile error.

```toml
axumstart_db = { path = "...", features = ["postgres"] }
```

## `#[repository(table = "...")]`

Annotate a trait to generate:
- The trait itself, wrapped with `#[async_trait]`
- A `Db{TraitName}` struct with a `pool: DbPool` field and `new(pool: DbPool) -> Self`
- `impl {TraitName} for Db{TraitName}` with bodies generated from method names (see below)

```rust
#[repository(table = "user_stats")]
pub trait UserStatsRepository: Send + Sync {
    async fn find_by_user_id(&self, user_id: i32) -> sqlx::Result<Option<UserStatsRow>>;
    async fn insert(&self, values: UserStatsValues) -> sqlx::Result<UserStatsRow>;
    fn db(&self) -> &DbPool;
}

// Generated: DbUserStatsRepository { pool }  +  impl UserStatsRepository for DbUserStatsRepository
```

### `join(table_name)` — cross-table field resolution

Declare related tables so that `find_by_*` method names can reference their columns:

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

Convention: `{join}_id` FK must exist on the main table. Multiple joins: `join(challenge, user)`.

Field resolution: `{join_name}_{col}` → `"{join_name}".{col}` with JOIN added. All other fields stay unqualified.

### Supported method name conventions

| Method name pattern | Generated SQL |
|---|---|
| `find_by_{field}` | `SELECT * FROM "{table}" WHERE {field} = $1` |
| `find_by_{f1}_and_{f2}` | `SELECT * FROM "{table}" WHERE f1 = $1 AND f2 = $2` |
| `find_all_by_{field}` | `SELECT * FROM "{table}" WHERE {field} = $1` (always `fetch_all`) |
| `find_random_by_{field}` | `SELECT * FROM "{table}" WHERE {field} = $1 ORDER BY RANDOM() LIMIT 1` |
| `exists_by_{field}` | `SELECT EXISTS(SELECT 1 FROM "{table}" WHERE {field} = $1)` |
| `delete_by_{field}` | `DELETE FROM "{table}" WHERE {field} = $1` |
| `insert` | Delegates to `SqlxInsert::__sqlx_insert_into` (requires `#[derive(SqlxInsert)]` on the values struct) |
| `insert_all` | Single multi-row INSERT for the whole batch |

`insert` and `insert_all` only fetch the written row(s) back when the method actually
returns them. Declare `-> sqlx::Result<()>` instead of `-> sqlx::Result<Row>` /
`-> sqlx::Result<Vec<Row>>` and codegen skips RETURNING entirely (Postgres/SQLite) or the
follow-up SELECT entirely (MySQL) — a plain INSERT, nothing fetched back:

```rust
async fn insert(&self, values: EventValues) -> sqlx::Result<()>;
async fn insert_all(&self, values: Vec<EventValues>) -> sqlx::Result<()>;
```

Bonus on MySQL: since the void form is a single statement, it doesn't need `E: Copy` the
way the Row-returning form does — `#[transactional] insert`/`insert_all` **does** work on
MySQL when the return type is `()` (see the `#[transactional]` limitation below, which
only applies to the Row-returning form).
| `db` or `pool` | Returns `&self.pool` |

Multi-field `_and_` chaining works for any number of fields:
`find_by_user_id_and_test_type` → `WHERE user_id = $1 AND test_type = $2`.

**Fetch mode** for `find_by_*` is inferred from the return type:
- `Option<T>` → `fetch_optional`
- `Vec<T>` → `fetch_all`
- anything else → `fetch_one`

**Methods with a default body** are emitted as-is; no codegen is applied.

**Unknown names** compile to `todo!("no codegen rule for `name`")`.

### `_in` — works the same on every backend

`find_by_id_in`, `delete_by_id_in`, `find_all_by_id_in`, etc. take a `Vec<T>` and render
an `IN (...)` list. Unlike the original Postgres-only implementation (which used
`= ANY($n)`), this is now built at runtime via `sqlx::QueryBuilder` — the same code path
on all three backends, since neither MySQL nor SQLite (nor, for consistency, Postgres
here) let a single placeholder bind a `Vec<T>`.

### `#[transactional]`

Marking a method `#[transactional]` generates a `{method}_tx` variant that takes an extra
`conn: &mut DbConnection` instead of using `self.pool`. The original method is also
generated using the pool.

```rust
#[transactional]
async fn update(&self, stats: UserStatsRow) -> sqlx::Result<UserStatsRow>;
// generates both:
//   update(&self, stats) — uses pool
//   update_tx(&self, conn: &mut DbConnection, stats) — uses conn
```

**MySQL limitation:** `upsert`, `update`, and the Row-returning form of `insert`/
`insert_all` emulate RETURNING with a follow-up SELECT on the same executor, which
requires the executor to be `Copy` (true for `&DbPool`, never true for
`&mut DbConnection`). This means `#[transactional]` combined with any of those does not
compile under the `mysql` feature — only the pool-based (non-`_tx`) call works. The void
form of `insert`/`insert_all` (`-> sqlx::Result<()>`) is exempt — see above. Postgres and
SQLite have no such restriction (RETURNING is a single statement either way).

### Combining with `mockall`

Add `mock` to the macro to generate `#[cfg_attr(test, ::mockall::automock)]` on the output trait:

```rust
#[repository(table = "onboarding_test", mock)]
pub trait OnboardingTestRepository: Send + Sync { ... }
```

### Type alias pattern

```rust
pub type OnboardingTestRepositoryImpl = DbOnboardingTestRepository;
```

---

## `#[derive(SqlxInsert)]`

Generates `__sqlx_insert_into(table, pool)`, `__sqlx_insert_ignore_into`,
`__sqlx_insert_all_into`, and `__sqlx_upsert_into` on a named struct, from its fields in
declaration order.

```rust
#[derive(SqlxInsert)]
pub struct UserStatsValues {
    pub user_id: i32,
    pub self_acceptance: i32,
    // ...
}
```

**Postgres/SQLite:** all four build `INSERT ... RETURNING *` (or `ON CONFLICT ...
RETURNING *` for upsert) — one round-trip.

**MySQL has no RETURNING.** Every write is followed by a second `SELECT` on the same
connection to hand back the row:
- `insert`: `INSERT` → `last_insert_id()` → `SELECT * WHERE {key} = ?`
- `insert_all`: multi-row `INSERT` → `SELECT * WHERE {key} BETWEEN first_id AND
  first_id + rows.len()`, relying on MySQL's contiguous auto-increment allocation for a
  single multi-row insert (true under the default `innodb_autoinc_lock_mode`) — if the
  table's key isn't an auto-increment column, this won't return the right rows.
- `upsert`: `INSERT ... ON DUPLICATE KEY UPDATE ...` → `SELECT * WHERE {conflict_col} = ?`

`{key}` for `insert`/`insert_all` comes from an optional struct-level attribute, defaulting
to `id`:

```rust
#[derive(SqlxInsert)]
#[key(user_id)]
pub struct UserStatsValues { ... }
```

All field types must implement `sqlx::Encode + sqlx::Type + Send`; on MySQL every field
must also implement `Clone` (the upsert path needs to keep a copy of whichever field ends
up being the runtime-selected conflict column, since the INSERT's bind chain already
consumes `self` by the time the follow-up SELECT needs it).

---

## `#[derive(SqlxUpdate)]`

One field marked `#[key]` is the WHERE target; every other field is a SET column.
Postgres/SQLite generate `UPDATE ... SET ... WHERE key = $n RETURNING *`. MySQL generates
`UPDATE ... SET ... WHERE key = ?` followed by `SELECT * WHERE key = ?` (same `Copy`
executor requirement as above, so no `#[transactional]` on MySQL here either).

---

## `_this_week` — cross-dialect caveat

`find_all_by_*_this_week` (and `#[created_at(col)]` to pick a column other than
`created_at`) filters rows from the current ISO week (Monday start). The boundary
expression is backend-specific (`axumstart_db::WEEK_START_SQL`) and **assumes a
particular storage convention per backend**:

- **Postgres/MySQL:** a native `TIMESTAMP`/`DATETIME` column — always comparable.
- **SQLite:** has no native timestamp type. `WEEK_START_SQL` produces an ISO8601 date
  string (`date(CURRENT_TIMESTAMP, 'weekday 1', '-7 days')`), which only compares
  correctly against a column storing ISO8601 **TEXT** (e.g. `created_at TEXT DEFAULT
  CURRENT_TIMESTAMP`). If your SQLite schema stores `created_at` as an INTEGER unix
  timestamp instead, this comparison silently returns no rows (SQLite compares
  INTEGER-affinity columns and TEXT literals by type, not value) — use a method body
  instead in that case.

Exact edge-boundary behavior (timezone handling, week-start inclusivity) is not
guaranteed to be bit-identical across the three backends.
