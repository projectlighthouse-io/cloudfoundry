-- Passkeys (`passkeys`). Times are unix seconds. Deleting a user needs these
-- rows gone first (`Passkeys::delete_all`): no cascades.
create table passkeys (
    id bigint primary key auto_increment,
    -- Names the passkey in URLs and JSON (`db::Uuid<Passkey>`), 16 bytes.
    public_id binary(16) not null unique,
    user_id bigint not null,
    name varchar(255) not null,
    -- The WebAuthn credential id (at most 1023 bytes); one account per credential.
    credential_id varbinary(1023) not null unique,
    -- The user's WebAuthn user handle (random, the same on all their passkeys).
    user_handle binary(16) not null,
    -- webauthn-rs `Passkey`: public key, algorithm, counter, backup flags.
    credential json not null,
    last_used_at bigint,
    created_at bigint not null,
    index passkeys_user (user_id),
    foreign key (user_id) references users (id)
);
