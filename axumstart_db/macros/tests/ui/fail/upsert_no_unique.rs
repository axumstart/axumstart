use axumstart_db::{repository, SqlxInsert};

#[derive(SqlxInsert)]
struct UserValues {
    username: String,
}

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
    username: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn upsert(&self, values: UserValues) -> sqlx::Result<UserRow>;
}

fn main() {}
