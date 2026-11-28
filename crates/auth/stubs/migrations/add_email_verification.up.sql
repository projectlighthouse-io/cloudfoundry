-- Email verification. Times are unix seconds.
alter table users add column email_verified_at bigint;

-- Links that verify an address: a new account's, or one the user changes to
-- (it replaces `users.email` once verified). Only SHA-256 hashes of the tokens.
create table email_verifications (
    id bigserial primary key,
    user_id bigint not null references users (id),
    email text not null,
    token_hash bytea not null unique,
    expires_at bigint not null,
    verified_at bigint,
    created_at bigint not null
);

create index email_verifications_user on email_verifications (user_id);
