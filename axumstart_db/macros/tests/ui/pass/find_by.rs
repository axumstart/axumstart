use axumstart_db::repository;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i32,
    name: String,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn find_by_id(&self, id: i32) -> sqlx::Result<Option<UserRow>>;
    async fn find_all_by_name(&self, name: String) -> sqlx::Result<Vec<UserRow>>;
    // filters on a column that is not part of the projected row
    #[unchecked_columns]
    async fn find_by_token_hash(&self, hash: String) -> sqlx::Result<Option<UserRow>>;
}

fn _assert_impl<T: UserRepository>() {}

fn main() {
    _assert_impl::<DbUserRepository>();
}
