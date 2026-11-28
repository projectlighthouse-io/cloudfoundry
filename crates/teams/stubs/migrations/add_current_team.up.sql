-- The team each user lands on. Kept on users so every app instance and
-- device agrees. No foreign key: a deleted team leaves a stale id, and
-- teams falls back to the user's first team (SQLite cannot drop a column
-- that has one).
alter table users add column current_team_id bigint;
