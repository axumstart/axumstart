use axumstart_db::SqlxUpdate;

#[derive(SqlxUpdate)]
#[allow(dead_code)]
struct UserRowUpdate {
    id: i32,
    username: String,
}

fn main() {}
