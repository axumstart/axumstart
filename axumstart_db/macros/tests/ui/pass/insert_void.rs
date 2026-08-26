use axumstart_db::{repository, SqlxInsert};

#[derive(SqlxInsert)]
struct EventValues {
    kind: String,
    payload: String,
}

#[repository(table = "event")]
trait EventRepository: Send + Sync {
    // No row data wanted back — skips RETURNING (or, on MySQL, the select-back).
    async fn insert(&self, values: EventValues) -> sqlx::Result<()>;
    async fn insert_all(&self, values: Vec<EventValues>) -> sqlx::Result<()>;
}

fn _assert_impl<T: EventRepository>() {}

fn main() {
    _assert_impl::<DbEventRepository>();
}
