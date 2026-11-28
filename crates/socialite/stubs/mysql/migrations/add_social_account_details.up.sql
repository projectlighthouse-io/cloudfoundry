-- What the provider last reported, and when the account was last used to sign in (unix seconds).
alter table social_accounts
    add column email varchar(255),
    add column username varchar(255),
    add column avatar_url varchar(1024),
    add column last_used_at bigint;
