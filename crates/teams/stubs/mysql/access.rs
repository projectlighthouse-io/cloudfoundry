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
use teams::Store as _;
use validate::Errors;

use crate::state::Primary;

pub type Store = teams::MysqlStore<TeamRole, OrgRole>;
pub type Teams = teams::Teams<Store>;

pub fn teams(db: &Db<Primary>) -> Teams {
    teams::Teams::new(teams::MysqlStore::new(db.pool().clone()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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

/// The accept link. Anyone logged in with the invited email can open it.
pub fn invitation_link(token: &str) -> String {
    let base = std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    format!("{base}/invitations/{token}")
}

/// Logged for local development only: anyone who reads the log can join.
/// Send it by mail instead.
pub fn send_invitation(email: &str, token: &str) {
    tracing::info!("invitation link for {email}: {}", invitation_link(token));
}
