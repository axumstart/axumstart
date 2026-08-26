use axumstart_db::repository;
use axumstart_db::Page;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct PostRow {
    id: i32,
    user_id: i32,
    created_at: i64,
}

#[repository(table = "post")]
trait PostRepository: Send + Sync {
    async fn find_all(&self, page: Page) -> sqlx::Result<Vec<PostRow>>;
    async fn find_all_order_by_created_at_desc(&self, page: Page) -> sqlx::Result<Vec<PostRow>>;
    async fn find_all_by_user_id(&self, user_id: i32, page: Page) -> sqlx::Result<Vec<PostRow>>;
    async fn find_all_by_user_id_order_by_id_desc(
        &self,
        user_id: i32,
        page: Page,
    ) -> sqlx::Result<Vec<PostRow>>;
}

fn _assert_impl<T: PostRepository>() {}

fn main() {
    _assert_impl::<DbPostRepository>();
    let _ = Page::new(10, 0);
    let _ = Page::number(2, 25);
}
