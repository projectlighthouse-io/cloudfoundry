//! Published by `art publish teams --with-ui`. App code: change it freely.

use std::time::Duration;

use auth::{Authed, Provider};
use route::{
    Body, Cx, Json, Path, Redirect,
    http::Response,
};
use serde::{Deserialize, Serialize};
use teams::{Can, CurrentTeam, Membership, Org, Store, Target, Team};
use validate::Valid;

use crate::{
    access::{self, DeleteTeam, InviteMembers, ManageMembers, RenameTeam, TeamRole},
    requests::team::{Invite, NewTeam, RoleChange, TeamName},
    state::App,
    users::{User, Users},
};

type Result<T> = std::result::Result<T, Response<Body>>;

const INVITATION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize)]
pub struct TeamItem {
    id: i64,
    name: String,
    organization: Option<String>,
    role: TeamRole,
    current: bool,
}

#[derive(Serialize)]
pub struct TeamsPage {
    email: String,
    teams: Vec<TeamItem>,
    /// Organizations the user can create teams in.
    organizations: Vec<Org>,
}

#[derive(Serialize)]
pub struct MemberItem {
    id: i64,
    email: String,
    name: Option<String>,
    role: TeamRole,
}

#[derive(Serialize)]
pub struct InvitationItem {
    id: i64,
    email: String,
    role: TeamRole,
}

#[derive(Serialize)]
pub struct Abilities {
    rename: bool,
    delete: bool,
    manage_members: bool,
    invite: bool,
}

#[derive(Serialize)]
pub struct TeamPage {
    email: String,
    team: Team,
    organization: Option<Org>,
    user_id: i64,
    role: TeamRole,
    via_org: bool,
    can: Abilities,
    members: Vec<MemberItem>,
    invitations: Vec<InvitationItem>,
}

#[derive(Deserialize)]
pub struct MemberPath {
    user: i64,
}

#[derive(Deserialize)]
pub struct InvitationPath {
    invitation: i64,
}

fn back(team: &Team) -> Redirect {
    Redirect::to(format!("/teams/{}", team.id))
}

pub async fn index(cx: Cx<App>, (user, current): (Authed<User>, CurrentTeam<TeamRole>)) -> Result<inertia::Page<TeamsPage>> {
    let t = access::teams(&cx.db);
    let current = current.as_ref().map(|m| m.team.id);
    let orgs = t.store().orgs_of(user.id).await.map_err(access::failed)?;
    let mut teams = Vec::new();
    for m in t.teams_of(user.id).await.map_err(access::failed)? {
        let organization = m.team.org_id.and_then(|id| orgs.iter().find(|(o, _)| o.id == id).map(|(o, _)| o.name.clone()));
        teams.push(TeamItem { id: m.team.id, current: current == Some(m.team.id), name: m.team.name, organization, role: m.role });
    }
    let organizations = orgs.into_iter().map(|(o, _)| o).collect();
    Ok(inertia::render("teams/index", TeamsPage { email: user.email.clone(), teams, organizations }))
}

pub async fn store(cx: Cx<App>, (user, input): (Authed<User>, Valid<Json<NewTeam>>)) -> Result<Redirect> {
    let team = access::teams(&cx.db).create_team(user.id, input.organization_id, input.name.trim()).await.map_err(access::failed)?;
    Ok(back(&team))
}

pub async fn show(cx: Cx<App>, (user, m): (Authed<User>, Membership<TeamRole>)) -> Result<inertia::Page<TeamPage>> {
    let t = access::teams(&cx.db);
    let users = Users { db: cx.db.clone() };
    let mut members = Vec::new();
    // ponytail: one query per member; fine for team-sized lists, batch it if teams get large.
    for member in t.store().team_members(m.team.id).await.map_err(access::failed)? {
        if let Ok(Some(u)) = users.find(member.user_id).await {
            members.push(MemberItem { id: u.id, email: u.email, name: u.name, role: member.role });
        }
    }
    let invitations = t
        .store()
        .team_invitations(m.team.id)
        .await
        .map_err(access::failed)?
        .into_iter()
        .filter_map(|i| match i.target {
            Target::Team { role, .. } => Some(InvitationItem { id: i.id, email: i.email, role }),
            Target::Org { .. } => None,
        })
        .collect();
    let organization = match m.team.org_id {
        Some(id) => t.store().org(id).await.map_err(access::failed)?,
        None => None,
    };
    let can = Abilities { rename: m.can::<RenameTeam>(), delete: m.can::<DeleteTeam>(), manage_members: m.can::<ManageMembers>(), invite: m.can::<InviteMembers>() };
    Ok(inertia::render(
        "teams/show",
        TeamPage { email: user.email.clone(), user_id: m.user_id, role: m.role, via_org: m.via_org, can, team: m.team, organization, members, invitations },
    ))
}

pub async fn update(cx: Cx<App>, (can, input): (Can<RenameTeam, TeamRole>, Valid<Json<TeamName>>)) -> Result<Redirect> {
    access::teams(&cx.db).rename_team(&can.team, input.name.trim()).await.map_err(access::failed)?;
    Ok(back(&can.team))
}

pub async fn destroy(cx: Cx<App>, can: Can<DeleteTeam, TeamRole>) -> Result<Redirect> {
    access::teams(&cx.db).delete_team(&can.team).await.map_err(access::failed)?;
    Ok(Redirect::to("/teams"))
}

pub async fn switch((m, mut current): (Membership<TeamRole>, CurrentTeam<TeamRole>)) -> Redirect {
    current.set(&m);
    back(&m.team)
}

pub async fn leave(cx: Cx<App>, m: Membership<TeamRole>) -> Result<Redirect> {
    access::teams(&cx.db).remove_member(&m.team, m.user_id).await.map_err(access::failed)?;
    Ok(Redirect::to("/teams"))
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

pub async fn update_member(cx: Cx<App>, (can, Path(path), input): (Can<ManageMembers, TeamRole>, Path<MemberPath>, Valid<Json<RoleChange>>)) -> Result<Redirect> {
    guard_owner(&cx, &can, path.user, Some(input.role)).await?;
    access::teams(&cx.db).change_role(&can.team, path.user, input.role).await.map_err(access::failed)?;
    Ok(back(&can.team))
}

pub async fn remove_member(cx: Cx<App>, (can, Path(path)): (Can<ManageMembers, TeamRole>, Path<MemberPath>)) -> Result<Redirect> {
    guard_owner(&cx, &can, path.user, None).await?;
    access::teams(&cx.db).remove_member(&can.team, path.user).await.map_err(access::failed)?;
    Ok(back(&can.team))
}

pub async fn invite(cx: Cx<App>, (can, input): (Can<InviteMembers, TeamRole>, Valid<Json<Invite>>)) -> Result<Redirect> {
    access::may_grant(can.role, input.role)?;
    let target = Target::Team { team: can.team.id, role: input.role };
    let new = access::teams(&cx.db).invite(target, input.email.trim(), can.user_id, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&new.invitation.email, &new.token);
    Ok(back(&can.team))
}

pub async fn revoke(cx: Cx<App>, (can, Path(path)): (Can<InviteMembers, TeamRole>, Path<InvitationPath>)) -> Result<Redirect> {
    access::teams(&cx.db).revoke_team_invitation(&can.team, path.invitation).await.map_err(access::failed)?;
    Ok(back(&can.team))
}
