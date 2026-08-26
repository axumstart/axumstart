use crate::Components;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use std::any::type_name;
use std::ops::Deref;
use std::sync::Arc;

pub struct Inject<T: ?Sized + 'static + Send + Sync>(pub Arc<T>);

impl<T: ?Sized + 'static + Send + Sync> Deref for Inject<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    self.0.deref()
  }
}

impl<T: ?Sized + 'static + Send + Sync> FromRequestParts<Components> for Inject<T> {
  type Rejection = String;

  fn from_request_parts(
    _: &mut Parts,
    state: &Components,
  ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
    let component = state.get::<T>();

    async move {
      match component {
        None => Err(format!("No component found for {:?}", type_name::<T>())),
        Some(component) => Ok(Self(component)),
      }
    }
  }
}
