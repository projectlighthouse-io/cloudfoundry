create table organizations (
    id bigint primary key auto_increment,
    name varchar(255) not null
);

create table organization_members (
    organization_id bigint not null,
    user_id bigint not null,
    role varchar(64) not null,
    primary key (organization_id, user_id),
    index organization_members_user (user_id),
    foreign key (organization_id) references organizations (id) on delete cascade
);

-- A team with no organization stands alone. Names are unique within an
-- organization; nulls are distinct, so standalone teams may share names.
create table teams (
    id bigint primary key auto_increment,
    organization_id bigint null,
    name varchar(255) not null,
    unique (organization_id, name),
    foreign key (organization_id) references organizations (id) on delete cascade
);

create table team_members (
    team_id bigint not null,
    user_id bigint not null,
    role varchar(64) not null,
    primary key (team_id, user_id),
    index team_members_user (user_id),
    foreign key (team_id) references teams (id) on delete cascade
);

create table team_invitations (
    id bigint primary key auto_increment,
    scope varchar(8) not null,
    scope_id bigint not null,
    email varchar(255) not null,
    role varchar(64) not null,
    token_hash char(64) not null unique,
    invited_by bigint not null,
    expires_at bigint not null,
    index team_invitations_scope (scope, scope_id)
);
