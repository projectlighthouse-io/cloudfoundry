//! Roles and permissions an app would define.
#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
}

impl teams::Role for TeamRole {
    const OWNER: Self = Self::Owner;
    fn key(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

impl teams::Role for OrgRole {
    const OWNER: Self = Self::Owner;
    fn key(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

impl teams::OrgRole for OrgRole {
    fn manages_teams(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

pub struct InviteMember;
pub struct DeleteTeam;
pub struct ManageBilling;
impl teams::Permission for InviteMember {}
impl teams::Permission for DeleteTeam {}
impl teams::Permission for ManageBilling {}

impl teams::Grants<InviteMember> for TeamRole {
    fn grants(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl teams::Grants<DeleteTeam> for TeamRole {
    fn grants(&self) -> bool {
        *self == Self::Owner
    }
}

impl teams::Grants<ManageBilling> for OrgRole {
    fn grants(&self) -> bool {
        *self == Self::Owner
    }
}

pub type Store = teams::MemoryStore<TeamRole, OrgRole>;
