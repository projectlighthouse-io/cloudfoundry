create table jobs (
    id bigint auto_increment primary key,
    queue varchar(191) not null,
    name varchar(191) not null,
    payload longtext not null,
    attempts int not null default 0,
    max_attempts int not null,
    available_at bigint not null,
    reserved_until bigint,
    last_error text,
    created_at bigint not null,
    index jobs_claim (queue, available_at)
);

create table failed_jobs (
    id bigint primary key,
    queue varchar(191) not null,
    name varchar(191) not null,
    payload longtext not null,
    attempts int not null,
    max_attempts int not null,
    error text not null,
    failed_at bigint not null
);
