-- Two-factor authentication (`factor2`). Times are unix seconds. Deleting a
-- user needs these rows gone first (`TwoFactor::disable`): no cascades.
create table totp_secrets (
    user_id bigint primary key references users (id),
    -- Sealed with `crypt` (APP_KEY), bound to the user id; never the plain secret.
    secret_encrypted bytea not null,
    -- Null until the first valid code: until then logins do not ask for one.
    confirmed_at bigint,
    -- The last accepted time step; a code for it or an older one is a replay.
    last_used_step bigint,
    created_at bigint not null
);

create table backup_codes (
    id bigserial primary key,
    user_id bigint not null references users (id),
    -- SHA-256 of the code; the codes themselves are shown once.
    code_hash bytea not null,
    used_at bigint
);

create index backup_codes_user on backup_codes (user_id);

create table trusted_devices (
    id bigserial primary key,
    -- Names the device in URLs and JSON (`db::Uuid<TrustedDevice>`).
    uuid uuid not null unique,
    user_id bigint not null references users (id),
    -- SHA-256 of the token in the signed `two_factor_trusted` cookie.
    token_hash bytea not null unique,
    user_agent text,
    ip text,
    expires_at bigint not null,
    last_used_at bigint,
    created_at bigint not null
);

create index trusted_devices_user on trusted_devices (user_id);
