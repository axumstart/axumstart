# axumstart

Axum starter crates for dependency injection, auto-registered routes, and compile-time SQLx
repositories.

## Quick start

```toml
[dependencies]
axumstart = { version = "0.1", features = ["router", "postgres"] }
```

```rust
use axumstart::router::get;

#[get("/health")]
async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> std::io::Result<()> {
    axumstart::serve(3000).await
}
```

Use `axumstart::build_app().await` when the application needs to merge manual routes or add global
middleware before serving.

## Features

- `components` — typed async components and `Inject<T>`.
- `router` — auto-registered Axum handlers and `build_app`/`serve` (default).
- Exactly one database backend: `postgres`, `mysql`, or `sqlite`.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cd docs && npm ci && npm run build
```

The quickstart database example lives in `examples/quickstart`. The current MSRV follows SQLx
0.9's declared Rust 1.94 requirement; CI tests stable Rust.

## Architecture notes

`inventory` supplies zero-boilerplate component and route registration. `async-trait` keeps the
generated async repository traits usable as `Arc<dyn Repository>` objects. Both are deliberate API
trade-offs; removing either would require a public design change.

For generated SQL troubleshooting, inspect macro expansion with `cargo expand` in a consumer crate;
do not log bound values in production.
