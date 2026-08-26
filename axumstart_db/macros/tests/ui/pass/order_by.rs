use axumstart_db::repository;

#[derive(sqlx::FromRow)]
struct AchievementRow {
    id: i32,
    sort_order: i32,
    active: bool,
}

#[repository(table = "achievement")]
trait AchievementRepository: Send + Sync {
    #[order_by("sort_order", "id")]
    async fn find_all_active_ordered(&self) -> sqlx::Result<Vec<AchievementRow>>;

    #[order_by("sort_order")]
    async fn find_all_by_active_ordered(&self, active: bool) -> sqlx::Result<Vec<AchievementRow>>;

    #[order_by("sort_order")]
    async fn find_by_id_ordered(&self, id: i32) -> sqlx::Result<Option<AchievementRow>>;
}

fn _assert_impl<T: AchievementRepository>() {}

fn main() {
    _assert_impl::<DbAchievementRepository>();
}
