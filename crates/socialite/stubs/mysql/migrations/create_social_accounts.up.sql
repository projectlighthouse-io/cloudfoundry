-- One row per provider account linked to a user.
create table social_accounts (
    id bigint auto_increment primary key,
    user_id bigint not null,
    provider varchar(64) not null,
    provider_id varchar(191) not null,
    created_at timestamp not null default current_timestamp,
    unique key social_accounts_provider (provider, provider_id),
    foreign key (user_id) references users (id) on delete cascade
);
