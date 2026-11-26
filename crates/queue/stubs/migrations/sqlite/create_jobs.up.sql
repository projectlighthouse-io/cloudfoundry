create table jobs (
    id integer primary key autoincrement,
    queue text not null,
    name text not null,
    payload text not null,
    attempts integer not null default 0,
    max_attempts integer not null,
    available_at integer not null,
    reserved_until integer,
    last_error text,
    created_at integer not null
);

create index jobs_claim on jobs (queue, available_at);

create table failed_jobs (
    id integer primary key,
    queue text not null,
    name text not null,
    payload text not null,
    attempts integer not null,
    max_attempts integer not null,
    error text not null,
    failed_at integer not null
);
