-- no-transaction
-- Fails if any account has no password: set one first.
pragma foreign_keys = off;
begin;
create table users_new (
    id integer primary key autoincrement,
    uuid blob not null unique,
    name text,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    email_verified_at integer,
    created_at text not null default current_timestamp
);
insert into users_new (id, uuid, name, email, password_hash, remember_token, email_verified_at, created_at)
    select id, uuid, name, email, password_hash, remember_token, email_verified_at, created_at from users;
drop table users;
alter table users_new rename to users;
commit;
pragma foreign_keys = on;
