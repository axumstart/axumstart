use axumstart_components::{ComponentProvider, OnCreate, async_trait, component, Component};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

trait Greeter: Send + Sync {
  fn greet(&self) -> &'static str;
}

#[derive(Component)]
struct InvBase;

#[derive(Component)]
#[as_trait(Greeter)]
struct InvGreeter {
  _base: Arc<InvBase>,
}

impl Greeter for InvGreeter {
  fn greet(&self) -> &'static str {
    "hi"
  }
}

struct InvHttpClient {
  label: &'static str,
}

#[component]
fn build_inv_http_client() -> InvHttpClient {
  InvHttpClient { label: "sync-no-ctx" }
}

struct InvAsyncThing {
  saw_base: bool,
}

#[component]
async fn build_inv_async_thing(ctx: &ComponentProvider) -> InvAsyncThing {
  let _base: Arc<InvBase> = ctx.get().await;
  InvAsyncThing { saw_base: true }
}

// tier 3: auto-derived ComponentBlueprint + hand-written OnCreate (no Dyn) — mirrors
// DispatchWorker before it moved to factory-fn mode; kept to prove struct-mode still
// detects OnCreate on its own.
#[derive(Component)]
struct InvOnCreateOnly {
  #[default]
  created: AtomicBool,
}

#[async_trait]
impl OnCreate for InvOnCreateOnly {
  async fn on_create(&self, _ctx: &ComponentProvider) {
    self.created.store(true, Ordering::SeqCst);
  }
}

// tier 1: auto-derived ComponentBlueprint + DynComponentBlueprint (`as = Trait`) + a
// hand-written OnCreate — proves register_on_create_dyn wiring end-to-end.
trait Pinger: Send + Sync {
  fn ping(&self) -> &'static str;
}

#[derive(Component)]
#[as_trait(Pinger)]
struct InvDynOnCreate {
  #[default]
  created: AtomicBool,
}

impl Pinger for InvDynOnCreate {
  fn ping(&self) -> &'static str {
    "pong"
  }
}

#[async_trait]
impl OnCreate for InvDynOnCreate {
  async fn on_create(&self, _ctx: &ComponentProvider) {
    self.created.store(true, Ordering::SeqCst);
  }
}

// factory mode: a plain function-registered type that also hand-implements OnCreate.
struct InvFactoryOnCreate {
  created: AtomicBool,
}

#[component]
fn build_inv_factory_on_create() -> InvFactoryOnCreate {
  InvFactoryOnCreate { created: AtomicBool::new(false) }
}

#[async_trait]
impl OnCreate for InvFactoryOnCreate {
  async fn on_create(&self, _ctx: &ComponentProvider) {
    self.created.store(true, Ordering::SeqCst);
  }
}

#[tokio::test]
async fn register_all_detects_on_create_on_plain_struct() {
  let ctx = ComponentProvider::default();
  ctx.register_all();

  let comp = ctx.get::<InvOnCreateOnly>().await;
  assert!(comp.created.load(Ordering::SeqCst));
}

#[tokio::test]
async fn register_all_detects_on_create_dyn_combo() {
  let ctx = ComponentProvider::default();
  ctx.register_all();

  let dyn_comp = ctx.get::<dyn Pinger>().await;
  assert_eq!(dyn_comp.ping(), "pong");

  let concrete = ctx.get::<InvDynOnCreate>().await;
  assert!(concrete.created.load(Ordering::SeqCst));
}

#[tokio::test]
async fn register_all_detects_on_create_for_factory_fn() {
  let ctx = ComponentProvider::default();
  ctx.register_all();

  let comp = ctx.get::<InvFactoryOnCreate>().await;
  assert!(comp.created.load(Ordering::SeqCst));
}

#[tokio::test]
async fn register_all_wires_struct_components() {
  let ctx = ComponentProvider::default();
  ctx.register_all();

  let greeter = ctx.get::<dyn Greeter>().await;
  assert_eq!(greeter.greet(), "hi");
}

#[tokio::test]
async fn register_all_wires_factory_fn_components() {
  let ctx = ComponentProvider::default();
  ctx.register_all();

  let client = ctx.get::<InvHttpClient>().await;
  assert_eq!(client.label, "sync-no-ctx");

  let thing = ctx.get::<InvAsyncThing>().await;
  assert!(thing.saw_base);
}
