# insert_all

`insert_all(&self, values: Vec<T>)` where `T` derives `#[derive(SqlxInsert)]` (see
[insert](./insert.md)) — a single multi-row `INSERT` for the whole batch, one round-trip.

```rust
async fn insert_all(&self, values: Vec<UserValues>) -> sqlx::Result<Vec<UserRow>>;
```

Void vs. row-returning works the same as [insert](./insert.md#void-vs-row-returning) — declare
`-> sqlx::Result<()>` to skip fetching the written rows back. An empty `Vec` short-circuits to
`Ok(vec![])` / `Ok(())` without touching the database.

## Per-dialect SQL

**Postgres/SQLite:** built with `sqlx::QueryBuilder::push_values` — `INSERT INTO "table" (cols...)
VALUES ($1,$2),($3,$4),... RETURNING *`.

**MySQL has no `RETURNING`.** The row-returning form requires the key field in the values struct,
performs one batch INSERT, and selects back using those bound keys. Without a supplied key, a
nonempty row-returning batch fails before writing instead of guessing an auto-increment range.
Use the `()`-returning form for generated-key batches.
