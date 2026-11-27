create table throttle (
    id text primary key,
    state bytea not null,
    expires_at bigint not null
);
create index throttle_expires_at on throttle (expires_at);
