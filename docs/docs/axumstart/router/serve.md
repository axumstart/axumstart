---
sidebar_position: 3
sidebar_label: serve
---

# `serve`

`serve(port: u16) -> std::io::Result<()>` is the one-call entry point: load config, build the DI
container, wire up every auto-registered route, and run the server.

```rust
use axumstart::router::get;

#[get("/health")]
async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    axumstart::serve(3000).await
}
```

## What it does, in order

1. **Loads `.env`** from the current working directory (`dotenvy::dotenv().ok()` — silently does
   nothing if there isn't one). This runs *before* components are built, so
   [`#[derive(ComponentConfig)]`](../components/config_components.md)'s `#[env_var(...)]` fields
   see values from it, not just pre-existing process environment variables.
2. **Registers components** via `ComponentProvider::register_all()` (every
   `#[component]`/`#[derive(Component)]`/`#[derive(ComponentConfig)]` type).
3. **Builds** the registered blueprints into a [`Components`](../components/components.md)
   snapshot.
4. **Folds every auto-registered route** (every `#[get]`/`#[post]`/etc. handler — see
   [HTTP methods](./http_methods.md)) onto a fresh `axum::Router`, with that `Components` value
   available to any `#[layer(...)]` factory that wants it.
5. Attaches `Components` as the router's state and binds a `TcpListener` on `0.0.0.0:{port}`,
   then runs `axum::serve`.

## Feature gating

`serve` requires the `router` feature (which implies `components`) — both are **default
features**, so most consumers get it for free:

```toml
axumstart = { version = "..." }
```

If you've opted out with `default-features = false`, add it back explicitly:

```toml
axumstart = { version = "...", default-features = false, features = ["router"] }
```

## When you need more than `serve` gives you

`build_app().await -> axum::Router` loads config, builds components, and returns the state-attached
auto-registered router. Add routes and middleware, choose a listener, and use Axum's native
graceful-shutdown support. `serve(port)` remains the convenience wrapper.

```rust
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = axumstart::build_app().await
        .merge(my_manual_router())
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
# fn my_manual_router() -> axum::Router { axum::Router::new() }
```

Use `register_routes` directly when manual handlers need the same `Components` state before it
is attached. `build_app` and `serve` retain the existing panic-on-component-initialization-failure
behavior.
