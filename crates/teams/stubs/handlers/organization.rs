//! Published by `art publish teams`. App code: change it freely.

use std::time::Duration;

use auth::Authed;
use route::{
    Body, Cx, IntoResponse, Json, Path,
    http::{Response, StatusCode},
};
use serde::{Deserialize, Serialize};
use teams::{Invitation, Member, Org, OrgCan, OrgMembership, Store, Target, Team};
use validate::Valid;

use crate::{
    access::{self, DeleteOrg, ManageOrgMembers, OrgRole, RenameOrg, TeamRole},
    requests::team::{NewOrganization, OrgInvite, OrgRoleChange},
    state::App,
    users::User,
};

type Result<T> = std::result::Result<T, Response<Body>>;

const INVITATION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize)]
pub struct OrgSummary {
    #[serde(flatten)]
    org: Org,
    role: OrgRole,
}

#[derive(Serialize)]
pub struct OrgDetail {
    #[serde(flatten)]
    summary: OrgSummary,
    members: Vec<Member<OrgRole>>,
    teams: Vec<Team>,
    invitations: Vec<Invitation<TeamRole, OrgRole>>,
}

#[derive(Deserialize)]
pub struct MemberPath {
    user: i64,
}

#[derive(Deserialize)]
pub struct InvitationPath {
    invitation: i64,
}

pub async fn index(cx: Cx<App>, user: Authed<User>) -> Result<Json<Vec<OrgSummary>>> {
    let orgs = access::teams(&cx.db).store().orgs_of(user.id).await.map_err(access::failed)?;
    Ok(Json(orgs.into_iter().map(|(org, role)| OrgSummary { org, role }).collect()))
}

/// A new organization, owned by the user.
pub async fn store(cx: Cx<App>, user: Authed<User>, input: Valid<Json<NewOrganization>>) -> Result<(StatusCode, Json<Org>)> {
    let org = access::teams(&cx.db).create_org(user.id, input.name.trim()).await.map_err(access::failed)?;
    Ok((StatusCode::CREATED, Json(org)))
}

pub async fn show(cx: Cx<App>, m: OrgMembership<OrgRole>) -> Result<Json<OrgDetail>> {
    let teams = access::teams(&cx.db);
    let store = teams.store();
    let members = store.org_members(m.org.id).await.map_err(access::failed)?;
    let org_teams = store.org_teams(m.org.id).await.map_err(access::failed)?;
    let invitations = store.org_invitations(m.org.id).await.map_err(access::failed)?;
    Ok(Json(OrgDetail { summary: OrgSummary { org: m.org, role: m.role }, members, teams: org_teams, invitations }))
}

pub async fn update(cx: Cx<App>, can: OrgCan<RenameOrg, OrgRole>, input: Valid<Json<NewOrganization>>) -> Result<StatusCode> {
    access::teams(&cx.db).rename_org(&can.org, input.name.trim()).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes its teams too.
pub async fn destroy(cx: Cx<App>, can: OrgCan<DeleteOrg, OrgRole>) -> Result<StatusCode> {
    let t = access::teams(&cx.db);
    t.keeps_a_team(can.user_id, |team| team.org_id == Some(can.org.id)).await.map_err(access::failed)?;
    t.delete_org(&can.org).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Leave the organization and its teams. The last owner cannot.
pub async fn leave(cx: Cx<App>, m: OrgMembership<OrgRole>) -> Result<StatusCode> {
    access::teams(&cx.db).remove_org_member(&m.org, m.user_id).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn guard_owner(cx: &App, can: &OrgMembership<OrgRole>, user: i64, role: Option<OrgRole>) -> Result<()> {
    let current = access::teams(&cx.db).store().org_role(can.org.id, user).await.map_err(access::failed)?;
    if current == Some(OrgRole::Owner) {
        access::may_grant(can.role, OrgRole::Owner)?;
    }
    if let Some(role) = role {
        access::may_grant(can.role, role)?;
    }
    Ok(())
}

pub async fn update_member(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<MemberPath>, input: Valid<Json<OrgRoleChange>>) -> Result<StatusCode> {
    guard_owner(&cx, &can, path.user, Some(input.role)).await?;
    access::teams(&cx.db).change_org_role(&can.org, path.user, input.role).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes them from the organization's teams too.
pub async fn remove_member(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<MemberPath>) -> Result<StatusCode> {
    guard_owner(&cx, &can, path.user, None).await?;
    access::teams(&cx.db).remove_org_member(&can.org, path.user).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn invite(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, input: Valid<Json<OrgInvite>>) -> Result<impl IntoResponse> {
    access::may_grant(can.role, input.role)?;
    let target = Target::Org { org: can.org.id, role: input.role };
    access::not_a_member(&cx.db, target, input.email.trim()).await?;
    let new = access::teams(&cx.db).invite(target, input.email.trim(), can.user_id, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    Ok((StatusCode::CREATED, Json(new.invitation)))
}

/// A new link and a fresh lifetime; the old link stops working.
pub async fn resend(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<InvitationPath>) -> Result<impl IntoResponse> {
    let new = access::teams(&cx.db).resend_org_invitation(&can.org, path.invitation, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    Ok((StatusCode::CREATED, Json(new.invitation)))
}

pub async fn revoke(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<InvitationPath>) -> Result<StatusCode> {
    access::teams(&cx.db).revoke_org_invitation(&can.org, path.invitation).await.map_err(access::failed)?;
    Ok(StatusCode::NO_CONTENT)
}
