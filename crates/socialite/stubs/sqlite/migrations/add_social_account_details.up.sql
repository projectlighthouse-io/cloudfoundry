-- What the provider last reported, and when the account was last used to sign in (unix seconds).
alter table social_accounts add column email text;
alter table social_accounts add column username text;
alter table social_accounts add column avatar_url text;
alter table social_accounts add column last_used_at integer;
