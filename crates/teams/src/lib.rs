//! Organizations and teams. A team belongs to one organization or stands
//! alone. Roles are your enums and each permission is your type, so a
//! handler states the permission it needs in its signature:
//!
//! ```ignore
//! pub struct InviteMember;
//! impl teams::Permission for InviteMember {}
//!
//! impl teams::Grants<InviteMember> for TeamRole {
//!     fn grants(&self) -> bool { matches!(self, Self::Owner | Self::Admin) }
//! }
//!
//! // Runs only for a member of `{team}` whose role grants `InviteMember`.
//! async fn invite(can: Can<InviteMember, TeamRole>, input: Valid<Json<Invite>>) -> ... {
//!     can.team; can.role;
//! }
//! ```
//!
//! Routes name their team by uuid (`/teams/{team}/...`, `/orgs/{org}/...`,
//! a [`Uuid`]): internal `i64` ids never appear in a URL or in JSON.
//! [`layer`] goes after `auth::layer`; it gives the inputs ([`Membership`],
//! [`Can`], [`OrgMembership`], [`OrgCan`]) the store and the current user.
//!
//! The rules live in [`Teams`]: a team keeps at least one owner, an
//! organization's teams only take its members, and organization roles that
//! [`OrgRole::manages_teams`] act as owners of every team in it.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, Serializer, ser::SerializeStruct};
use sha2::{Digest, Sha256};

#[cfg(feature = "session")]
mod current;
mod inputs;
mod memory;
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mysql"))]
mod sql;

#[cfg(feature = "session")]
pub use current::{CurrentTeam, RequireCurrentTeam, RequireCurrentTeamService, intended, require_current_team};
#[cfg(feature = "auth")]
pub use inputs::{AuthUser, auth_user};
pub use inputs::{Can, Identify, Membership, OrgCan, OrgMembership, TeamsLayer, TeamsService, layer};
pub use db::Uuid;
pub use memory::MemoryStore;
#[cfg(feature = "mysql")]
pub use sql::MysqlStore;
#[cfg(feature = "postgres")]
pub use sql::PostgresStore;
#[cfg(feature = "sqlite")]
pub use sql::SqliteStore;

/// A role on a team or an organization: your enum.
pub trait Role: Copy + Eq + Send + Sync + 'static {
    /// The role a creator gets, and the one that cannot run out: a team or
    /// organization always keeps at least one.
    const OWNER: Self;
    /// The value stored in the database.
    fn key(&self) -> &'static str;
    /// The role for a stored key. A key that maps to nothing grants no access.
    fn from_key(key: &str) -> Option<Self>;
}

/// A role on an organization.
pub trait OrgRole: Role {
    /// Whether this role manages every team in the organization, as an owner,
    /// without being a member of it.
    fn manages_teams(&self) -> bool;
}

/// A permission: your type, usually a unit struct. Which roles have it is
/// decided by [`Grants`].
pub trait Permission: Send + Sync + 'static {}

/// Whether a role has permission `P`. A handler taking `Can<P, R>` does not
/// compile without this impl.
pub trait Grants<P: Permission>: Role {
    fn grants(&self) -> bool;
}

/// Serializes as `{ "uuid", "name" }`; `id` is for app queries only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Org {
    #[serde(skip)]
    pub id: i64,
    pub uuid: Uuid<Org>,
    pub name: String,
}

/// Serializes as `{ "uuid", "name" }`; the internal ids are for app queries only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Team {
    #[serde(skip)]
    pub id: i64,
    pub uuid: Uuid<Team>,
    /// `None` for a team that stands alone.
    #[serde(skip)]
    pub org_id: Option<i64>,
    pub name: String,
}

/// A member and their role. Not `Serialize`: `user_id` is internal. Map
/// members to your users (by `uuid`) for a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member<R> {
    pub user_id: i64,
    pub role: R,
}

/// What an invitation joins, and with which role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<R, O> {
    Team { team: i64, role: R },
    Org { org: i64, role: O },
}

/// The role parameters default to `()` so the id type reads `Uuid<Invitation>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation<R = (), O = ()> {
    pub id: i64,
    pub uuid: Uuid<Invitation>,
    pub email: String,
    pub target: Target<R, O>,
    pub invited_by: i64,
    /// Unix seconds.
    pub expires_at: i64,
}

impl<R: Role, O: Role> Serialize for Invitation<R, O> {
    fn serialize<Se: Serializer>(&self, s: Se) -> Result<Se::Ok, Se::Error> {
        // `invited_by` is a user's internal id: left out.
        let mut st = s.serialize_struct("Invitation", 4)?;
        st.serialize_field("uuid", &self.uuid)?;
        st.serialize_field("email", &self.email)?;
        match self.target {
            Target::Team { role, .. } => st.serialize_field("role", role.key())?,
            Target::Org { role, .. } => st.serialize_field("role", role.key())?,
        }
        st.serialize_field("expires_at", &self.expires_at)?;
        st.end()
    }
}

#[derive(Debug)]
pub enum Error {
    /// Another team in the same organization has this name.
    NameTaken,
    /// The change would leave the team or organization without an owner.
    LastOwner,
    /// An organization's team only takes members of the organization.
    NotOrgMember,
    /// The user is not a member.
    NotMember,
    /// The team, organization or invitation does not exist.
    NotFound,
    /// The invitation's time ran out; it has been deleted.
    InvitationExpired,
    /// The invitation was sent to another email address.
    EmailMismatch,
    /// This email already has an invitation that has not expired.
    /// Send that one again instead.
    AlreadyInvited,
    /// The store failed.
    Store(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameTaken => f.write_str("a team in this organization already has that name"),
            Self::LastOwner => f.write_str("the last owner cannot leave or be demoted"),
            Self::NotOrgMember => f.write_str("the user is not a member of the organization"),
            Self::NotMember => f.write_str("the user is not a member"),
            Self::NotFound => f.write_str("not found"),
            Self::InvitationExpired => f.write_str("the invitation has expired"),
            Self::EmailMismatch => f.write_str("the invitation was sent to another email address"),
            Self::AlreadyInvited => f.write_str("this email already has a pending invitation"),
            Self::Store(e) => write!(f, "teams store: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Persistence for organizations, teams and invitations. The crate ships
/// [`MemoryStore`] and, behind features, SQLite, Postgres and MySQL stores.
/// Stores keep data; [`Teams`] applies the rules.
pub trait Store: Send + Sync + 'static {
    type Role: Role;
    type OrgRole: OrgRole;

    /// Create an organization with `owner` as `OrgRole::OWNER`.
    fn create_org(&self, owner: i64, name: &str) -> impl Future<Output = Result<Org, Error>> + Send;
    fn org(&self, id: i64) -> impl Future<Output = Result<Option<Org>, Error>> + Send;
    fn org_by_uuid(&self, id: Uuid<Org>) -> impl Future<Output = Result<Option<Org>, Error>> + Send;
    fn rename_org(&self, id: i64, name: &str) -> impl Future<Output = Result<(), Error>> + Send;
    /// Deletes its teams, memberships and invitations too.
    fn delete_org(&self, id: i64) -> impl Future<Output = Result<(), Error>> + Send;
    fn org_role(&self, org: i64, user: i64) -> impl Future<Output = Result<Option<Self::OrgRole>, Error>> + Send;
    /// Add a member or change their role.
    fn set_org_role(&self, org: i64, user: i64, role: Self::OrgRole) -> impl Future<Output = Result<(), Error>> + Send;
    /// Remove a member from the organization and from all of its teams.
    fn remove_org_member(&self, org: i64, user: i64) -> impl Future<Output = Result<(), Error>> + Send;
    fn org_members(&self, org: i64) -> impl Future<Output = Result<Vec<Member<Self::OrgRole>>, Error>> + Send;
    fn orgs_of(&self, user: i64) -> impl Future<Output = Result<Vec<(Org, Self::OrgRole)>, Error>> + Send;

    /// Create a team with `owner` as `Role::OWNER`. [`Error::NameTaken`] if
    /// the organization already has a team with this name.
    fn create_team(&self, owner: i64, org: Option<i64>, name: &str) -> impl Future<Output = Result<Team, Error>> + Send;
    fn team(&self, id: i64) -> impl Future<Output = Result<Option<Team>, Error>> + Send;
    fn team_by_uuid(&self, id: Uuid<Team>) -> impl Future<Output = Result<Option<Team>, Error>> + Send;
    fn rename_team(&self, id: i64, name: &str) -> impl Future<Output = Result<(), Error>> + Send;
    /// Deletes its memberships and invitations too.
    fn delete_team(&self, id: i64) -> impl Future<Output = Result<(), Error>> + Send;
    fn team_role(&self, team: i64, user: i64) -> impl Future<Output = Result<Option<Self::Role>, Error>> + Send;
    fn set_team_role(&self, team: i64, user: i64, role: Self::Role) -> impl Future<Output = Result<(), Error>> + Send;
    fn remove_team_member(&self, team: i64, user: i64) -> impl Future<Output = Result<(), Error>> + Send;
    fn team_members(&self, team: i64) -> impl Future<Output = Result<Vec<Member<Self::Role>>, Error>> + Send;
    /// Teams the user is a member of, with their role, by id.
    fn teams_of(&self, user: i64) -> impl Future<Output = Result<Vec<(Team, Self::Role)>, Error>> + Send;
    fn org_teams(&self, org: i64) -> impl Future<Output = Result<Vec<Team>, Error>> + Send;

    fn create_invitation(
        &self,
        email: &str,
        target: Target<Self::Role, Self::OrgRole>,
        invited_by: i64,
        expires_at: i64,
        token_hash: &str,
    ) -> impl Future<Output = Result<InvitationOf<Self>, Error>> + Send;
    fn invitation(&self, token_hash: &str) -> impl Future<Output = Result<Option<InvitationOf<Self>>, Error>> + Send;
    fn team_invitations(&self, team: i64) -> impl Future<Output = Result<Vec<InvitationOf<Self>>, Error>> + Send;
    fn org_invitations(&self, org: i64) -> impl Future<Output = Result<Vec<InvitationOf<Self>>, Error>> + Send;
    fn delete_invitation(&self, id: i64) -> impl Future<Output = Result<(), Error>> + Send;

    /// The team the user last switched to: `users.current_team_id` in the SQL
    /// stores. `None` when unset. May name a team they left or that was deleted;
    /// [`crate::CurrentTeam`] checks access and falls back.
    fn current_team_id(&self, user: i64) -> impl Future<Output = Result<Option<i64>, Error>> + Send;
    fn set_current_team_id(&self, user: i64, team: Option<i64>) -> impl Future<Output = Result<(), Error>> + Send;
}

/// An invitation with a store's role types.
pub type InvitationOf<S> = Invitation<<S as Store>::Role, <S as Store>::OrgRole>;

/// A new invitation and its token. The token is shown once: put it in the
/// accept link you send; only its hash is stored.
pub struct NewInvitation<R, O> {
    pub invitation: Invitation<R, O>,
    pub token: String,
}

/// The rules over a [`Store`]. Cheap to clone.
pub struct Teams<S>(Arc<S>);

impl<S> Clone for Teams<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

type R<S> = <S as Store>::Role;
type O<S> = <S as Store>::OrgRole;

impl<S: Store> Teams<S> {
    pub fn new(store: S) -> Self {
        Self(Arc::new(store))
    }

    /// The store, for reads such as `team_members` and `org_invitations`.
    pub fn store(&self) -> &S {
        &self.0
    }

    pub async fn create_org(&self, owner: i64, name: &str) -> Result<Org, Error> {
        self.0.create_org(owner, name).await
    }

    /// A team owned by `owner`. In an organization, `owner` must be a member of it.
    pub async fn create_team(&self, owner: i64, org: Option<i64>, name: &str) -> Result<Team, Error> {
        if let Some(org) = org {
            self.0.org(org).await?.ok_or(Error::NotFound)?;
            self.0.org_role(org, owner).await?.ok_or(Error::NotOrgMember)?;
        }
        self.0.create_team(owner, org, name).await
    }

    pub async fn rename_team(&self, team: &Team, name: &str) -> Result<(), Error> {
        self.0.rename_team(team.id, name).await
    }

    pub async fn delete_team(&self, team: &Team) -> Result<(), Error> {
        self.0.delete_team(team.id).await
    }

    pub async fn rename_org(&self, org: &Org, name: &str) -> Result<(), Error> {
        self.0.rename_org(org.id, name).await
    }

    pub async fn delete_org(&self, org: &Org) -> Result<(), Error> {
        self.0.delete_org(org.id).await
    }

    /// Add `user` to `team`, or change their role. An organization's team
    /// only takes members of the organization.
    pub async fn add_member(&self, team: &Team, user: i64, role: R<S>) -> Result<(), Error> {
        if let Some(org) = team.org_id {
            self.0.org_role(org, user).await?.ok_or(Error::NotOrgMember)?;
        }
        if self.0.team_role(team.id, user).await?.is_some() {
            return self.change_role(team, user, role).await;
        }
        self.0.set_team_role(team.id, user, role).await
    }

    /// Change a member's role. The last owner cannot be demoted.
    pub async fn change_role(&self, team: &Team, user: i64, role: R<S>) -> Result<(), Error> {
        let current = self.0.team_role(team.id, user).await?.ok_or(Error::NotMember)?;
        if current == R::<S>::OWNER && role != R::<S>::OWNER && self.team_owners(team.id).await? == 1 {
            return Err(Error::LastOwner);
        }
        self.0.set_team_role(team.id, user, role).await
    }

    /// Remove a member, or let them leave. The last owner cannot.
    pub async fn remove_member(&self, team: &Team, user: i64) -> Result<(), Error> {
        let current = self.0.team_role(team.id, user).await?.ok_or(Error::NotMember)?;
        if current == R::<S>::OWNER && self.team_owners(team.id).await? == 1 {
            return Err(Error::LastOwner);
        }
        self.0.remove_team_member(team.id, user).await
    }

    /// Add `user` to `org`, or change their role.
    pub async fn add_org_member(&self, org: &Org, user: i64, role: O<S>) -> Result<(), Error> {
        if self.0.org_role(org.id, user).await?.is_some() {
            return self.change_org_role(org, user, role).await;
        }
        self.0.set_org_role(org.id, user, role).await
    }

    /// Change an organization member's role. The last owner cannot be demoted.
    pub async fn change_org_role(&self, org: &Org, user: i64, role: O<S>) -> Result<(), Error> {
        let current = self.0.org_role(org.id, user).await?.ok_or(Error::NotMember)?;
        if current == O::<S>::OWNER && role != O::<S>::OWNER && self.org_owners(org.id).await? == 1 {
            return Err(Error::LastOwner);
        }
        self.0.set_org_role(org.id, user, role).await
    }

    /// Remove a member from the organization and its teams, or let them leave.
    /// The last owner cannot.
    pub async fn remove_org_member(&self, org: &Org, user: i64) -> Result<(), Error> {
        let current = self.0.org_role(org.id, user).await?.ok_or(Error::NotMember)?;
        if current == O::<S>::OWNER && self.org_owners(org.id).await? == 1 {
            return Err(Error::LastOwner);
        }
        self.0.remove_org_member(org.id, user).await
    }

    /// What `user` may do in `team`: their team role, or `OWNER` (with
    /// `via_org`) when an organization role manages the team's organization
    /// and the team role is lower. `None` without access.
    pub async fn team_access(&self, team: i64, user: i64) -> Result<Option<Membership<R<S>>>, Error> {
        let Some(found) = self.0.team(team).await? else { return Ok(None) };
        self.access_to(found, user).await
    }

    /// [`Teams::team_access`] for the team with this uuid.
    pub async fn team_access_by_uuid(&self, team: Uuid<Team>, user: i64) -> Result<Option<Membership<R<S>>>, Error> {
        let Some(found) = self.0.team_by_uuid(team).await? else { return Ok(None) };
        self.access_to(found, user).await
    }

    async fn access_to(&self, found: Team, user: i64) -> Result<Option<Membership<R<S>>>, Error> {
        let role = self.0.team_role(found.id, user).await?;
        if role != Some(R::<S>::OWNER) {
            if let Some(org) = found.org_id {
                if self.0.org_role(org, user).await?.is_some_and(|r| r.manages_teams()) {
                    return Ok(Some(Membership { team: found, role: R::<S>::OWNER, via_org: true, user_id: user }));
                }
            }
        }
        Ok(role.map(|role| Membership { team: found, role, via_org: false, user_id: user }))
    }

    /// `user`'s role in `org`; `None` if they are not a member.
    pub async fn org_access(&self, org: i64, user: i64) -> Result<Option<OrgMembership<O<S>>>, Error> {
        let Some(found) = self.0.org(org).await? else { return Ok(None) };
        Ok(self.0.org_role(org, user).await?.map(|role| OrgMembership { org: found, role, user_id: user }))
    }

    /// [`Teams::org_access`] for the organization with this uuid.
    pub async fn org_access_by_uuid(&self, org: Uuid<Org>, user: i64) -> Result<Option<OrgMembership<O<S>>>, Error> {
        let Some(found) = self.0.org_by_uuid(org).await? else { return Ok(None) };
        Ok(self.0.org_role(found.id, user).await?.map(|role| OrgMembership { org: found, role, user_id: user }))
    }

    /// Every team `user` can open: their own, and all teams of organizations
    /// they manage. By team id.
    pub async fn teams_of(&self, user: i64) -> Result<Vec<Membership<R<S>>>, Error> {
        let mut out: Vec<Membership<R<S>>> =
            self.0.teams_of(user).await?.into_iter().map(|(team, role)| Membership { team, role, via_org: false, user_id: user }).collect();
        for (org, role) in self.0.orgs_of(user).await? {
            if !role.manages_teams() {
                continue;
            }
            for team in self.0.org_teams(org.id).await? {
                if out.iter().any(|m| m.team.id == team.id && m.role == R::<S>::OWNER) {
                    continue;
                }
                out.retain(|m| m.team.id != team.id);
                out.push(Membership { team, role: R::<S>::OWNER, via_org: true, user_id: user });
            }
        }
        out.sort_by_key(|m| m.team.id);
        Ok(out)
    }

    /// Invite `email` to a team or organization for `valid_for`. Returns the
    /// token for the accept link; it is not stored. [`Error::AlreadyInvited`]
    /// if `email` (ignoring case) has an unexpired invitation to the same
    /// team or organization; an expired one is deleted. Whether `email`
    /// already belongs to a member is the app's to check: the store keeps
    /// user ids, not emails.
    pub async fn invite(&self, target: Target<R<S>, O<S>>, email: &str, invited_by: i64, valid_for: Duration) -> Result<NewInvitation<R<S>, O<S>>, Error> {
        let pending = match target {
            Target::Team { team, .. } => {
                self.0.team(team).await?.ok_or(Error::NotFound)?;
                self.0.team_invitations(team).await?
            }
            Target::Org { org, .. } => {
                self.0.org(org).await?.ok_or(Error::NotFound)?;
                self.0.org_invitations(org).await?
            }
        };
        let now = now();
        for old in pending.iter().filter(|i| i.email.eq_ignore_ascii_case(email)) {
            if old.expires_at > now {
                return Err(Error::AlreadyInvited);
            }
            self.0.delete_invitation(old.id).await?;
        }
        self.issue(target, email, invited_by, valid_for).await
    }

    /// Send one of `team`'s invitations again: a new token and a new
    /// `valid_for`; the old link stops working. [`Error::NotFound`] if it is
    /// not one of them.
    pub async fn resend_team_invitation(&self, team: &Team, invitation: Uuid<Invitation>, valid_for: Duration) -> Result<NewInvitation<R<S>, O<S>>, Error> {
        let old = self.0.team_invitations(team.id).await?.into_iter().find(|i| i.uuid == invitation).ok_or(Error::NotFound)?;
        self.reissue(old, valid_for).await
    }

    /// Send one of `org`'s invitations again; see [`Teams::resend_team_invitation`].
    pub async fn resend_org_invitation(&self, org: &Org, invitation: Uuid<Invitation>, valid_for: Duration) -> Result<NewInvitation<R<S>, O<S>>, Error> {
        let old = self.0.org_invitations(org.id).await?.into_iter().find(|i| i.uuid == invitation).ok_or(Error::NotFound)?;
        self.reissue(old, valid_for).await
    }

    // ponytail: delete + create instead of a store method to rotate the token; the id changes.
    async fn reissue(&self, old: InvitationOf<S>, valid_for: Duration) -> Result<NewInvitation<R<S>, O<S>>, Error> {
        self.0.delete_invitation(old.id).await?;
        self.issue(old.target, &old.email, old.invited_by, valid_for).await
    }

    async fn issue(&self, target: Target<R<S>, O<S>>, email: &str, invited_by: i64, valid_for: Duration) -> Result<NewInvitation<R<S>, O<S>>, Error> {
        let token = random_token();
        let expires_at = now().saturating_add(valid_for.as_secs() as i64);
        let invitation = self.0.create_invitation(email, target, invited_by, expires_at, &hash_token(&token)).await?;
        Ok(NewInvitation { invitation, token })
    }

    /// Accept an invitation as `user`, whose email must be the invited one
    /// (compared ignoring case). Joins with the invited role, or keeps a role
    /// the user already has. An expired invitation is deleted.
    pub async fn accept(&self, token: &str, user: i64, email: &str) -> Result<Target<R<S>, O<S>>, Error> {
        let invitation = self.0.invitation(&hash_token(token)).await?.ok_or(Error::NotFound)?;
        if invitation.expires_at <= now() {
            self.0.delete_invitation(invitation.id).await?;
            return Err(Error::InvitationExpired);
        }
        if !invitation.email.eq_ignore_ascii_case(email) {
            return Err(Error::EmailMismatch);
        }
        match invitation.target {
            Target::Team { team, role } => {
                let team = self.0.team(team).await?.ok_or(Error::NotFound)?;
                if self.0.team_role(team.id, user).await?.is_none() {
                    self.add_member(&team, user, role).await?;
                }
            }
            Target::Org { org, role } => {
                if self.0.org_role(org, user).await?.is_none() {
                    self.0.set_org_role(org, user, role).await?;
                }
            }
        }
        self.0.delete_invitation(invitation.id).await?;
        Ok(invitation.target)
    }

    /// Withdraw one of `team`'s invitations. [`Error::NotFound`] if it is not one of them.
    pub async fn revoke_team_invitation(&self, team: &Team, invitation: Uuid<Invitation>) -> Result<(), Error> {
        let found = self.0.team_invitations(team.id).await?.into_iter().find(|i| i.uuid == invitation).ok_or(Error::NotFound)?;
        self.0.delete_invitation(found.id).await
    }

    /// Withdraw one of `org`'s invitations. [`Error::NotFound`] if it is not one of them.
    pub async fn revoke_org_invitation(&self, org: &Org, invitation: Uuid<Invitation>) -> Result<(), Error> {
        let found = self.0.org_invitations(org.id).await?.into_iter().find(|i| i.uuid == invitation).ok_or(Error::NotFound)?;
        self.0.delete_invitation(found.id).await
    }

    async fn team_owners(&self, team: i64) -> Result<usize, Error> {
        Ok(self.0.team_members(team).await?.iter().filter(|m| m.role == R::<S>::OWNER).count())
    }

    async fn org_owners(&self, org: i64) -> Result<usize, Error> {
        Ok(self.0.org_members(org).await?.iter().filter(|m| m.role == O::<S>::OWNER).count())
    }
}

fn now() -> i64 {
    // A clock set before 1970 reads as 0 rather than panicking.
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    // No secure fallback exists: a guessable invitation token must not be issued.
    #[allow(clippy::expect_used, reason = "the OS random source failed; there is no safe fallback")]
    getrandom::fill(&mut b).expect("OS random source");
    URL_SAFE_NO_PAD.encode(b)
}

/// The stored form of an invitation token: SHA-256, hex.
fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}
