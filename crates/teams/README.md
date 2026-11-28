# teams

Organizations and teams. A team belongs to one organization or stands alone.
Roles are your enums and each permission is your type, so a handler states
the permission it needs in its signature and does not compile without a rule
for it.

```toml
[dependencies]
teams = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["sqlite", "auth", "session"] }
```

```rust
// src/access.rs (published by `art publish teams`)
pub struct InviteMembers;
impl teams::Permission for InviteMembers {}

impl teams::Grants<InviteMembers> for TeamRole {
    fn grants(&self) -> bool { matches!(self, Self::Owner | Self::Admin) }
}

// A handler: 404 unless the user can open `{team}`, 403 unless their role grants the permission.
async fn invite(cx: Cx<App>, (can, input): (Can<InviteMembers, TeamRole>, Valid<Json<Invite>>)) -> ... {
    let target = Target::Team { team: can.team.id, role: input.role };
    let new = access::teams(&cx.db).invite(target, &input.email, can.user_id, Duration::from_secs(7 * 86400)).await?;
    // mail a link with new.token
}
```

Routes name their team by id: `/teams/{team}/...` and `/organizations/{org}/...`.
`teams::layer(teams, teams::auth_user(|u: &User| u.id))` goes after `auth::layer`.

## Rules

`Teams` enforces these; the stores only keep data.

- A team or organization keeps at least one owner: the last one cannot leave or be demoted.
- An organization's team takes only members of the organization. Leaving the organization leaves its teams.
- Organization roles whose `manages_teams()` is true act as owners of every team in it, without joining.
- Team names are unique within an organization. Standalone teams may share names.
- Invitations are single use, expire, and must be accepted by the invited email. Only the token's hash is stored,
  so a link can be shown once: right after `invite` or `resend_team_invitation` / `resend_org_invitation`, which
  issue a new token and lifetime (the old link stops working).
- One pending invitation per email and team or organization (ignoring case): `Error::AlreadyInvited`. An expired one
  is replaced. Whether the email already belongs to a member is the app's check (the store keeps user ids); the
  published `access::not_a_member` does it.

## Required teams

`Teams::new(store).require_team(|user_id| "Personal".to_owned())` (on in the published `src/access.rs`):

- A user with no team gets a personal team, as owner, the first time `CurrentTeam` is resolved.
- Leaving or being removed from your last team is `Error::LastTeam`. Before deleting a team or an organization,
  call `keeps_a_team(user, |team| ...)` with the teams that go away.

## Current team

`CurrentTeam<R>` is stored in `users.current_team_id` (the teams migration adds the column; publish auth first) and
cached in the session. It resolves, in order: the session's id, the stored id, then the first team by id (or a new
personal team in required mode), and stores what it falls back to. Each id is checked, so a team the user left is
skipped. A new login, another device or another app instance lands on the stored team; a session keeps its cached
choice until it ends. Switch with `current.set(&membership).await`.

## Inputs

| Input | Gives | Fails with |
| --- | --- | --- |
| `Membership<R>` | `team`, `role`, `via_org`, `user_id` for `{team}` | `404` |
| `Can<P, R>` | a `Membership<R>` whose role grants `P` | `404`, `403` |
| `OrgMembership<O>` / `OrgCan<P, O>` | the same for `{org}` | `404`, `403` |
| `CurrentTeam<R>` (feature `session`) | the current team; see [Current team](#current-team) | `500` if the store fails |

No logged-in user is whatever `auth` answers (redirect to login, or `401`).

## Features

| Feature | Adds |
| --- | --- |
| `sqlite`, `postgres`, `mysql` | `SqliteStore`, `PostgresStore`, `MysqlStore`; tables via `art publish teams [--driver ...]` |
| `auth` | `teams::auth_user`: the user id from `auth`'s logged-in user |
| `session` | `CurrentTeam` |

`MemoryStore` is always there, for tests. `art publish teams [--with-ui]` copies
roles, permissions, handlers, requests and (with UI) Inertia pages into the app.
