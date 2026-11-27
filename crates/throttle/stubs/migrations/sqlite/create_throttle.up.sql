create table throttle (
    id text primary key,
    state blob not null,
    expires_at integer not null
);
create index throttle_expires_at on throttle (expires_at);
