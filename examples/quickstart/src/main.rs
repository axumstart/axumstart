//! Verifies that consumers depending only on `axumstart` (not the sub-crates directly)
//! can use the re-exported macros without hitting E0433 on `axumstart_components`/
//! `axumstart_db` — including the `#[repository(component)]` path, which needs both.
//!
//! Also doubles as a runnable reference for the `#[repository]` DSL: CRUD basics
//! (`insert`, pagination, `#[transactional]`) plus all four relation kinds (`belongs_to`,
//! `has_one`, `has_many`, `belongs_to_many`). `cargo test` only exercises macro expansion
//! (no query ever runs), so a database isn't required for that. `cargo run` is a real
//! scratchpad: bring up `compose.yaml`, `sqlx migrate run`, then edit `main()`'s "quick
//! actions" section freely to poke at the database directly.

use axumstart::components::{Component, ComponentProvider, component};
use axumstart::db::{DbPool, Page, Repository, SqlxInsert, repository};
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;

#[derive(Component)]
#[allow(dead_code)]
struct Settings {
  #[default]
  port: u16,
}

#[allow(dead_code)]
impl Settings {
  fn port(&self) -> u16 {
    self.port
  }
}

#[derive(FromRow)]
pub struct User {
  pub id: i32,
  pub name: String,
}

#[derive(SqlxInsert)]
pub struct UserValues {
  pub name: String,
}

#[repository(table = "user", component)]
#[has_one(table = "profile")]
#[has_many(table = "order")]
pub trait UserRepository: Repository {
  async fn insert(&self, values: UserValues) -> sqlx::Result<User>;

  async fn find_all_by_name(&self, name: &str) -> sqlx::Result<Vec<User>>;

  async fn exists_by_id(&self, id: i32) -> sqlx::Result<bool>;

  // has_one — at most one profile per user via "profile".user_id; first_by_* is fine
  // since has_one, unlike has_many, can never match more than one row.
  async fn first_by_profile_bio(&self, bio: &str) -> sqlx::Result<Option<User>>;

  // has_many — a user can have many orders, so this column is only legal on
  // find_all_by_*; referencing it from first_by_*/count_by_*/etc. is a compile error.
  async fn find_all_by_order_status(&self, status: &str) -> sqlx::Result<Vec<User>>;
}

#[derive(FromRow)]
pub struct Order {
  pub id: i32,
  pub user_id: i32,
  pub total_cents: i64,
}

#[derive(SqlxInsert)]
pub struct OrderValues {
  pub user_id: i32,
  pub total_cents: i64,
}

#[repository(table = "order", component)]
#[belongs_to(table = "user")]
pub trait OrderRepository: Repository {
  // belongs_to FK shortcut — "user_id" resolves straight to the FK column on "order",
  // no JOIN needed.
  async fn find_all_by_user_id(&self, user_id: i32) -> sqlx::Result<Vec<Order>>;

  // belongs_to joined column — JOIN "user" ON "user".id = "order".user_id
  async fn find_all_by_user_name(&self, name: &str) -> sqlx::Result<Vec<Order>>;

  async fn find_all(&self, page: Page) -> sqlx::Result<Vec<Order>>;

  async fn insert(&self, values: OrderValues) -> sqlx::Result<Order>;

  #[transactional]
  async fn delete_by_id(&self, id: i32) -> sqlx::Result<()>;
}

#[derive(FromRow)]
pub struct Post {
  pub id: i32,
  pub title: String,
}

#[derive(SqlxInsert)]
pub struct PostValues {
  pub title: String,
}

#[repository(table = "post", component)]
#[belongs_to_many(table = "tag", through = "post_tag")]
pub trait PostRepository: Repository {
  // belongs_to_many — double JOIN through the "post_tag" pivot table; only legal on
  // find_all_by_*, same fan-out restriction as has_many.
  async fn find_all_by_tag_name(&self, name: &str) -> sqlx::Result<Vec<Post>>;

  async fn insert(&self, values: PostValues) -> sqlx::Result<Post>;
}

#[derive(FromRow)]
pub struct Tag {
  id: i32,
  #[allow(dead_code)]
  name: String,
}

#[derive(SqlxInsert)]
pub struct TagValues {
  pub name: String,
}

#[repository(table = "tag", component)]
#[belongs_to_many(table = "post", through = "post_tag")]
pub trait TagRepository: Repository {
  async fn insert(&self, values: TagValues) -> sqlx::Result<Tag>;

  async fn find_all_by_post_id(&self, post_id: i32) -> sqlx::Result<Vec<Tag>>;
}

#[component]
async fn pool() -> PgPool {
  let database_url =
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set — see .env / compose.yaml");

  PgPoolOptions::new()
    .connect(&database_url)
    .await
    .expect("Database connection failed")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> sqlx::Result<()> {
  dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

  let component_provider = ComponentProvider::default();
  component_provider.register_all();
  let components = component_provider.build().await;

  let users: Arc<dyn UserRepository> = components
    .get()
    .expect("UserRepository component not registered");
  let orders: Arc<dyn OrderRepository> = components
    .get()
    .expect("OrderRepository component not registered");
  let posts: Arc<dyn PostRepository> = components
    .get()
    .expect("PostRepository component not registered");
  let tags: Arc<dyn TagRepository> = components
    .get()
    .expect("TagRepository component not registered");

  let pool: DbPool = components
    .get_cloned()
    .expect("DbPool component not registered");

  // ---- quick actions — edit this section freely, it's a scratchpad ----

  let user = users.insert(UserValues { name: "Ada".into() }).await?;

  sqlx::query("INSERT INTO profile (user_id, bio) VALUES ($1, $2)")
    .bind(user.id)
    .bind("Loves compilers")
    .execute(&pool)
    .await?;
  orders
    .insert(OrderValues {
      user_id: user.id,
      total_cents: 4_200,
    })
    .await?;
  let post = posts
    .insert(PostValues {
      title: "Hello, relations".into(),
    })
    .await?;
  let tag = tags
    .insert(TagValues {
      name: "rust".into(),
    })
    .await?;

  sqlx::query("INSERT INTO post_tag (post_id, tag_id) VALUES ($1, $2)")
    .bind(post.id)
    .bind(tag.id)
    .execute(&pool)
    .await?;

  println!(
    "exists_by_id:          {:?}",
    users.exists_by_id(user.id).await?
  );
  println!(
    "has_one (profile):     {:?}",
    users
      .first_by_profile_bio("Loves compilers")
      .await?
      .map(|u| u.name)
  );
  println!(
    "has_many (order):      {} match(es)",
    users.find_all_by_order_status("pending").await?.len()
  );
  println!(
    "belongs_to (user):     {} match(es)",
    orders.find_all_by_user_name("Ada").await?.len()
  );
  println!(
    "belongs_to_many (tag): {} match(es)",
    posts.find_all_by_tag_name("rust").await?.len()
  );
  println!(
    "reverse belongs_to_many: {} match(es)",
    tags.find_all_by_post_id(post.id).await?.len()
  );
  println!(
    "pagination:            {} order(s) on page 1",
    orders.find_all(Page::new(10, 0)).await?.len()
  );

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use axumstart::components::ComponentProvider;

  #[tokio::test]
  async fn components_resolve_through_the_axumstart_meta_crate() {
    let ctx = ComponentProvider::default();
    ctx.register::<Settings>();
    let components = ctx.build().await;
    let settings = components
      .get::<Settings>()
      .expect("Settings should resolve");
    assert_eq!(settings.port(), 0);
  }
}
