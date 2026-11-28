-- One row per provider account linked to a user.
create table social_accounts (
    id integer primary key autoincrement,
    user_id integer not null references users (id),
    provider text not null,
    provider_id text not null,
    created_at text not null default current_timestamp,
    unique (provider, provider_id)
);
create index social_accounts_user_id on social_accounts (user_id);
