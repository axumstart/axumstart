use axumstart_db::repository;

#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    category: String,
}

#[repository(table = "post")]
trait PostRepository: Send + Sync {
    async fn find_random_by_category(&self, category: String) -> sqlx::Result<PostRow>;
}

fn _assert_impl<T: PostRepository>() {}

fn main() {
    _assert_impl::<DbPostRepository>();
}
