use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
    username: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn find_by_usrname(&self, usrname: String) -> sqlx::Result<Option<UserRow>>;
}

fn main() {}
