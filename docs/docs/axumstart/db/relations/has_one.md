# has_one

One-to-one — the foreign key lives on the **target** table, pointing back at this one.

The target foreign key must have a `UNIQUE` constraint in the database schema. The macro does not
create or inspect constraints. The quickstart's `0007_profile_user_unique.sql` migration enforces
this after existing duplicate profiles are reconciled.

```rust
#[repository(table = "user")]
#[has_one(table = "profile")]
pub trait UserRepository: Repository {
    // profile_bio → JOIN "profile" ON "profile".user_id = "user".id WHERE "profile".bio = $1 LIMIT 1
    async fn first_by_profile_bio(&self, bio: String) -> sqlx::Result<Option<UserRow>>;
}
```

Unlike `belongs_to`, there's no FK-shortcut column here — even `profile_id` would JOIN, since
there's no FK on `user` itself to resolve to (the FK lives on `profile`, pointing back).

See [relations](./relations.md) for the full attribute table and defaults.
