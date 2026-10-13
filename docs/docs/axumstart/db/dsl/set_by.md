# set_by

`set_{field}[_and_{field2}...]_by_{filter}` — an `UPDATE` that sets one or more fixed columns on
every row matching a [filter](./dsl.md#shared-filter-grammar), without needing a full values
struct:

```rust
async fn set_email_verified_at_by_id(&self, email_verified_at: DateTime<Utc>, id: i32) -> sqlx::Result<()>;
// UPDATE "user" SET email_verified_at = $1 WHERE id = $2
```

Multiple `SET` columns are chained with `_and_` before `_by_`:

```rust
async fn set_verified_and_verified_at_by_id(
    &self, verified: bool, verified_at: DateTime<Utc>, id: i32,
) -> sqlx::Result<()>;
// UPDATE "user" SET verified = $1, verified_at = $2 WHERE id = $3
```

## Parameter order

Parameters are positional, not name-matched: the **first N** parameters (N = number of `SET`
columns) are the new values, in the order the `SET` columns appear in the method name; the
**rest** are the filter's bound values, in filter order. Arity is checked at compile time against
`(SET column count) + (filter parameter count)`.

## What this is for vs. `update`

`set_by_*` updates a **fixed set of columns to values you pass directly** — no values struct, no
`#[derive(SqlxUpdate)]`, and it can touch **many rows** (it's filter-based, not keyed to one row).
Use [update](./update.md) instead when you're updating a whole row by its primary key from a
values struct.

## Constraints

- `_in` in the filter works as usual, switching that clause to the `QueryBuilder` path.
- Filters requiring a JOIN are rejected at compile time. A belongs-to FK shortcut remains valid;
  use an explicit method body/subquery for updates filtered through other tables.
