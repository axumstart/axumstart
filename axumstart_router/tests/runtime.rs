use axum::{
  body::{Body, to_bytes},
  http::{Request, StatusCode},
  middleware::{Next, from_fn},
  response::Response,
};
use axumstart_components::ComponentProvider;
use axumstart_router::{Inject, get, post, register_routes};
use tower::ServiceExt;

async fn add_header(request: Request<Body>, next: Next) -> Response {
  let mut response = next.run(request).await;
  response
    .headers_mut()
    .insert("x-layer", "yes".parse().unwrap());
  response
}

#[get("/registered")]
#[layer(from_fn(add_header))]
async fn registered(value: Inject<u32>) -> String {
  value.to_string()
}

#[post("/plain")]
async fn plain() -> &'static str {
  "plain"
}

#[get("/missing")]
async fn missing(_: Inject<String>) -> &'static str {
  "unreachable"
}

#[get("/disabled")]
#[cfg(any())]
async fn disabled() -> &'static str {
  "unreachable"
}

#[tokio::test]
async fn registration_layers_and_injection_work_over_http() {
  let provider = ComponentProvider::default();
  provider.register_factory(|_| async { 42u32 });
  let state = provider.build().await;
  let app = register_routes(axum::Router::new(), &state).with_state(state);
  for (method, path, status, body, layered) in [
    ("GET", "/registered", StatusCode::OK, "42", true),
    ("POST", "/plain", StatusCode::OK, "plain", false),
    (
      "GET",
      "/missing",
      StatusCode::INTERNAL_SERVER_ERROR,
      "component unavailable",
      false,
    ),
    ("GET", "/disabled", StatusCode::NOT_FOUND, "", false),
  ] {
    let response = app
      .clone()
      .oneshot(
        Request::builder()
          .method(method)
          .uri(path)
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(response.headers().contains_key("x-layer"), layered);
    assert_eq!(
      to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
      body.as_bytes()
    );
  }
}
