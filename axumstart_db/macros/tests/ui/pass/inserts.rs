use axumstart_db::{repository, SqlxInsert, SqlxUpdate};

#[derive(SqlxInsert)]
struct UserValues {
    username: String,
    email: String,
}

#[derive(SqlxUpdate)]
#[allow(dead_code)]
struct UserRowUpdate {
    #[key]
    id: i32,
    username: String,
    email: String,
}

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
    username: String,
    email: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn insert(&self, values: UserValues) -> sqlx::Result<UserRow>;
    async fn insert_or_ignore(&self, values: UserValues) -> sqlx::Result<()>;
    async fn insert_all(&self, values: Vec<UserValues>) -> sqlx::Result<Vec<UserRow>>;
    #[unique(username)]
    async fn upsert(&self, values: UserValues) -> sqlx::Result<UserRow>;
    async fn update(&self, row: UserRowUpdate) -> sqlx::Result<UserRow>;
}

fn _assert_impl<T: UserRepository>() {}

fn main() {
    _assert_impl::<DbUserRepository>();
}
