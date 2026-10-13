-- has_one(profile) requires at most one profile for each user.
-- Existing duplicate profiles must be reconciled explicitly before applying this migration.
ALTER TABLE profile ADD CONSTRAINT profile_user_id_unique UNIQUE (user_id);
