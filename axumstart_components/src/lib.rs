pub use async_trait::async_trait;
pub use axumstart_components_macros::{Component, ComponentConfig, component};
pub use inventory;
use parking_lot::Mutex;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::OnceCell;

pub mod axum;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type InjectableFactory =
  Arc<dyn Fn(ComponentProvider) -> BoxFuture<Arc<dyn Any + Send + Sync>> + Send + Sync>;
type ComponentInstances = Arc<Mutex<HashMap<TypeId, Arc<OnceCell<Arc<dyn Any + Send + Sync>>>>>>;

tokio::task_local! {
  // Scoped per factory future: nested calls inherit the chain, sibling futures and
  // spawned tasks do not share mutable cycle-detection state.
  static RESOLUTION: Vec<(usize, TypeId, &'static str)>;
}

#[derive(Default, Clone)]
pub struct ComponentProvider {
  blueprints: Arc<Mutex<HashMap<TypeId, InjectableFactory>>>,
  instances: ComponentInstances,
  names: Arc<Mutex<HashMap<TypeId, &'static str>>>,
}

#[derive(Clone)]
pub struct Components {
  components: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Components {
  pub fn len(&self) -> usize {
    self.components.len()
  }

  pub fn is_empty(&self) -> bool {
    self.components.is_empty()
  }

  pub fn get<T: ?Sized + 'static + Send + Sync>(&self) -> Option<Arc<T>> {
    self
      .components
      .get(&TypeId::of::<T>())
      .and_then(|any| any.downcast_ref::<Arc<T>>())
      .cloned()
  }

  pub fn get_cloned<T: Clone + 'static + Send + Sync>(&self) -> Option<T> {
    self.get::<T>().map(|arc| (*arc).clone())
  }
}
impl ComponentProvider {
  pub fn register<T: 'static + Send + Sync + ComponentBlueprint>(&self) {
    self.register_factory(|ctx: ComponentProvider| async move { T::new(&ctx).await });
  }

  pub fn register_factory<T, F, Fut>(&self, factory: F)
  where
    T: 'static + Send + Sync,
    F: Fn(ComponentProvider) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = T> + Send + 'static,
  {
    self
      .names
      .lock()
      .insert(TypeId::of::<T>(), std::any::type_name::<T>());
    let mut lock = self.blueprints.lock();
    lock.insert(
      TypeId::of::<T>(),
      Arc::new(move |ctx: ComponentProvider| {
        let fut = factory(ctx);
        Box::pin(async move {
          let concrete: Arc<T> = Arc::new(fut.await);
          Arc::new(concrete) as Arc<dyn Any + Send + Sync>
        }) as BoxFuture<Arc<dyn Any + Send + Sync>>
      }),
    );
  }

  pub fn register_on_create<T>(&self)
  where
    T: 'static + Send + Sync + ComponentBlueprint + OnCreate,
  {
    self.register_factory(|ctx: ComponentProvider| async move {
      let instance = T::new(&ctx).await;
      instance.on_create(&ctx).await;
      instance
    });
  }

  pub fn register_dyn<T>(&self)
  where
    T: 'static + Send + Sync + DynComponentBlueprint,
  {
    self.register::<T>();
    self.register_dyn_target::<T>();
  }

  pub fn register_on_create_dyn<T>(&self)
  where
    T: 'static + Send + Sync + DynComponentBlueprint + OnCreate,
  {
    self.register_on_create::<T>();
    self.register_dyn_target::<T>();
  }

  fn register_dyn_target<T>(&self)
  where
    T: 'static + Send + Sync + DynComponentBlueprint,
  {
    self
      .names
      .lock()
      .insert(TypeId::of::<T::Dyn>(), std::any::type_name::<T::Dyn>());
    let mut lock = self.blueprints.lock();
    lock.insert(
      TypeId::of::<T::Dyn>(),
      Arc::new(move |ctx: ComponentProvider| {
        Box::pin(async move {
          let concrete: Arc<T> = ctx.get::<T>().await;
          let trait_obj: Arc<T::Dyn> = T::upcast(concrete);
          Arc::new(trait_obj) as Arc<dyn Any + Send + Sync>
        }) as BoxFuture<Arc<dyn Any + Send + Sync>>
      }),
    );
  }

  async fn resolve_by_id(
    &self,
    type_id: TypeId,
    type_name: &'static str,
  ) -> Arc<dyn Any + Send + Sync> {
    let provider_id = Arc::as_ptr(&self.instances) as usize;
    let mut resolution = RESOLUTION.try_with(Clone::clone).unwrap_or_default();
    if resolution
      .iter()
      .any(|(provider, id, _)| *provider == provider_id && *id == type_id)
    {
      let chain: Vec<&str> = resolution.iter().map(|(_, _, name)| *name).collect();
      panic!(
        "circular dependency detected: {} -> {}",
        chain.join(" -> "),
        type_name
      );
    }
    let requested_by = resolution.last().map(|(_, _, name)| *name);
    resolution.push((provider_id, type_id, type_name));

    let once_cell = {
      let mut lock = self.instances.lock();
      lock
        .entry(type_id)
        .or_insert_with(|| Arc::new(OnceCell::new()))
        .clone()
    };

    once_cell
      .get_or_init(|| async {
        let factory = {
          let lock = self.blueprints.lock();
          match lock.get(&type_id) {
            Some(f) => f.clone(),
            None => {
              drop(lock);
              match requested_by {
                Some(parent) => panic!(
                  "no registration for type `{}` (required by `{}`)",
                  type_name, parent
                ),
                None => panic!("no registration for type `{}`", type_name),
              }
            }
          }
        };
        RESOLUTION
          .scope(resolution, async { factory(self.clone()).await })
          .await
      })
      .await
      .clone()
  }

  pub async fn get_cloned<T: Clone + 'static + Send + Sync>(&self) -> T {
    (*self.get::<T>().await).clone()
  }

  pub async fn get<T: ?Sized + 'static + Send + Sync>(&self) -> Arc<T> {
    self
      .resolve_by_id(TypeId::of::<T>(), std::any::type_name::<T>())
      .await
      .downcast_ref::<Arc<T>>()
      .expect("internal DI error: stored Any does not match TypeId key")
      .clone()
  }

  pub async fn build(&self) -> Components {
    let type_ids: Vec<TypeId> = {
      let lock = self.blueprints.lock();
      lock.keys().copied().collect()
    };

    let mut components = HashMap::new();
    for type_id in type_ids {
      let type_name = self
        .names
        .lock()
        .get(&type_id)
        .copied()
        .unwrap_or("<unknown>");
      let value = self.resolve_by_id(type_id, type_name).await;
      components.insert(type_id, value);
    }

    Components {
      components: Arc::new(components),
    }
  }

  pub fn register_all(&self) {
    for registration in inventory::iter::<ComponentRegistration> {
      registration.0(self);
    }
  }
}

pub struct ComponentRegistration(pub fn(&ComponentProvider));
inventory::collect!(ComponentRegistration);

// Macro plumbing for `#[component]` — picks the right `ComponentProvider` registration
// method (`register` / `register_dyn` / `register_on_create` / `register_on_create_dyn`)
// purely from which traits `T` implements. Not part of the public API.
//
// Implemented as nested newtypes connected by `Deref`, each with its own inherent
// `register` method under a different bound. `RegisterProbe<T>::new().register(ctx)` tries
// the outermost (most-bounded) inherent method first; if `T` doesn't satisfy that impl's
// bound, that impl simply doesn't exist for this `T`, so method lookup autoderefs to the
// next layer, and so on down to the always-applicable base case. This is the standard
// "autoref/autoderef stable specialization" idiom (inherent methods only — no trait needs
// to be in scope at the call site).
#[doc(hidden)]
pub struct RegisterPlain<T>(pub std::marker::PhantomData<T>);
#[doc(hidden)]
pub struct RegisterOnCreateTier<T>(pub RegisterPlain<T>);
#[doc(hidden)]
pub struct RegisterDynTier<T>(pub RegisterOnCreateTier<T>);
#[doc(hidden)]
pub struct RegisterProbe<T>(pub RegisterDynTier<T>);

impl<T> RegisterProbe<T> {
  pub fn new() -> Self {
    RegisterProbe(RegisterDynTier(RegisterOnCreateTier(RegisterPlain(
      std::marker::PhantomData,
    ))))
  }
}

impl<T> Default for RegisterProbe<T> {
  fn default() -> Self {
    Self::new()
  }
}

impl<T> std::ops::Deref for RegisterProbe<T> {
  type Target = RegisterDynTier<T>;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}
impl<T> std::ops::Deref for RegisterDynTier<T> {
  type Target = RegisterOnCreateTier<T>;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}
impl<T> std::ops::Deref for RegisterOnCreateTier<T> {
  type Target = RegisterPlain<T>;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

#[doc(hidden)]
impl<T> RegisterProbe<T>
where
  T: 'static + Send + Sync + ComponentBlueprint + DynComponentBlueprint + OnCreate,
{
  pub fn register(&self, ctx: &ComponentProvider) {
    ctx.register_on_create_dyn::<T>();
  }
}

#[doc(hidden)]
impl<T> RegisterDynTier<T>
where
  T: 'static + Send + Sync + ComponentBlueprint + DynComponentBlueprint,
{
  pub fn register(&self, ctx: &ComponentProvider) {
    ctx.register_dyn::<T>();
  }
}

#[doc(hidden)]
impl<T> RegisterOnCreateTier<T>
where
  T: 'static + Send + Sync + ComponentBlueprint + OnCreate,
{
  pub fn register(&self, ctx: &ComponentProvider) {
    ctx.register_on_create::<T>();
  }
}

#[doc(hidden)]
impl<T> RegisterPlain<T>
where
  T: 'static + Send + Sync + ComponentBlueprint,
{
  pub fn register(&self, ctx: &ComponentProvider) {
    ctx.register::<T>();
  }
}

// Same technique for `#[component]` factory functions — picks whether to call
// `T::on_create` after the factory produces `T`, purely from whether `T: OnCreate`.
#[doc(hidden)]
pub struct FactoryPlain<T>(pub std::marker::PhantomData<T>);
#[doc(hidden)]
pub struct FactoryProbe<T>(pub FactoryPlain<T>);

impl<T> FactoryProbe<T> {
  pub fn new() -> Self {
    FactoryProbe(FactoryPlain(std::marker::PhantomData))
  }
}

impl<T> Default for FactoryProbe<T> {
  fn default() -> Self {
    Self::new()
  }
}

impl<T> std::ops::Deref for FactoryProbe<T> {
  type Target = FactoryPlain<T>;
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

#[doc(hidden)]
impl<T> FactoryProbe<T>
where
  T: 'static + Send + Sync + OnCreate,
{
  pub fn register<F, Fut>(&self, ctx: &ComponentProvider, factory: F)
  where
    F: Fn(ComponentProvider) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = T> + Send + 'static,
  {
    ctx.register_factory(move |ctx: ComponentProvider| {
      let fut = factory(ctx.clone());
      async move {
        let instance = fut.await;
        instance.on_create(&ctx).await;
        instance
      }
    });
  }
}

#[doc(hidden)]
impl<T> FactoryPlain<T>
where
  T: 'static + Send + Sync,
{
  pub fn register<F, Fut>(&self, ctx: &ComponentProvider, factory: F)
  where
    F: Fn(ComponentProvider) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = T> + Send + 'static,
  {
    ctx.register_factory(factory);
  }
}

#[async_trait]
pub trait ComponentBlueprint {
  async fn new(ctx: &ComponentProvider) -> Self;
}

pub trait DynComponentBlueprint: ComponentBlueprint {
  type Dyn: ?Sized + 'static + Send + Sync;
  fn upcast(arc: Arc<Self>) -> Arc<Self::Dyn>;
}

#[async_trait]
pub trait OnCreate: Send + Sync {
  async fn on_create(&self, ctx: &ComponentProvider);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  #[should_panic(expected = "circular dependency detected")]
  async fn test_circular_dependency() {
    struct A {
      _b: Arc<B>,
    }
    struct B {
      _a: Arc<A>,
    }
    #[async_trait]
    impl ComponentBlueprint for A {
      async fn new(ctx: &ComponentProvider) -> Self {
        Self {
          _b: ctx.get().await,
        }
      }
    }
    #[async_trait]
    impl ComponentBlueprint for B {
      async fn new(ctx: &ComponentProvider) -> Self {
        Self {
          _a: ctx.get().await,
        }
      }
    }

    let ctx = ComponentProvider::default();
    ctx.register::<A>();
    ctx.register::<B>();
    ctx.get::<A>().await;
  }

  #[tokio::test]
  async fn concurrent_resolution_waits_instead_of_looking_like_a_cycle() {
    use tokio::sync::Notify;

    let ctx = ComponentProvider::default();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let entered_factory = entered.clone();
    let release_factory = release.clone();
    ctx.register_factory(move |_| {
      let entered = entered_factory.clone();
      let release = release_factory.clone();
      async move {
        entered.notify_one();
        release.notified().await;
        42u32
      }
    });

    let first_ctx = ctx.clone();
    let first = tokio::spawn(async move { first_ctx.get::<u32>().await });
    entered.notified().await;
    let second_ctx = ctx.clone();
    let second = tokio::spawn(async move { second_ctx.get::<u32>().await });
    tokio::task::yield_now().await;
    release.notify_one();

    let first_value = first.await.expect("first resolution should succeed");
    let second_value = second.await.expect("second resolution should succeed");
    assert_eq!(*first_value, 42);
    assert!(Arc::ptr_eq(&first_value, &second_value));
  }
}
