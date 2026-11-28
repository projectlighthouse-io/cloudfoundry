//! Published by `art publish teams --with-ui`. App code: change it freely.

use std::time::Duration;

use auth::Authed;
use route::{
    Body, Cx, Json, Path, Redirect,
    http::Response,
};
use serde::{Deserialize, Serialize};
use session::Session;
use teams::{Invitation, Org, OrgCan, OrgMembership, Uuid, Store, Target, Team};
use validate::Valid;

use crate::{
    access::{self, MemberItem, DeleteOrg, ManageOrgMembers, OrgRole, RenameOrg, TeamRole},
    requests::team::{NewOrganization, OrgInvite, OrgRoleChange},
    state::App,
    users::User,
};

type Result<T> = std::result::Result<T, Response<Body>>;

const INVITATION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize)]
pub struct OrgItem {
    uuid: Uuid<Org>,
    name: String,
    role: OrgRole,
}

#[derive(Serialize)]
pub struct OrgsPage {
    email: String,
    organizations: Vec<OrgItem>,
}

#[derive(Serialize)]
pub struct InvitationItem {
    uuid: Uuid<Invitation>,
    email: String,
    role: OrgRole,
    /// Unix seconds.
    expires_at: i64,
}

/// The link of the invitation just sent, flashed for the next page so it
/// can be copied: the token is not stored, so it cannot be shown later.
#[derive(Serialize, Deserialize)]
pub struct SentLink {
    uuid: Uuid<Invitation>,
    url: String,
}

const SENT_LINK: &str = "teams.invitation_link";

fn flash_link(app_url: &str, session: &Session, new: &teams::NewInvitation<TeamRole, OrgRole>) {
    let link = SentLink { uuid: new.invitation.uuid, url: access::invitation_link(app_url, &new.token) };
    if let Err(e) = session.flash(SENT_LINK, &link) {
        tracing::error!("flashing the invitation link: {e}");
    }
}

#[derive(Serialize)]
pub struct Abilities {
    rename: bool,
    delete: bool,
    manage_members: bool,
}

#[derive(Serialize)]
pub struct OrgPage {
    email: String,
    organization: Org,
    /// The viewer, to mark their own row.
    user_uuid: Uuid<User>,
    role: OrgRole,
    can: Abilities,
    members: Vec<MemberItem<OrgRole>>,
    teams: Vec<Team>,
    invitations: Vec<InvitationItem>,
    invitation_link: Option<SentLink>,
}

#[derive(Deserialize)]
pub struct MemberPath {
    user: Uuid<User>,
}

#[derive(Deserialize)]
pub struct InvitationPath {
    invitation: Uuid<Invitation>,
}

fn back(org: &Org) -> Redirect {
    Redirect::to(format!("/organizations/{}", org.uuid))
}

pub async fn index(cx: Cx<App>, user: Authed<User>) -> Result<inertia::Page<OrgsPage>> {
    let orgs = access::teams(&cx.db).store().orgs_of(user.id).await.map_err(access::failed)?;
    let organizations = orgs.into_iter().map(|(o, role)| OrgItem { uuid: o.uuid, name: o.name, role }).collect();
    Ok(inertia::render("organizations/index", OrgsPage { email: user.email.clone(), organizations }))
}

pub async fn store(cx: Cx<App>, user: Authed<User>, input: Valid<Json<NewOrganization>>) -> Result<Redirect> {
    let org = access::teams(&cx.db).create_org(user.id, input.name.trim()).await.map_err(access::failed)?;
    Ok(back(&org))
}

pub async fn show(cx: Cx<App>, user: Authed<User>, session: Session, m: OrgMembership<OrgRole>) -> Result<inertia::Page<OrgPage>> {
    let t = access::teams(&cx.db);
    let members = access::member_items(&cx.db, t.store().org_members(m.org.id).await.map_err(access::failed)?).await?;
    let teams = t.store().org_teams(m.org.id).await.map_err(access::failed)?;
    let invitations = t
        .store()
        .org_invitations(m.org.id)
        .await
        .map_err(access::failed)?
        .into_iter()
        .filter_map(|i| match i.target {
            Target::Org { role, .. } => Some(InvitationItem { uuid: i.uuid, email: i.email, role, expires_at: i.expires_at }),
            Target::Team { .. } => None,
        })
        .collect();
    let can = Abilities { rename: m.can::<RenameOrg>(), delete: m.can::<DeleteOrg>(), manage_members: m.can::<ManageOrgMembers>() };
    Ok(inertia::render(
        "organizations/show",
        OrgPage {
            email: user.email.clone(),
            user_uuid: user.uuid,
            role: m.role,
            can,
            organization: m.org,
            members,
            teams,
            invitations,
            invitation_link: session.get(SENT_LINK).ok().flatten(),
        },
    ))
}

pub async fn update(cx: Cx<App>, can: OrgCan<RenameOrg, OrgRole>, input: Valid<Json<NewOrganization>>) -> Result<Redirect> {
    access::teams(&cx.db).rename_org(&can.org, input.name.trim()).await.map_err(access::failed)?;
    Ok(back(&can.org))
}

pub async fn destroy(cx: Cx<App>, can: OrgCan<DeleteOrg, OrgRole>) -> Result<Redirect> {
    let t = access::teams(&cx.db);
    t.keeps_a_team(can.user_id, |team| team.org_id == Some(can.org.id)).await.map_err(access::failed)?;
    t.delete_org(&can.org).await.map_err(access::failed)?;
    Ok(Redirect::to("/organizations"))
}

pub async fn leave(cx: Cx<App>, m: OrgMembership<OrgRole>) -> Result<Redirect> {
    access::teams(&cx.db).remove_org_member(&m.org, m.user_id).await.map_err(access::failed)?;
    Ok(Redirect::to("/organizations"))
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

pub async fn update_member(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<MemberPath>, input: Valid<Json<OrgRoleChange>>) -> Result<Redirect> {
    let user = access::user_id(&cx.db, path.user).await?;
    guard_owner(&cx, &can, user, Some(input.role)).await?;
    access::teams(&cx.db).change_org_role(&can.org, user, input.role).await.map_err(access::failed)?;
    Ok(back(&can.org))
}

pub async fn remove_member(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<MemberPath>) -> Result<Redirect> {
    let user = access::user_id(&cx.db, path.user).await?;
    guard_owner(&cx, &can, user, None).await?;
    access::teams(&cx.db).remove_org_member(&can.org, user).await.map_err(access::failed)?;
    Ok(back(&can.org))
}

pub async fn invite(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, session: Session, input: Valid<Json<OrgInvite>>) -> Result<Redirect> {
    access::may_grant(can.role, input.role)?;
    let target = Target::Org { org: can.org.id, role: input.role };
    access::not_a_member(&cx.db, target, input.email.trim()).await?;
    let new = access::teams(&cx.db).invite(target, input.email.trim(), can.user_id, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    flash_link(&cx.cfg.app_url, &session, &new);
    Ok(back(&can.org))
}

/// A new link and a fresh lifetime; the old link stops working.
pub async fn resend(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, session: Session, Path(path): Path<InvitationPath>) -> Result<Redirect> {
    let new = access::teams(&cx.db).resend_org_invitation(&can.org, path.invitation, INVITATION_LIFETIME).await.map_err(access::failed)?;
    access::send_invitation(&cx.cfg.app_url, &new.invitation.email, &new.token);
    flash_link(&cx.cfg.app_url, &session, &new);
    Ok(back(&can.org))
}

pub async fn revoke(cx: Cx<App>, can: OrgCan<ManageOrgMembers, OrgRole>, Path(path): Path<InvitationPath>) -> Result<Redirect> {
    access::teams(&cx.db).revoke_org_invitation(&can.org, path.invitation).await.map_err(access::failed)?;
    Ok(back(&can.org))
}
