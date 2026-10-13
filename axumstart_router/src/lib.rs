//! Auto-registered Axum routes. `#[get("/path")]`/`#[post(...)]`/etc. (see
//! `axumstart_router_macros`, re-exported here) submit a [`RouteRegistration`] via `inventory`;
//! [`register_routes`] folds all of them onto a [`axum::Router`] at startup, the same way
//! `axumstart_components::ComponentProvider::register_all` wires up DI components.

pub use axumstart_components::Components;
pub use axumstart_components::axum::Inject;
pub use axumstart_router_macros::{
  connect, delete, get, head, layer, options, patch, post, put, trace,
};
pub use inventory;

/// One auto-registered route: takes the router built so far (and the resolved DI container, for
/// `#[layer(...)]` factories that want it) and returns the router with this route added.
pub struct RouteRegistration(
  pub fn(axum::Router<Components>, &Components) -> axum::Router<Components>,
);
inventory::collect!(RouteRegistration);

/// Folds every `#[get]`/`#[post]`/etc.-registered route onto `router`, in whatever order
/// `inventory` happens to iterate them in (unspecified, but harmless here since each
/// registration only ever touches its own path — a path/method collision panics via axum's
/// normal duplicate-route check, regardless of fold order).
pub fn register_routes(
  router: axum::Router<Components>,
  ctx: &Components,
) -> axum::Router<Components> {
  let mut router = router;
  for registration in inventory::iter::<RouteRegistration> {
    router = registration.0(router, ctx);
  }
  router
}

/// Resolves a `#[layer(...)]` argument that was a bare function name rather than a ready
/// expression — the function may optionally take `&Components`, picked via the same
/// autoref/autoderef "try the more-bounded impl first" trick
/// `axumstart_components::RegisterProbe`/`FactoryProbe` already use, since we can't inspect the
/// referenced function's real signature at macro-expansion time (unlike `#[component]`, which is
/// applied directly to the function it inspects).
#[doc(hidden)]
pub struct LayerFactoryPlain<F>(pub F);
#[doc(hidden)]
pub struct LayerFactoryProbe<F>(pub LayerFactoryPlain<F>);

impl<F> LayerFactoryProbe<F> {
  pub fn new(f: F) -> Self {
    LayerFactoryProbe(LayerFactoryPlain(f))
  }
}

impl<F> std::ops::Deref for LayerFactoryProbe<F> {
  type Target = LayerFactoryPlain<F>;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

#[doc(hidden)]
impl<F, L> LayerFactoryProbe<F>
where
  F: Fn(&Components) -> L,
{
  pub fn call(&self, ctx: &Components) -> L {
    (self.0.0)(ctx)
  }
}

#[doc(hidden)]
impl<F, L> LayerFactoryPlain<F>
where
  F: Fn() -> L,
{
  pub fn call(&self, _ctx: &Components) -> L {
    (self.0)()
  }
}
