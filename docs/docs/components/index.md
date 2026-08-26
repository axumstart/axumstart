---
sidebar_position: 1
---

# axumstart_components

An async dependency-injection container. Components are registered as *blueprints* (a recipe
for constructing `T`), resolved lazily and cached on first use, and can be pulled out of Axum
handlers with the `Inject<T>` extractor.

## Core types

| Type | Purpose |
|---|---|
| `ComponentProvider` | Registration-time container. Register blueprints, then call `.build()`. |
| `Components` | Immutable, `Clone`-cheap snapshot of every built component — this is your Axum app state. |
| `Inject<T>` | `FromRequestParts<Components>` extractor; derefs to `&T`. |

```rust
use axumstart_components::{ComponentProvider, Component};

#[derive(Component)]
struct Settings {
    #[default]
    port: u16,
}

let ctx = ComponentProvider::default();
ctx.register::<Settings>();

let components = ctx.build().await; // resolves every registered blueprint
let settings = components.get::<Settings>().unwrap();
```

## Registering a component

### `#[derive(Component)]`

The most common path. Each named field is resolved from the `ComponentProvider`:

- `Arc<T>` fields resolve via `ctx.get::<T>().await` (shared with every other consumer of `T`).
- Any other field resolves via `ctx.get_cloned::<T>().await` (requires `T: Clone`).
- A field marked `#[default]` is built with `Default::default()` instead, skipping the
  container entirely.

```rust
use axumstart_components::Component;
use std::sync::Arc;

#[derive(Component)]
struct Database {
    pool: Arc<DbPool>,
    #[default]
    query_count: AtomicU64,
}
```

This expands to a `ComponentBlueprint for Database` impl and an `inventory` registration, so it
participates in [`register_all()`](#registering-everything-at-once-inventory) automatically.

### Registering trait objects — `#[as_trait(Trait)]`

Add `#[as_trait(Trait)]` to also register the component as `Arc<dyn Trait>`:

```rust
trait Greeter: Send + Sync {
    fn greet(&self) -> &'static str;
}

#[derive(Component)]
#[as_trait(Greeter)]
struct EnglishGreeter;

impl Greeter for EnglishGreeter {
    fn greet(&self) -> &'static str { "hi" }
}

// later:
let greeter: Arc<dyn Greeter> = components.get::<dyn Greeter>().unwrap();
```

Concrete and trait-object registrations resolve to the same underlying instance.

### Factory functions — `#[component]`

For construction logic that doesn't fit a plain struct literal (or types you don't own), write
a free function instead:

```rust
use axumstart_components::{component, ComponentProvider};

#[component]
fn build_http_client() -> reqwest::Client {
    reqwest::Client::new()
}

#[component]
async fn build_thing(ctx: &ComponentProvider) -> Thing {
    let db: Arc<Database> = ctx.get().await;
    Thing::new(db)
}
```

Rules: at most one parameter, and it must be `&ComponentProvider`; the function must return the
constructed type (async or sync, both work).

### Config components — `#[derive(ComponentConfig)]`

A config struct whose fields are read from environment variables once at registration time
(rather than re-read at every call site):

```rust
use axumstart_components::ComponentConfig;

#[derive(Clone, ComponentConfig)]
struct AppConfig {
    #[env_var("DATABASE_URL")]
    database_url: String,
    #[env_var("PORT", 8080)]
    port: u16,
}
```

`#[env_var("VAR")]` panics at resolution time if the variable is unset or fails to parse.
`#[env_var("VAR", default)]` falls back to `default` (any expression of the field's type) if the
variable is unset, but still panics on a parse failure. Read it back with
`ctx.get_cloned::<AppConfig>().await`.

## Lifecycle: `OnCreate`

Implement `OnCreate` to run logic exactly once, right after a component is first constructed —
useful for spawning background tasks or kicking off warm-up work:

```rust
use axumstart_components::{async_trait, ComponentProvider, OnCreate};

#[async_trait]
impl OnCreate for Worker {
    async fn on_create(&self, ctx: &ComponentProvider) {
        // runs once, before any caller observes the resolved instance
    }
}
```

Register with `register_on_create::<T>()` (or `register_on_create_dyn::<T>()` to combine with
`#[as_trait]`) instead of the plain `register` variants. `#[derive(Component)]` /
`#[component]` detect a hand-written `OnCreate` impl automatically and route to the right
`register_*` call when `register_all()` runs — you don't call these manually alongside the
derive/attribute macros.

## Registering everything at once: `inventory`

Every `#[derive(Component)]`, `#[component]`, and `#[derive(ComponentConfig)]` submits itself to
a global `inventory` registry at compile time. Instead of calling `ctx.register::<T>()` for each
type by hand, call:

```rust
let ctx = ComponentProvider::default();
ctx.register_all();
let components = ctx.build().await;
```

This walks every collected registration and wires it up — the common case for an application
entry point. Manual `ComponentBlueprint` impls (not using the derive/attribute macros) are not
picked up by `register_all()` and must be registered explicitly.

## Resolving components

- `ctx.get::<T>().await -> Arc<T>` — resolves (and caches) during the registration phase.
- `ctx.get_cloned::<T>().await -> T` — same, then clones out of the `Arc` (`T: Clone`).
- After `ctx.build().await` produces a `Components` snapshot: `components.get::<T>()`,
  `components.get_cloned::<T>()` — both return `Option`, no `.await` needed.

Resolution is lazy and memoized per `TypeId` via a `tokio::sync::OnceCell`, so a component with
multiple dependents is only constructed once. A dependency cycle panics with the resolution
chain (`circular dependency detected: A -> B -> A`) rather than deadlocking.

## Axum integration

`Components` implements the shape Axum's `FromRequestParts` needs, so use it directly as router
state, and pull dependencies out of handlers with `Inject<T>`:

```rust
use axum::{Router, routing::get};
use axumstart_components::{ComponentProvider, axum::Inject};
use std::sync::Arc;

async fn handler(Inject(db): Inject<Database>) -> String {
    db.query_count.load(Ordering::Relaxed).to_string()
}

async fn build_app() -> Router {
    let ctx = ComponentProvider::default();
    ctx.register_all();
    let components = ctx.build().await;

    Router::new()
        .route("/", get(handler))
        .with_state(components)
}
```

`Inject<T>` rejects with a `String` (used as the Axum rejection body) naming the missing type if
`T` was never registered — this only happens if the component is missing from the container, not
per-request.
