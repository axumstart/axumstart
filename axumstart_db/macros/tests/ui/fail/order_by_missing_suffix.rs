use axumstart_db::repository;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct AchievementRow {
    id: i32,
    sort_order: i32,
}

#[repository(table = "achievement")]
trait AchievementRepository: Send + Sync {
    #[order_by("sort_order")]
    async fn find_all(&self) -> sqlx::Result<Vec<AchievementRow>>;
}

fn main() {}
