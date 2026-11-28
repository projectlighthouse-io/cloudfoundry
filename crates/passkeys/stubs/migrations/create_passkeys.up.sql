-- Passkeys (`passkeys`). Times are unix seconds. Deleting a user needs these
-- rows gone first (`Passkeys::delete_all`): no cascades.
create table passkeys (
    id bigserial primary key,
    -- Names the passkey in URLs and JSON (`db::Uuid<Passkey>`).
    public_id uuid not null unique,
    user_id bigint not null references users (id),
    name text not null,
    -- The WebAuthn credential id; one account per credential.
    credential_id bytea not null unique,
    -- The user's WebAuthn user handle (random, the same on all their passkeys).
    user_handle uuid not null,
    -- webauthn-rs `Passkey`: public key, algorithm, counter, backup flags.
    credential jsonb not null,
    last_used_at bigint,
    created_at bigint not null
);

create index passkeys_user on passkeys (user_id);
