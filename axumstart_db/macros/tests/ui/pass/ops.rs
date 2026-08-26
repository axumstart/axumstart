use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct ItemRow {
    id: i32,
    name: String,
    elo: i32,
    deleted_at: Option<String>,
}

#[repository(table = "item")]
trait ItemRepository: Send + Sync {
    async fn find_all_by_id_in(&self, ids: Vec<i32>) -> sqlx::Result<Vec<ItemRow>>;
    async fn find_by_name_like(&self, pattern: String) -> sqlx::Result<Option<ItemRow>>;
    async fn find_all_by_elo_gt(&self, elo: i32) -> sqlx::Result<Vec<ItemRow>>;
    async fn find_all_by_elo_gte_and_elo_lte(&self, lo: i32, hi: i32) -> sqlx::Result<Vec<ItemRow>>;
    async fn find_all_by_deleted_at_is_null(&self) -> sqlx::Result<Vec<ItemRow>>;
    async fn find_by_id_and_deleted_at_is_not_null(&self, id: i32) -> sqlx::Result<Option<ItemRow>>;
    async fn count_by_elo_lt(&self, elo: i32) -> sqlx::Result<i64>;
    async fn exists_by_name_like(&self, pattern: String) -> sqlx::Result<bool>;
    async fn delete_by_id_in(&self, ids: Vec<i32>) -> sqlx::Result<()>;
    async fn find_all_by_deleted_at_is_null_order_by_elo_desc(&self) -> sqlx::Result<Vec<ItemRow>>;
}

fn _assert_impl<T: ItemRepository>() {}

fn main() {
    _assert_impl::<DbItemRepository>();
}
