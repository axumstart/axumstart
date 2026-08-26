use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct OrderRow {
    id: i32,
    user_id: i32,
    total: i64,
}

#[repository(table = "order", join(user))]
trait OrderRepository: Send + Sync {
    // joined column — resolves through JOIN "user"
    async fn find_all_by_user_email(&self, email: String) -> sqlx::Result<Vec<OrderRow>>;
    // FK shortcut — no JOIN, queries order.user_id directly
    async fn find_all_by_user_id(&self, user_id: i32) -> sqlx::Result<Vec<OrderRow>>;
    #[transactional]
    async fn find_by_id(&self, id: i32) -> sqlx::Result<Option<OrderRow>>;
    #[transactional]
    async fn delete_by_id(&self, id: i32) -> sqlx::Result<()>;
}

fn _assert_impl<T: OrderRepository>() {}

fn main() {
    _assert_impl::<DbOrderRepository>();
}
