use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct AchievementRow {
    id: i32,
}

#[repository(table = "achievement")]
trait AchievementRepository: Send + Sync {
    #[order_by("id")]
    async fn count_by_active_ordered(&self, active: bool) -> sqlx::Result<i64>;
}

fn main() {}
