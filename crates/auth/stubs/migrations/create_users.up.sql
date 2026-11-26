create table users (
    id bigserial primary key,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    created_at timestamptz not null default now()
);
