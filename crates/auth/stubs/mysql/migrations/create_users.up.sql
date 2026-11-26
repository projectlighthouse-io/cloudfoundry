create table users (
    id bigint auto_increment primary key,
    email varchar(191) not null unique,
    password_hash varchar(255) not null,
    remember_token varchar(100),
    created_at timestamp not null default current_timestamp
);
