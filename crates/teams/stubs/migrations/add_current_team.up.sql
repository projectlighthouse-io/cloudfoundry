-- The team each user picked. Kept on users so every app instance and
-- device agrees. No foreign key: a deleted team leaves a stale id, and
-- teams clears it on the next request (SQLite cannot drop a column that
-- has one).
alter table users add column current_team_id bigint;
