-- Email verification. Times are unix seconds.
alter table users add column email_verified_at integer;

-- Links that verify an address: a new account's, or one the user changes to
-- (it replaces `users.email` once verified). Only SHA-256 hashes of the tokens.
create table email_verifications (
    id integer primary key autoincrement,
    user_id integer not null references users (id),
    email text not null,
    token_hash blob not null unique,
    expires_at integer not null,
    verified_at integer,
    created_at integer not null
);

create index email_verifications_user on email_verifications (user_id);
