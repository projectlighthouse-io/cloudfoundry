-- One row per provider account linked to a user.
create table social_accounts (
    id bigserial primary key,
    user_id bigint not null references users (id),
    provider text not null,
    provider_id text not null,
    created_at timestamptz not null default now(),
    unique (provider, provider_id)
);
create index social_accounts_user_id on social_accounts (user_id);
