use axumstart_components::{ComponentConfig, ComponentProvider};

#[derive(Clone, ComponentConfig)]
struct TestConfig {
  #[env_var("DI_TEST_REQUIRED")]
  required: String,
  #[env_var("DI_TEST_WITH_DEFAULT", 42)]
  with_default: i32,
  #[env_var("DI_TEST_ABSENT_BOOL", false)]
  absent_bool: bool,
}

#[tokio::test]
async fn component_config_reads_env_and_falls_back_to_default() {
  unsafe {
    std::env::set_var("DI_TEST_REQUIRED", "hello");
    std::env::set_var("DI_TEST_WITH_DEFAULT", "7");
    std::env::remove_var("DI_TEST_ABSENT_BOOL");
  }

  let ctx = ComponentProvider::default();
  ctx.register_all();

  let cfg = ctx.get_cloned::<TestConfig>().await;
  assert_eq!(cfg.required, "hello");
  assert_eq!(cfg.with_default, 7);
  assert!(!cfg.absent_bool);
}

#[tokio::test]
#[should_panic(expected = "Environment variable DI_TEST_MISSING_REQUIRED not set")]
async fn component_config_panics_on_missing_required_env() {
  unsafe {
    std::env::remove_var("DI_TEST_MISSING_REQUIRED");
  }

  #[derive(Clone, ComponentConfig)]
  struct MissingRequired {
    #[env_var("DI_TEST_MISSING_REQUIRED")]
    value: String,
  }

  let ctx = ComponentProvider::default();
  ctx.register_all();
  ctx.get_cloned::<MissingRequired>().await;
}
