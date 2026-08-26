use axumstart_components::{ComponentProvider, Component};
use std::sync::Arc;

trait Messager: Send + Sync {
  fn message(&self);
}

#[derive(Component)]
struct Idk;

#[derive(Component)]
#[as_trait(Messager)]
struct Lol {
  _idk: Arc<Idk>,
}

impl Messager for Lol {
  fn message(&self) {}
}

#[tokio::test]
async fn test_basic_resolution() {
  let ctx = ComponentProvider::default();
  ctx.register::<Idk>();
  ctx.register_dyn::<Lol>();

  let components = ctx.build().await;
  assert!(components.get::<Lol>().is_some());
  assert!(components.get::<dyn Messager>().is_some());
  assert_eq!(components.len(), 3);
}

#[tokio::test]
async fn test_get_cloned() {
  #[derive(Component)]
  #[derive(Clone)]
  struct Cheap {
    inner: Arc<Idk>,
  }

  let ctx = ComponentProvider::default();
  ctx.register::<Idk>();
  ctx.register::<Cheap>();

  let owned: Cheap = ctx.get_cloned::<Cheap>().await;
  let _ = owned.inner;
}

#[tokio::test]
async fn test_default_field() {
  #[derive(Component)]
  struct WithDefault {
    _idk: Arc<Idk>,
    #[default]
    count: u32,
  }

  let ctx = ComponentProvider::default();
  ctx.register::<Idk>();
  ctx.register::<WithDefault>();

  let c = ctx.get::<WithDefault>().await;
  assert_eq!(c.count, 0);
}
