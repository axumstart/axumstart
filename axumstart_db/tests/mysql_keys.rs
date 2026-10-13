#![cfg(feature = "mysql")]

use axumstart_db::{SqlxInsert, repository};

#[derive(SqlxInsert)]
#[key(code)]
struct CustomValues {
  code: String,
  name: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CustomRow {
  code: String,
  name: String,
}

#[derive(SqlxInsert)]
struct GeneratedValues {
  name: String,
}

#[derive(Debug, sqlx::FromRow)]
struct GeneratedRow {
  id: i64,
  name: String,
}

#[repository(table = "custom_keys")]
trait Custom: Send + Sync {
  async fn insert(&self, values: CustomValues) -> sqlx::Result<CustomRow>;
  async fn insert_all(&self, values: Vec<CustomValues>) -> sqlx::Result<Vec<CustomRow>>;
}

#[repository(table = "generated_keys")]
trait Generated: Send + Sync {
  async fn insert(&self, values: GeneratedValues) -> sqlx::Result<GeneratedRow>;
  async fn insert_all(&self, values: Vec<GeneratedValues>) -> sqlx::Result<Vec<GeneratedRow>>;
}

#[tokio::test]
#[ignore = "requires a disposable DATABASE_URL"]
async fn returns_actual_keys_and_rejects_unsupported_batches_before_writing() -> sqlx::Result<()> {
  let pool = sqlx::mysql::MySqlPoolOptions::new()
    .max_connections(1)
    .connect(&std::env::var("DATABASE_URL").expect("set disposable DATABASE_URL"))
    .await?;
  for sql in [
    "DROP TABLE IF EXISTS custom_keys",
    "DROP TABLE IF EXISTS generated_keys",
    "CREATE TABLE custom_keys (code VARCHAR(100) PRIMARY KEY, name VARCHAR(100))",
    "CREATE TABLE generated_keys (id BIGINT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(100))",
  ] {
    sqlx::query(sql).execute(&pool).await?;
  }
  let custom = DbCustom::new(pool.clone());
  let row = custom
    .insert(CustomValues {
      code: "key-1".into(),
      name: "first".into(),
    })
    .await?;
  assert_eq!((row.code.as_str(), row.name.as_str()), ("key-1", "first"));
  let mut rows = custom
    .insert_all(vec![
      CustomValues {
        code: "key-50".into(),
        name: "second".into(),
      },
      CustomValues {
        code: "key-99".into(),
        name: "third".into(),
      },
    ])
    .await?;
  rows.sort_by(|a, b| a.code.cmp(&b.code));
  assert_eq!(
    rows.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(),
    ["key-50", "key-99"]
  );
  let generated = DbGenerated::new(pool.clone());
  let row = generated
    .insert(GeneratedValues {
      name: "generated".into(),
    })
    .await?;
  assert!(row.id > 0);
  assert_eq!(row.name, "generated");
  assert!(matches!(
    generated
      .insert_all(vec![GeneratedValues {
        name: "reject".into()
      }])
      .await,
    Err(sqlx::Error::Protocol(_))
  ));
  assert_eq!(
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generated_keys")
      .fetch_one(&pool)
      .await?,
    1
  );
  sqlx::query("ALTER TABLE generated_keys MODIFY id BIGINT NOT NULL DEFAULT 7")
    .execute(&pool)
    .await?;
  assert!(matches!(
    generated
      .insert(GeneratedValues {
        name: "reject-single".into()
      })
      .await,
    Err(sqlx::Error::Protocol(_))
  ));
  assert_eq!(
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM generated_keys")
      .fetch_one(&pool)
      .await?,
    1
  );
  Ok(())
}
