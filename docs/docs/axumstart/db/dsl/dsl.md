---
sidebar_position: 1
---

# Dsl

`#[repository(table = "...")]` generates a method's body purely from its **name** — there's no
attribute per method, no query-builder calls to write. Each page below documents one method-name
pattern (one "function type"). This page covers the grammar shared across all of them, plus the
modifiers that apply across several.

| Pattern | Returns | Page |
|---|---|---|
| `first_by_{filter}` | one row | [first_by](./first_by.md) |
| `find_random_by_{filter}` | one random row | [find_random_by](./find_random_by.md) |
| `find_all_by_{filter}`, `find_all`, `find_all_order_by_{col}`, `find_all_{bool_field}` | rows | [find_all](./find_all.md) |
| `count_by_{filter}` | `i64` | [count_by](./count_by.md) |
| `exists_by_{filter}` | `bool` | [exists_by](./exists_by.md) |
| `delete_by_{filter}` | `()` | [delete_by](./delete_by.md) |
| `set_{field}_by_{filter}` | `()` | [set_by](./set_by.md) |
| `insert` | inserted row (or `()`) | [insert](./insert.md) |
| `insert_all` | inserted rows (or `()`) | [insert_all](./insert_all.md) |
| `insert_or_ignore` | `()` | [insert_or_ignore](./insert_or_ignore.md) |
| `upsert` | upserted row | [upsert](./upsert.md) |
| `upsert_all` | upserted rows (or `()`) | [upsert_all](./upsert_all.md) |
| `update` | updated row | [update](./update.md) |
| `pool` | `&DbPool` | [pool](./pool.md) |

Relations (`belongs_to` / `has_one` / `has_many` / `belongs_to_many`) let a `{filter}` reference a
joined table's columns — see [relations](../relations/relations.md).

## Shared filter grammar

`first_by_*`, `find_random_by_*`, `find_all_by_*`, `count_by_*`, `exists_by_*`, `delete_by_*`, and
the filter half of `set_*_by_*` all parse `{filter}` the same way.

### Operators

A trailing suffix on a field picks its operator; no suffix means `=`. Longest suffix wins, so
`_gte` doesn't get mistaken for `_gt`.

| Suffix | SQL | Takes a bound parameter? |
|---|---|---|
| *(none)* | `= $n` | yes |
| `_gt` | `> $n` | yes |
| `_gte` | `>= $n` | yes |
| `_lt` | `< $n` | yes |
| `_lte` | `<= $n` | yes |
| `_like` | `LIKE $n` | yes |
| `_in` | `IN (...)` | yes — takes a `Vec<T>` |
| `_is_null` | `IS NULL` | no |
| `_is_not_null` | `IS NOT NULL` | no |

`_in` is built at runtime with `sqlx::QueryBuilder` (via the exported `push_in_list` helper)
instead of `= ANY($n)`, since neither MySQL nor SQLite let a single placeholder bind a `Vec<T>` —
so it works identically on all three backends, Postgres included. A method whose filter uses
`_in` at all switches its *entire* WHERE clause to the `QueryBuilder` path, not just that one
condition.

An empty `_in` list renders `IN (NULL)`, so it matches no rows without producing invalid `IN ()`.
SQL identifiers are quoted for the selected backend while filter values remain bound.

### Connectors — `_and_` / `_or_`

Chain fields with `_and_` / `_or_`: `first_by_username_or_email`, `first_by_user_id_and_deleted_at_is_null`.

**There's no grouping.** The name is parsed left to right into a flat sequence of conditions —
whichever of `_and_`/`_or_` occurs first in the string wins that split — and the resulting SQL has
no parentheses, so it inherits SQL's normal `AND`-binds-tighter-than-`OR` precedence. A name like
`a_and_b_or_c` renders as `a = $1 AND b = $2 OR c = $3`, which evaluates as `(a AND b) OR c`, not
`a AND (b OR c)`. If you need different grouping, write the method body by hand instead.

## Compile-time checks

Two things are checked at macro-expansion time, before any SQL ever runs:

- **Arity** — the number of non-`Page` parameters must match the number of bound placeholders the
  name implies. `first_by_username_or_email(&self)` (missing both params) is a compile error, not a
  runtime one.
- **Column probing** — every column name the DSL parses out of the method name is checked against
  the return type's row struct (`r.column` must compile), catching typos immediately instead of
  failing at query time. Columns resolved through a relation's JOIN are *not* probed (they don't
  live on this table's row struct); FK-shortcut columns and plain base-table columns are.

Opt a single method out of column probing with `#[unchecked_columns]` (e.g. if the row struct has
a field with an unrelated name via `#[sqlx(rename = "...")]`):

```rust
#[unchecked_columns]
async fn first_by_legacy_uid(&self, uid: String) -> sqlx::Result<UserRow>;
```

## `#[order_by("col1", "col2")]`

An alternative to an embedded `_order_by_` suffix (see [find_all](./find_all.md)) for when the
order needs to come from something other than the method name, or needs multiple columns. The
method name must end in `_ordered` (stripped before the rest of the name is parsed):

```rust
#[order_by("elo", "created_at")]
async fn first_by_team_id_ordered(&self, team_id: i32) -> sqlx::Result<Option<UserRow>>;
// SELECT * FROM "user" WHERE team_id = $1 ORDER BY elo, created_at LIMIT 1
```

Supported on `first_by_*`, `find_all_by_*`, bare `find_all`, and `find_all_{bool_field}` only —
not `find_random_by_*`, `count_by_*`, `exists_by_*`, `delete_by_*`, `set_*_by_*`, or any of the
insert/upsert/update methods (compile error otherwise). Can't be combined with an embedded
`_order_by_` suffix on the same method.

## `Page` — pagination

A parameter of type `Page` (`Page { limit: i64, offset: i64 }`, built via `Page::new(limit,
offset)` or `Page::number(page, size)`) appends `LIMIT $n OFFSET $m` — detected by type, not by
name, so it can go anywhere in the parameter list. Supported on `find_all_by_*`, bare `find_all`,
`find_all_order_by_*`, and `find_all_{bool_field}` only — see [find_all](./find_all.md).

For external input, prefer `Page::try_new(limit, offset)` or `Page::try_number(page, size)`;
they reject negative values and overflow. Generated queries validate `Page` before sending SQL.

## `_this_week`

`find_all_by_*_this_week` filters to the current ISO week (Monday start) — see
[find_all](./find_all.md) for the full caveat on cross-dialect `TEXT`/`TIMESTAMP` behavior.

The complete filter is parenthesized before the week predicate, so an OR branch cannot bypass the
date condition. SQLite's boundary expression returns the current Monday even when today is Monday.

## `#[transactional]`

Generates a `{method}_tx` twin taking an extra `conn: &mut DbConnection` instead of the pool —
the original pool-based method is still generated alongside it:

```rust
#[transactional]
async fn update(&self, stats: UserStatsRow) -> sqlx::Result<UserStatsRow>;
// generates both:
//   update(&self, stats)                            — uses the pool
//   update_tx(&self, conn: &mut DbConnection, stats) — uses the given connection
```

**MySQL limitation:** the row-returning forms of `insert`/`insert_all`/`upsert`/`upsert_all`/
`update` emulate `RETURNING` with a follow-up `SELECT` on the *same* executor, which requires the
executor to be `Copy` — true for `&DbPool`, never true for `&mut DbConnection`. So combining
`#[transactional]` with any of those does **not** compile under the `mysql` feature; only the
pool-based (non-`_tx`) call works there. The `()`-returning forms are exempt (a single statement,
nothing to fetch back). Postgres and SQLite have no such restriction.

## `#[repository(table = "...", mock)]`

Add `mock` to generate `#[cfg_attr(test, ::mockall::automock)]` on the trait — see
[db](../db.md#combining-with-mockall).
