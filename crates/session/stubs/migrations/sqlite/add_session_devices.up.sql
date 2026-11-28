-- Browsers & devices: who a session belongs to and where it was last seen.
-- Times are unix seconds; `public_id` is 16 bytes. No foreign key: sessions come before users.
alter table sessions add column user_id integer;
alter table sessions add column public_id blob;
alter table sessions add column user_agent text;
alter table sessions add column ip text;
alter table sessions add column last_seen_at integer;

create index sessions_user_id on sessions (user_id);
