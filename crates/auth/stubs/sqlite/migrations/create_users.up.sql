create table users (
    id integer primary key autoincrement,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    created_at text not null default current_timestamp
);

-- API tokens (`.tokens()` on the auth layer). Only SHA-256 hashes are stored.
create table api_tokens (
    id integer primary key autoincrement,
    user_id integer not null references users (id) on delete cascade,
    name text not null,
    token_hash text not null unique,
    expires_at integer,
    created_at text not null default current_timestamp
);
