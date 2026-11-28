-- Browsers & devices: who a session belongs to and where it was last seen.
-- Times are unix seconds; `ip` is text. No foreign key: sessions come before users.
alter table sessions
    add column user_id bigint,
    -- Names the session in URLs and JSON; the session id never leaves the cookie.
    add column public_id uuid,
    add column user_agent text,
    add column ip text,
    add column last_seen_at bigint;

create index sessions_user_id on sessions (user_id);
