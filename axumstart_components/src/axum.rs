use crate::Components;
use axum::extract::FromRequestParts;
use axum::http::{StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use std::any::type_name;
use std::ops::Deref;
use std::sync::Arc;

pub struct Inject<T: ?Sized + 'static + Send + Sync>(pub Arc<T>);

#[derive(Debug)]
pub struct MissingComponent {
  type_name: &'static str,
}

impl IntoResponse for MissingComponent {
  fn into_response(self) -> Response {
    eprintln!("missing component: {}", self.type_name);
    (StatusCode::INTERNAL_SERVER_ERROR, "component unavailable").into_response()
  }
}

impl<T: ?Sized + 'static + Send + Sync> Deref for Inject<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    self.0.deref()
  }
}

impl<T: ?Sized + 'static + Send + Sync> FromRequestParts<Components> for Inject<T> {
  type Rejection = MissingComponent;

  fn from_request_parts(
    _: &mut Parts,
    state: &Components,
  ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
    let component = state.get::<T>();

    async move {
      match component {
        None => Err(MissingComponent {
          type_name: type_name::<T>(),
        }),
        Some(component) => Ok(Self(component)),
      }
    }
  }
}
