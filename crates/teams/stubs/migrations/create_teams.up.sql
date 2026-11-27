create table organizations (
    id bigserial primary key,
    name text not null
);

create table organization_members (
    organization_id bigint not null references organizations (id) on delete cascade,
    user_id bigint not null,
    role text not null,
    primary key (organization_id, user_id)
);

create index organization_members_user on organization_members (user_id);

-- A team with no organization stands alone. Names are unique within an
-- organization; nulls are distinct, so standalone teams may share names.
create table teams (
    id bigserial primary key,
    organization_id bigint references organizations (id) on delete cascade,
    name text not null,
    unique (organization_id, name)
);

create table team_members (
    team_id bigint not null references teams (id) on delete cascade,
    user_id bigint not null,
    role text not null,
    primary key (team_id, user_id)
);

create index team_members_user on team_members (user_id);

create table team_invitations (
    id bigserial primary key,
    scope text not null,
    scope_id bigint not null,
    email text not null,
    role text not null,
    token_hash text not null unique,
    invited_by bigint not null,
    expires_at bigint not null
);

create index team_invitations_scope on team_invitations (scope, scope_id);
