create table sessions (
    id varchar(64) primary key,
    data mediumblob not null,
    expires_at bigint not null,
    index sessions_expires_at (expires_at)
);
