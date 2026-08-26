use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
    username: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn find_by_id(&self, id: i32, extra: i32) -> sqlx::Result<Option<UserRow>>;
    async fn set_username_by_id(&self, username: String) -> sqlx::Result<()>;
}

fn main() {}
