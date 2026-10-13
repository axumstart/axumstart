# find_random_by

`find_random_by_{filter}` — same [filter grammar](./dsl.md#shared-filter-grammar) as
[first_by](./first_by.md), but orders randomly and limits to one row:

```rust
async fn find_random_by_team_id(&self, team_id: i32) -> sqlx::Result<Option<UserRow>>;
// SELECT * FROM "user" WHERE team_id = $1 ORDER BY RANDOM() LIMIT 1
```

The `ORDER BY` clause is backend-specific — `RANDOM()` on Postgres/SQLite, `RAND()` on MySQL —
chosen at macro-expansion time from the active `postgres`/`mysql`/`sqlite` feature, so the method
body itself is identical across backends.

Random ordering evaluates candidate rows and can become expensive for a large matching set. Keep
the filter selective; for high-volume sampling, write a custom method using an indexed strategy.

Fetch mode is inferred from the return type the same way as `first_by_*`, including the same
`Vec<T>` compile error (see
[first_by](./first_by.md#fetch-mode--inferred-from-the-return-type)).

**Not supported here:** `#[order_by(...)]` (there's already a random order) — a compile error if
present.
