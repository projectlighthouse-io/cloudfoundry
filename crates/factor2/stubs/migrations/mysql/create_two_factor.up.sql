-- Two-factor authentication (`factor2`). Times are unix seconds. Deleting a
-- user needs these rows gone first (`TwoFactor::disable`): no cascades.
create table totp_secrets (
    user_id bigint primary key,
    -- Sealed with `crypt` (APP_KEY), bound to the user id; never the plain secret.
    secret_encrypted varbinary(255) not null,
    -- Null until the first valid code: until then logins do not ask for one.
    confirmed_at bigint,
    -- The last accepted time step; a code for it or an older one is a replay.
    last_used_step bigint,
    created_at bigint not null,
    foreign key (user_id) references users (id)
);

create table backup_codes (
    id bigint primary key auto_increment,
    user_id bigint not null,
    -- SHA-256 of the code; the codes themselves are shown once.
    code_hash binary(32) not null,
    used_at bigint,
    index backup_codes_user (user_id),
    foreign key (user_id) references users (id)
);

create table trusted_devices (
    id bigint primary key auto_increment,
    -- Names the device in URLs and JSON (`db::Uuid<TrustedDevice>`), 16 bytes.
    uuid binary(16) not null unique,
    user_id bigint not null,
    -- SHA-256 of the token in the signed `two_factor_trusted` cookie.
    token_hash binary(32) not null unique,
    user_agent varchar(512),
    ip varchar(45),
    expires_at bigint not null,
    last_used_at bigint,
    created_at bigint not null,
    index trusted_devices_user (user_id),
    foreign key (user_id) references users (id)
);
