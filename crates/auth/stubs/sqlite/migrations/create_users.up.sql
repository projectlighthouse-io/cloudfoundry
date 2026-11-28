create table users (
    id integer primary key autoincrement,
    -- Names the user in URLs and JSON (`db::Uuid<User>`); `id` stays internal.
    uuid blob not null unique,
    name text,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    created_at text not null default current_timestamp
);

-- API tokens (`.tokens()` on the auth layer). Only SHA-256 hashes are stored.
create table api_tokens (
    id integer primary key autoincrement,
    -- The UUIDv7 that names the token in URLs and JSON (`db::Uuid<ApiToken>`).
    uuid blob not null unique,
    user_id integer not null references users (id),
    name text not null,
    token_hash text not null unique,
    expires_at integer,
    created_at text not null default current_timestamp
);

-- Password reset tokens: one per user, SHA-256 hash, unix-seconds creation time.
create table password_resets (
    user_id integer primary key references users (id),
    token_hash text not null,
    created_at integer not null
);
