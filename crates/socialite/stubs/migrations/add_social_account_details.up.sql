-- What the provider last reported, and when the account was last used to sign in (unix seconds).
alter table social_accounts
    add column email text,
    add column username text,
    add column avatar_url text,
    add column last_used_at bigint;
