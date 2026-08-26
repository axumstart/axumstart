use axumstart_components::{ComponentBlueprint, ComponentProvider, OnCreate, async_trait};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::oneshot;

struct Base {
  created: Arc<Mutex<u32>>,
}

#[async_trait]
impl ComponentBlueprint for Base {
  async fn new(_ctx: &ComponentProvider) -> Self {
    Self { created: Arc::new(Mutex::new(0)) }
  }
}

#[async_trait]
impl OnCreate for Base {
  async fn on_create(&self, _ctx: &ComponentProvider) {
    *self.created.lock() += 1;
  }
}

struct Dependent {
  saw_created: bool,
}

#[async_trait]
impl ComponentBlueprint for Dependent {
  async fn new(ctx: &ComponentProvider) -> Self {
    let base = ctx.get::<Base>().await;
    Self { saw_created: *base.created.lock() == 1 }
  }
}

#[tokio::test]
async fn on_create_runs_once() {
  let ctx = ComponentProvider::default();
  ctx.register_on_create::<Base>();

  let first = ctx.get::<Base>().await;
  let second = ctx.get::<Base>().await;

  assert_eq!(*first.created.lock(), 1);
  assert!(Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn on_create_runs_before_dependents_observe_it() {
  let ctx = ComponentProvider::default();
  ctx.register_on_create::<Base>();
  ctx.register::<Dependent>();

  let dependent = ctx.get::<Dependent>().await;

  assert!(dependent.saw_created);
}

// Mirrors the NotificationQueue/DispatchWorker shape: a "signal" component owns
// a take-once oneshot sender, and SelfSpawner's on_create clones the ComponentProvider,
// spawns a task, and re-resolves Arc<Self> from inside that task.
struct Signal {
  tx: Mutex<Option<oneshot::Sender<Arc<SelfSpawner>>>>,
}

#[async_trait]
impl ComponentBlueprint for Signal {
  async fn new(_ctx: &ComponentProvider) -> Self {
    let (tx, _rx) = oneshot::channel();
    Self { tx: Mutex::new(Some(tx)) }
  }
}

struct SelfSpawner {
  signal: Arc<Signal>,
}

#[async_trait]
impl ComponentBlueprint for SelfSpawner {
  async fn new(ctx: &ComponentProvider) -> Self {
    Self { signal: ctx.get().await }
  }
}

#[async_trait]
impl OnCreate for SelfSpawner {
  async fn on_create(&self, ctx: &ComponentProvider) {
    let Some(tx) = self.signal.tx.lock().take() else { return };
    let ctx = ctx.clone();
    tokio::spawn(async move {
      let me: Arc<SelfSpawner> = ctx.get().await;
      let _ = tx.send(me);
    });
  }
}

#[tokio::test]
async fn on_create_can_spawn_task_that_resolves_self() {
  let ctx = ComponentProvider::default();
  let (tx, rx) = oneshot::channel();
  let tx = Arc::new(Mutex::new(Some(tx)));

  ctx.register_factory(move |_ctx: ComponentProvider| {
    let tx = tx.clone();
    async move { Signal { tx: Mutex::new(tx.lock().take()) } }
  });
  ctx.register_on_create::<SelfSpawner>();

  let spawner = ctx.get::<SelfSpawner>().await;
  let resolved = rx.await.expect("on_create spawned task should resolve self");

  assert!(Arc::ptr_eq(&spawner, &resolved));
}
