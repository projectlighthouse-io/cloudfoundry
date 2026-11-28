-- Accounts made through social login have no password until they set one.
-- auth's `users` table allows a null `password_hash` already, so there is
-- nothing to do. (SQLite cannot drop NOT NULL in place; rebuilding `users`
-- here would lose the columns other modules add.) Apps whose users table
-- predates that: rebuild it by hand with `password_hash text` nullable,
-- keeping every column it has now.
select 1;
