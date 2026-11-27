-- Accounts made through social login have no password until they set one.
alter table users alter column password_hash drop not null;
