//! Published by `art publish teams`. App code: change it freely.

use serde::Deserialize;
use validate::Validate;

use crate::access::{OrgRole, TeamRole};

#[derive(Deserialize, Validate)]
pub struct NewTeam {
    #[validate(required, max = 80)]
    pub name: String,
    /// Leave out for a team that stands alone.
    pub organization_id: Option<i64>,
}

#[derive(Deserialize, Validate)]
pub struct TeamName {
    #[validate(required, max = 80)]
    pub name: String,
}

#[derive(Deserialize, Validate)]
pub struct Invite {
    #[validate(required, email)]
    pub email: String,
    pub role: TeamRole,
}

#[derive(Deserialize, Validate)]
pub struct RoleChange {
    pub role: TeamRole,
}

#[derive(Deserialize, Validate)]
pub struct NewOrganization {
    #[validate(required, max = 80)]
    pub name: String,
}

#[derive(Deserialize, Validate)]
pub struct OrgInvite {
    #[validate(required, email)]
    pub email: String,
    pub role: OrgRole,
}

#[derive(Deserialize, Validate)]
pub struct OrgRoleChange {
    pub role: OrgRole,
}
