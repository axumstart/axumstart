use axumstart_db::repository;

#[repository(table = "item")]
#[belongs_to(table = "owner")]
#[has_one(table = "owner")]
trait Items: Send + Sync {}

fn main() {}
