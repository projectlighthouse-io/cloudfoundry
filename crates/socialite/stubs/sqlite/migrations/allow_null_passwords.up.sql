-- no-transaction
-- Accounts made through social login have no password until they set one.
-- SQLite cannot drop NOT NULL in place, so the table is rebuilt. Foreign keys
-- are off meanwhile, so rows that point at users are kept. If you changed the
-- users table, list your columns here.
pragma foreign_keys = off;
begin;
create table users_new (
    id integer primary key autoincrement,
    name text,
    email text not null unique,
    password_hash text,
    remember_token text,
    created_at text not null default current_timestamp
);
insert into users_new (id, name, email, password_hash, remember_token, created_at)
    select id, name, email, password_hash, remember_token, created_at from users;
drop table users;
alter table users_new rename to users;
commit;
pragma foreign_keys = on;
