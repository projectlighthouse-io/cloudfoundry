create table throttle (
    id varchar(191) primary key,
    state varbinary(255) not null,
    expires_at bigint not null,
    index throttle_expires_at (expires_at)
);
