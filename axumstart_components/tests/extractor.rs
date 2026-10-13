use axum::extract::FromRequestParts;
use axum::http::Request;
use axum::response::IntoResponse;
use axumstart_components::{ComponentProvider, Components, axum::Inject};

#[tokio::test]
async fn missing_component_is_an_internal_server_error() {
  let components: Components = ComponentProvider::default().build().await;
  let mut parts = Request::new(()).into_parts().0;
  let result =
    <Inject<u32> as FromRequestParts<Components>>::from_request_parts(&mut parts, &components)
      .await;
  let rejection = match result {
    Err(rejection) => rejection,
    Ok(_) => panic!("unregistered component should reject"),
  };

  assert_eq!(
    rejection.into_response().status(),
    axum::http::StatusCode::INTERNAL_SERVER_ERROR
  );
}
