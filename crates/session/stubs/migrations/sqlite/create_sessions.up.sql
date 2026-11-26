create table sessions (
    id text primary key,
    data blob not null,
    expires_at integer not null
);

create index sessions_expires_at on sessions (expires_at);
