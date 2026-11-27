create table users (
    id bigserial primary key,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    created_at timestamptz not null default now()
);

-- API tokens (`.tokens()` on the auth layer). Only SHA-256 hashes are stored.
create table api_tokens (
    id bigserial primary key,
    user_id bigint not null references users (id) on delete cascade,
    name text not null,
    token_hash text not null unique,
    expires_at bigint,
    created_at timestamptz not null default now()
);
