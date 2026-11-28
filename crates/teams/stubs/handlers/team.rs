//! Published by `art publish teams`. App code: change it freely.

use std::time::Duration;

use auth::Authed;
use route::{
    Body, Cx, IntoResponse, Json, Path,
    http::{Response, StatusCode},
};
use serde::{Deserialize, Serialize};
use teams::{Can, CurrentTeam, Invitation, Member, Membership, Store, Target, Team};
use validate::Valid;

use crate::{
    access::{self, DeleteTeam, InviteMembers, ManageMembers, OrgRole, RenameTeam, TeamRole},
    requests::team::{Invite, NewTeam, RoleChange, TeamName},
    state::App,
    users::User,
};

type Result<T> = std::result::Result<T, Response<Body>>;

/// How long an invitation link works.
const INVITATION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize)]
pub struct TeamSummary {
    #[serde(flatten)]
    team: Team,
    role: TeamRole,
    /// The role comes from managing the team's organization.
    via_org: bool,
}

#[derive(Serialize)]
pub struct TeamDetail {
    #[serde(flatten)]
    summary: TeamSummary,
    members: Vec<Member<TeamRole>>,
    invitations: Vec<Invitation<TeamRole, OrgRole>>,
}

fn summary(m: Membership<TeamRole>) -> TeamSummary {
    TeamSummary { team: m.team, role: m.role, via_org: m.via_org }
}

#[derive(Deserialize)]
pub struct MemberPath {
    user: i64,
}

#[derive(Deserialize)]
pub struct InvitationPath {
    invitation: i64,
}

/// Every team the user can open.
pub async fn index(cx: Cx<App>, user: Authed<User>) -> Result<Json<Vec<TeamSummary>>> {
    let teams = access::teams(&cx.db).teams_of(user.id).await.map_err(access::failed)?;
    Ok(Json(teams.into_iter().map(summary).collect()))
}

/// A new team, owned by the user. In an organization, the user must be a member of it.
pub async fn store(cx: Cx<App>, user: Authed<User>, input: Valid<Json<NewTeam>>) -> Result<(StatusCode, Json<Team>)> {
    let team = access::teams(&cx.db).create_team(user.id, input.organization_id, input.name.trim()).await.map_err(access::failed)?;
    Ok((StatusCode::CREATED, Json(team)))
}

pub async fn show(cx: Cx<App>, m: Membership<TeamRole>) -> Result<Json<TeamDetail>> {
    let teams = access::teams(&cx.db);
    let store = teams.store();
    let members = store.team_members(m.team.id).await.map_err(access::failed)?;
    let invitations = store.team_invitations(m.team.id).await.map_err(access::failed)?;
    Ok(Json(TeamDetail { summary: summary(m), members, invitations }))
}

pub async fn update(cx: Cx<App>, can: Can<RenameTeam, TeamRole>, input: Valid<Json<TeamName>>) -> Result<StatusCode> {
    access::teams(&cx.db).rename_team(&can.team, input.name.trim()).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy(cx: Cx<App>, can: Can<DeleteTeam, TeamRole>) -> Result<StatusCode> {
    let t = access::teams(&cx.db);
    t.keeps_a_team(can.user_id, |team| team.id == can.team.id).await.map_err(access::failed)?;
    t.delete_team(&can.team).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Make this the user's current team (where they land after login).
pub async fn switch(m: Membership<TeamRole>, mut current: CurrentTeam<TeamRole>) -> Result<StatusCode> {
    current.set(&m).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Leave the team. The last owner cannot.
pub async fn leave(cx: Cx<App>, m: Membership<TeamRole>) -> Result<StatusCode> {
    access::teams(&cx.db).remove_member(&m.team, m.user_id).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Only owners change an owner's role or make someone an owner.
async fn guard_owner(cx: &App, can: &Membership<TeamRole>, user: i64, role: Option<TeamRole>) -> Result<()> {
    let current = access::teams(&cx.db).store().team_role(can.team.id, user).await.map_err(access::failed)?;
    if current == Some(TeamRole::Owner) {
        access::may_grant(can.role, TeamRole::Owner)?;
    }
    if let Some(role) = role {
        access::may_grant(can.role, role)?;
    }
    Ok(())
}

pub async fn update_member(cx: Cx<App>, can: Can<ManageMembers, TeamRole>, Path(path): Path<MemberPath>, input: Valid<Json<RoleChange>>) -> Result<StatusCode> {
    guard_owner(&cx, &can, path.user, Some(input.role)).await?;
    access::teams(&cx.db).change_role(&can.team, path.user, input.role).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member(cx: Cx<App>, can: Can<ManageMembers, TeamRole>, Path(path): Path<MemberPath>) -> Result<StatusCode> {
    guard_owner(&cx, &can, path.user, None).await?;
    access::teams(&cx.db).remove_member(&can.team, path.user).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Invite by email. The link goes out through `access::send_invitation`.
pub async fn invite(cx: Cx<App>, can: Can<InviteMembers, TeamRole>, input: Valid<Json<Invite>>) -> Result<impl IntoResponse> {
    access::may_grant(can.role, input.role)?;
    let target = Target::Team { team: can.team.id, role: input.role };
    access::not_a_member(&cx.db, target, input.email.trim()).await?;
    let new = access::teams(&cx.db).invite(target, input.email.trim(), can.user_id, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    Ok((StatusCode::CREATED, Json(new.invitation)))
}

/// A new link and a fresh lifetime; the old link stops working.
pub async fn resend(cx: Cx<App>, can: Can<InviteMembers, TeamRole>, Path(path): Path<InvitationPath>) -> Result<impl IntoResponse> {
    let new = access::teams(&cx.db).resend_team_invitation(&can.team, path.invitation, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    Ok((StatusCode::CREATED, Json(new.invitation)))
}

pub async fn revoke(cx: Cx<App>, can: Can<InviteMembers, TeamRole>, Path(path): Path<InvitationPath>) -> Result<StatusCode> {
    access::teams(&cx.db).revoke_team_invitation(&can.team, path.invitation).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}
