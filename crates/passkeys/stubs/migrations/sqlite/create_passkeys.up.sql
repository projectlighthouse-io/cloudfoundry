-- Passkeys (`passkeys`). Times are unix seconds. Deleting a user needs these
-- rows gone first (`Passkeys::delete_all`): no cascades.
create table passkeys (
    id integer primary key autoincrement,
    -- Names the passkey in URLs and JSON (`db::Uuid<Passkey>`), 16 bytes.
    public_id blob not null unique,
    user_id integer not null references users (id),
    name text not null,
    -- The WebAuthn credential id; one account per credential.
    credential_id blob not null unique,
    -- The user's WebAuthn user handle (random, the same on all their passkeys), 16 bytes.
    user_handle blob not null,
    -- webauthn-rs `Passkey` as JSON: public key, algorithm, counter, backup flags.
    credential text not null,
    last_used_at integer,
    created_at integer not null
);

create index passkeys_user on passkeys (user_id);
