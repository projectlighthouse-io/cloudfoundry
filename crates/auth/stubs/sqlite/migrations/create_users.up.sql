create table users (
    id integer primary key autoincrement,
    email text not null unique,
    password_hash text not null,
    remember_token text,
    created_at text not null default current_timestamp
);
