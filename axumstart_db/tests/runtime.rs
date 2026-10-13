use axumstart_db::{DbPool, Page, SqlxInsert, SqlxUpdate, quote_identifier, repository};

#[derive(SqlxInsert)]
struct Values {
  id: i64,
  name: String,
}

#[derive(SqlxUpdate)]
struct Update {
  #[key]
  id: i64,
  name: String,
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
  id: i64,
  name: String,
}

#[repository(table = "order")]
#[belongs_to(table = "owner")]
trait Orders: Send + Sync {
  async fn insert(&self, values: Values) -> sqlx::Result<()>;
  async fn update(&self, values: Update) -> sqlx::Result<Row>;
  #[unique(id)]
  async fn upsert(&self, values: Values) -> sqlx::Result<Row>;
  async fn insert_all(&self, values: Vec<Values>) -> sqlx::Result<()>;
  async fn find_all_by_owner_name_and_id(&self, name: &str, id: i64) -> sqlx::Result<Vec<Row>>;
  async fn find_all_by_owner_name_order_by_id_desc(&self, name: &str) -> sqlx::Result<Vec<Row>>;
  async fn find_all_order_by_id(&self, page: Page) -> sqlx::Result<Vec<Row>>;
  async fn first_by_id(&self, id: i64) -> sqlx::Result<Option<Row>>;
  async fn count_by_id_in(&self, ids: Vec<i64>) -> sqlx::Result<i64>;
  async fn exists_by_id(&self, id: i64) -> sqlx::Result<bool>;
  async fn find_random_by_id_in(&self, ids: Vec<i64>) -> sqlx::Result<Option<Row>>;
  async fn set_name_by_id_in(&self, name: &str, ids: Vec<i64>) -> sqlx::Result<()>;
  #[transactional]
  async fn delete_by_id_in(&self, ids: Vec<i64>) -> sqlx::Result<()>;
}

#[repository(table = "archive")]
trait Archive: Send + Sync {
  async fn insert(&self, values: Values) -> sqlx::Result<()>;
  async fn update(&self, values: Update) -> sqlx::Result<Row>;
  #[unique(name)]
  async fn upsert(&self, values: Values) -> sqlx::Result<Row>;
}

async fn pool() -> DbPool {
  #[cfg(feature = "sqlite")]
  let url = "sqlite::memory:".to_owned();
  #[cfg(not(feature = "sqlite"))]
  let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a disposable test database");
  sqlx::pool::PoolOptions::new()
    .max_connections(1)
    .connect(&url)
    .await
    .unwrap()
}

#[tokio::test]
#[cfg_attr(not(feature = "sqlite"), ignore = "requires a disposable DATABASE_URL")]
async fn generated_queries_preserve_tables_and_filter_semantics() -> sqlx::Result<()> {
  let pool = pool().await;
  for table in ["order", "archive", "owner"] {
    sqlx::query(sqlx::AssertSqlSafe(format!(
      "DROP TABLE IF EXISTS {}",
      quote_identifier(table)
    )))
    .execute(&pool)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE TABLE {} (id BIGINT PRIMARY KEY, name VARCHAR(100) UNIQUE, owner_id BIGINT DEFAULT 1)", quote_identifier(table)))).execute(&pool).await?;
  }
  sqlx::query("INSERT INTO owner (id, name) VALUES (1, 'Ada')")
    .execute(&pool)
    .await?;
  let orders = DbOrders::new(pool.clone());
  let archive = DbArchive::new(pool.clone());
  orders
    .insert(Values {
      id: 1,
      name: "order".into(),
    })
    .await?;
  archive
    .insert(Values {
      id: 1,
      name: "archive".into(),
    })
    .await?;
  assert_eq!(orders.first_by_id(1).await?.unwrap().name, "order");
  assert_eq!(
    orders
      .update(Update {
        id: 1,
        name: "updated-order".into()
      })
      .await?
      .name,
    "updated-order"
  );
  assert_eq!(
    archive
      .update(Update {
        id: 1,
        name: "updated-archive".into()
      })
      .await?
      .name,
    "updated-archive"
  );
  assert_eq!(orders.first_by_id(1).await?.unwrap().name, "updated-order");
  assert_eq!(
    orders
      .upsert(Values {
        id: 1,
        name: "upserted-order".into()
      })
      .await?
      .id,
    1
  );
  assert_eq!(
    archive
      .upsert(Values {
        id: 2,
        name: "updated-archive".into()
      })
      .await?
      .id,
    2
  );
  assert_eq!(
    orders.find_all_by_owner_name_and_id("Ada", 1).await?.len(),
    1
  );
  assert_eq!(
    orders
      .find_all_by_owner_name_order_by_id_desc("Ada")
      .await?
      .len(),
    1
  );
  assert_eq!(orders.count_by_id_in(vec![]).await?, 0);
  orders.set_name_by_id_in("should-not-write", vec![]).await?;
  orders.delete_by_id_in(vec![]).await?;
  assert!(orders.exists_by_id(1).await?);
  assert_eq!(orders.find_random_by_id_in(vec![1]).await?.unwrap().id, 1);
  orders.set_name_by_id_in("renamed", vec![1]).await?;
  assert_eq!(orders.first_by_id(1).await?.unwrap().name, "renamed");
  orders
    .insert_all(vec![
      Values {
        id: 3,
        name: "third".into(),
      },
      Values {
        id: 4,
        name: "fourth".into(),
      },
    ])
    .await?;
  assert_eq!(orders.find_all_order_by_id(Page::new(1, 1)).await?[0].id, 3);
  assert!(matches!(
    orders.find_all_order_by_id(Page::new(-1, 0)).await,
    Err(sqlx::Error::Protocol(_))
  ));
  let mut tx = pool.begin().await?;
  orders.delete_by_id_in_tx(&mut tx, vec![1]).await?;
  tx.rollback().await?;
  assert!(orders.exists_by_id(1).await?);
  orders.delete_by_id_in(vec![1]).await?;
  assert!(!orders.exists_by_id(1).await?);
  Ok(())
}

#[cfg(feature = "sqlite")]
mod sqlite {
  use super::*;

  #[derive(sqlx::FromRow)]
  struct DatedRow {
    id: i64,
    name: String,
    created_at: String,
  }

  #[repository(table = "dated")]
  trait Dated: Send + Sync {
    async fn find_all_by_id_or_name_this_week(
      &self,
      id: i64,
      name: &str,
    ) -> sqlx::Result<Vec<DatedRow>>;
    async fn find_all_by_id_in_or_name_this_week(
      &self,
      ids: Vec<i64>,
      name: &str,
    ) -> sqlx::Result<Vec<DatedRow>>;
  }

  #[tokio::test]
  async fn week_boundaries_and_or_grouping() -> sqlx::Result<()> {
    let pool = pool().await;
    for (day, monday) in [
      ("2026-10-12", "2026-10-12"),
      ("2026-10-18", "2026-10-12"),
      ("2026-10-11", "2026-10-05"),
    ] {
      let expression =
        axumstart_db::WEEK_START_SQL.replace("CURRENT_TIMESTAMP", &format!("'{day}'"));
      let actual: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT {expression}")))
        .fetch_one(&pool)
        .await?;
      assert_eq!(actual, monday);
    }
    sqlx::raw_sql("CREATE TABLE dated (id INTEGER, name TEXT, created_at TEXT); INSERT INTO dated VALUES (1, 'old', '2000-01-01'); INSERT INTO dated VALUES (2, 'current', CURRENT_TIMESTAMP);").execute(&pool).await?;
    let repo = DbDated::new(pool);
    let rows = repo.find_all_by_id_or_name_this_week(1, "current").await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 2);
    assert_eq!(rows[0].name, "current");
    assert!(rows[0].created_at.starts_with("20"));
    assert_eq!(
      repo
        .find_all_by_id_in_or_name_this_week(vec![1], "current")
        .await?
        .len(),
      1
    );
    Ok(())
  }
}
