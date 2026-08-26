---
sidebar_position: 1
slug: /
---

# axumstart

axumstart is a small Rust workspace built around [Axum](https://github.com/tokio-rs/axum) that
provides two independent libraries for building services:

- **[axumstart_components](/docs/components/)** — an async dependency-injection container, with
  derive macros for registering components and an Axum extractor (`Inject<T>`) for pulling them
  out of request handlers.
- **[axumstart_db](/docs/db/)** — a proc-macro that generates a repository implementation from a
  trait definition, turning method names like `find_by_user_id` or `insert_all` directly into
  SQL against Postgres, MySQL, or SQLite via [`sqlx`](https://github.com/launchbadge/sqlx).

The two crates are independent — you can use either one on its own, or both together (a
repository is a natural fit for `#[derive(Component)]`).

## Installation

Both crates are path dependencies inside this workspace. Add whichever you need to your
service's `Cargo.toml`:

```toml
[dependencies]
axumstart_components = { path = "../axumstart_components" }
axumstart_db = { path = "../axumstart_db", features = ["postgres"] }
```

`axumstart_db` requires picking exactly one backend feature — `postgres`, `mysql`, or `sqlite`.
See [axumstart_db](/docs/db/) for why that's a compile-time choice.
