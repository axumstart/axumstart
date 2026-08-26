use axumstart_db::repository;
use axumstart_db::Page;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: i32,
}

#[repository(table = "user")]
trait UserRepository: Send + Sync {
    async fn find_by_id(&self, id: i32, page: Page) -> sqlx::Result<Option<UserRow>>;
}

fn main() {}
