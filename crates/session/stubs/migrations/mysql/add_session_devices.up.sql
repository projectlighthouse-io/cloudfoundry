-- Browsers & devices: who a session belongs to and where it was last seen.
-- Times are unix seconds; `public_id` is 16 bytes. No foreign key: sessions come before users.
alter table sessions
    add column user_id bigint,
    add column public_id binary(16),
    add column user_agent varchar(512),
    add column ip varchar(45),
    add column last_seen_at bigint,
    add index sessions_user_id (user_id);
