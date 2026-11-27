create table users (
    id bigint auto_increment primary key,
    name varchar(255),
    email varchar(191) not null unique,
    password_hash varchar(255) not null,
    remember_token varchar(100),
    created_at timestamp not null default current_timestamp
);

-- API tokens (`.tokens()` on the auth layer). Only SHA-256 hashes are stored.
create table api_tokens (
    id bigint auto_increment primary key,
    user_id bigint not null,
    name varchar(255) not null,
    token_hash char(64) not null unique,
    expires_at bigint,
    created_at timestamp not null default current_timestamp,
    foreign key (user_id) references users (id) on delete cascade
);

-- Password reset tokens: one per user, SHA-256 hash, unix-seconds creation time.
create table password_resets (
    user_id bigint primary key,
    token_hash char(64) not null,
    created_at bigint not null,
    foreign key (user_id) references users (id) on delete cascade
);
