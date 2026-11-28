//! Published by `art publish teams`. App code: change it freely.
//!
//! Roles are enums; each permission is a type. Which roles have a
//! permission is one `Grants` impl: a handler taking `Can<P, TeamRole>`
//! runs only when the user's role grants `P`.

use db::Db;
use route::{
    Body, IntoResponse,
    http::{Response, StatusCode},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use teams::Store as _;
use validate::Errors;

use crate::state::Primary;

pub type Store = teams::PostgresStore<TeamRole, OrgRole>;
pub type Teams = teams::Teams<Store>;

/// Every user has a team: one with none gets a personal team on their next
/// request that resolves `CurrentTeam`, and cannot leave their last one.
/// Drop `.require_team(...)` to make teams optional.
pub fn teams(db: &Db<Primary>) -> Teams {
    teams::Teams::new(teams::PostgresStore::new(db.pool().clone())).require_team(|_user| "Personal".to_owned())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
}

macro_rules! keys {
    ($role:ty { $($variant:ident = $key:literal),* $(,)? }) => {
        impl teams::Role for $role {
            const OWNER: Self = Self::Owner;
            fn key(&self) -> &'static str {
                match self { $(Self::$variant => $key),* }
            }
            fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some(Self::$variant),)* _ => None }
            }
        }
    };
}

// The stored keys. Renaming a variant is fine; changing a key needs a migration.
keys!(TeamRole { Owner = "owner", Admin = "admin", Member = "member" });
keys!(OrgRole { Owner = "owner", Admin = "admin", Member = "member" });

impl teams::OrgRole for OrgRole {
    /// Organization owners and admins manage every team in it, as owners.
    fn manages_teams(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

/// Declare permissions: `permissions!(A, B);` makes unit structs that implement `teams::Permission`.
macro_rules! permissions {
    ($($name:ident),* $(,)?) => {$(
        pub struct $name;
        impl teams::Permission for $name {}
    )*};
}

permissions!(RenameTeam, DeleteTeam, ManageMembers, InviteMembers);
permissions!(RenameOrg, DeleteOrg, ManageOrgMembers);

impl teams::Grants<RenameTeam> for TeamRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl teams::Grants<DeleteTeam> for TeamRole {
    fn grants(&self) -> bool {
        *self == Self::Owner
    }
}

impl teams::Grants<ManageMembers> for TeamRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl teams::Grants<InviteMembers> for TeamRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl teams::Grants<RenameOrg> for OrgRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl teams::Grants<DeleteOrg> for OrgRole {
    fn grants(&self) -> bool {
        *self == Self::Owner
    }
}

impl teams::Grants<ManageOrgMembers> for OrgRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

/// A rule broken or a store failure, as a response: `422` with the field
/// to show it under, `404`, `403`, or `500` (logged).
pub fn failed(e: teams::Error) -> Response<Body> {
    match e {
        teams::Error::NameTaken => invalid("name", "Another team in this organization has that name."),
        teams::Error::LastOwner => invalid("role", "Make someone else an owner first."),
        teams::Error::NotOrgMember => invalid("email", "Invite them to the organization first."),
        teams::Error::NotMember | teams::Error::NotFound => StatusCode::NOT_FOUND.into_response(),
        teams::Error::InvitationExpired => invalid("token", "This invitation has expired. Ask for a new one."),
        teams::Error::EmailMismatch => invalid("token", "This invitation was sent to another email address."),
        teams::Error::LastTeam => invalid("team", "You need at least one team. Create or join another one first."),
        teams::Error::AlreadyInvited => invalid("email", "They already have a pending invitation. Send it again from the list."),
        teams::Error::Store(e) => {
            tracing::error!("teams: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Only owners make owners: no one hands out a role above their own.
pub fn may_grant<R: teams::Role>(actor: R, role: R) -> Result<(), Response<Body>> {
    if role == R::OWNER && actor != R::OWNER { Err(StatusCode::FORBIDDEN.into_response()) } else { Ok(()) }
}

/// A 422 with one message on `field`, as validation errors are.
pub fn invalid(field: &'static str, message: &'static str) -> Response<Body> {
    let mut errors = Errors::new();
    errors.add(field, message);
    errors.into_response()
}

/// 422 on `email` when that address already belongs to a member of the
/// team or organization. The store keeps user ids, so this is the app's check.
pub async fn not_a_member(db: &Db<Primary>, target: teams::Target<TeamRole, OrgRole>, email: &str) -> Result<(), Response<Body>> {
    let user = match (crate::users::Users { db: db.clone() }).by_email(email).await {
        Ok(Some(user)) => user,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::error!("finding a user by email: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    let t = teams(db);
    let member = match target {
        teams::Target::Team { team, .. } => t.store().team_role(team, user.id).await.map(|r| r.is_some()),
        teams::Target::Org { org, .. } => t.store().org_role(org, user.id).await.map(|r| r.is_some()),
    }
    .map_err(failed)?;
    if member { Err(invalid("email", "They are already a member.")) } else { Ok(()) }
}

/// A member as responses show them: by the user's `uuid`, never their id.
#[derive(Serialize, TS)]
pub struct MemberItem<R> {
    pub uuid: teams::Uuid<crate::users::User>,
    pub name: Option<String>,
    pub email: String,
    pub role: R,
}

/// Members with their users' uuids, names and emails. Members whose user is gone are left out.
pub async fn member_items<R>(db: &Db<Primary>, members: Vec<teams::Member<R>>) -> Result<Vec<MemberItem<R>>, Response<Body>> {
    let users = crate::users::Users { db: db.clone() };
    let mut out = Vec::with_capacity(members.len());
    // ponytail: one query per member; fine for team-sized lists, batch it if teams get large.
    for m in members {
        match auth::Provider::find(&users, m.user_id).await {
            Ok(Some(u)) => out.push(MemberItem { uuid: u.uuid, name: u.name, email: u.email, role: m.role }),
            Ok(None) => {}
            Err(e) => {
                tracing::error!("loading a member: {e}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        }
    }
    Ok(out)
}

/// The internal id of the user `{user}` names. `404` if there is none.
pub async fn user_id(db: &Db<Primary>, user: teams::Uuid<crate::users::User>) -> Result<i64, Response<Body>> {
    match (crate::users::Users { db: db.clone() }).by_uuid(user).await {
        Ok(Some(u)) => Ok(u.id),
        Ok(None) => Err(StatusCode::NOT_FOUND.into_response()),
        Err(e) => {
            tracing::error!("finding a user by uuid: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// The internal id of the organization a new team goes in, from its public
/// id. `404` for an id that names no organization.
pub async fn org_id(db: &Db<Primary>, org: Option<teams::Uuid<teams::Org>>) -> Result<Option<i64>, Response<Body>> {
    let Some(org) = org else { return Ok(None) };
    match teams(db).store().org_by_uuid(org).await.map_err(failed)? {
        Some(found) => Ok(Some(found.id)),
        None => Err(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Where an accepted invitation leads: `/teams/{uuid}` or
/// `/organizations/{uuid}`. `404` if it is gone.
pub async fn joined_path(db: &Db<Primary>, target: teams::Target<TeamRole, OrgRole>) -> Result<String, Response<Body>> {
    let t = teams(db);
    let path = match target {
        teams::Target::Team { team, .. } => t.store().team(team).await.map_err(failed)?.map(|t| format!("/teams/{}", t.uuid)),
        teams::Target::Org { org, .. } => t.store().org(org).await.map_err(failed)?.map(|o| format!("/organizations/{}", o.uuid)),
    };
    path.ok_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// The accept link. Anyone logged in with the invited email can open it.
pub fn invitation_link(app_url: &str, token: &str) -> String {
    format!("{app_url}/invitations/{token}")
}

/// Logged for local development only: anyone who reads the log can join.
/// Send it by mail instead.
pub fn send_invitation(app_url: &str, email: &str, token: &str) {
    tracing::info!("invitation link for {email}: {}", invitation_link(app_url, token));
}
