use axumstart_db::repository;

#[repository(table = "item")]
#[belongs_to(table = "owner")]
trait Items: Send + Sync {
    async fn set_name_by_owner_name(&self, name: &str, owner: &str) -> sqlx::Result<()>;
}

fn main() {}
