# insert

`insert(&self, values: T)` where `T` derives `#[derive(SqlxInsert)]` — a single-row `INSERT`
built from every named field of `T`, in declaration order.

```rust
#[derive(SqlxInsert)]
pub struct UserValues {
    pub username: String,
    pub email: String,
}

async fn insert(&self, values: UserValues) -> sqlx::Result<UserRow>;
```

## Void vs. row-returning

The return type decides whether the written row is fetched back:

```rust
async fn insert(&self, values: EventValues) -> sqlx::Result<()>;       // INSERT, nothing fetched back
async fn insert(&self, values: EventValues) -> sqlx::Result<EventRow>; // INSERT ... RETURNING *
```

## Per-dialect SQL

**Postgres/SQLite:** `INSERT INTO "table" (cols...) VALUES ($1, ...) RETURNING *` — one
round-trip.

**MySQL has no `RETURNING`.** When the values struct contains its key, the row-returning form
selects back using that bound value. If the key is omitted, the helper checks for an auto-increment
key before writing, then uses `last_insert_id()`; unsupported schemas return an error before the
INSERT. `{key}` defaults to `id`:

```rust
#[derive(SqlxInsert)]
#[key(user_id)]
pub struct UserValues { /* ... */ }
```

## Field requirements

Every field must implement `sqlx::Encode + sqlx::Type + Send`. **On MySQL, every field must also
implement `Clone`** — this applies to the whole derived struct unconditionally (not just when you
call `insert`), because the `upsert`/`upsert_all` methods `#[derive(SqlxInsert)]` also generates
on the same struct (see [upsert](./upsert.md)) need to keep a copy of whichever field ends up
being the runtime-selected conflict column, and that code is compiled as part of the struct's
`impl` block regardless of whether you ever call it.

## See also

- [insert_all](./insert_all.md) — batch insert
- [insert_or_ignore](./insert_or_ignore.md) — `ON CONFLICT DO NOTHING` / `INSERT IGNORE`
- [upsert](./upsert.md) / [upsert_all](./upsert_all.md) — insert-or-update, same derive
