use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
    username: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    #[unique(username)]
    async fn find_by_username(&self, username: String) -> sqlx::Result<Option<UserRow>>;
}

fn main() {}
