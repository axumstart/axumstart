use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn refresh_cache(&self) -> sqlx::Result<()>;
}

fn main() {}
