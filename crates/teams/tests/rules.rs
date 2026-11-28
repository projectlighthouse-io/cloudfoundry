#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers

mod common;

use std::time::Duration;

use common::{OrgRole, TeamRole};
use teams::{Error, Store, Target, Teams};

const ADA: i64 = 1;
const BOB: i64 = 2;
const CY: i64 = 3;

#[tokio::test]
async fn a_team_keeps_its_last_owner() {
    let t = Teams::new(common::Store::new());
    let team = t.create_team(ADA, None, "Core").await.unwrap();
    assert_eq!(t.store().team_role(team.id, ADA).await.unwrap(), Some(TeamRole::Owner), "the creator owns it");

    assert!(matches!(t.change_role(&team, ADA, TeamRole::Member).await, Err(Error::LastOwner)));
    assert!(matches!(t.remove_member(&team, ADA).await, Err(Error::LastOwner)));

    t.add_member(&team, BOB, TeamRole::Member).await.unwrap();
    t.change_role(&team, BOB, TeamRole::Owner).await.unwrap();
    t.remove_member(&team, ADA).await.unwrap();
    assert_eq!(t.store().team_role(team.id, ADA).await.unwrap(), None, "with another owner, the first can leave");
    assert!(matches!(t.remove_member(&team, CY).await, Err(Error::NotMember)));
}

#[tokio::test]
async fn standalone_teams_share_names_org_teams_do_not() {
    let t = Teams::new(common::Store::new());
    t.create_team(ADA, None, "Core").await.unwrap();
    t.create_team(BOB, None, "Core").await.unwrap();

    let org = t.create_org(ADA, "Acme").await.unwrap();
    let core = t.create_team(ADA, Some(org.id), "Core").await.unwrap();
    assert!(matches!(t.create_team(ADA, Some(org.id), "Core").await, Err(Error::NameTaken)));
    let web = t.create_team(ADA, Some(org.id), "Web").await.unwrap();
    assert!(matches!(t.rename_team(&web, "Core").await, Err(Error::NameTaken)));
    t.rename_team(&core, "Platform").await.unwrap();
    t.rename_team(&web, "Core").await.unwrap();

    let other = t.create_org(BOB, "Other").await.unwrap();
    t.create_team(BOB, Some(other.id), "Core").await.unwrap();
}

#[tokio::test]
async fn org_teams_take_org_members_only() {
    let t = Teams::new(common::Store::new());
    let org = t.create_org(ADA, "Acme").await.unwrap();
    assert!(matches!(t.create_team(BOB, Some(org.id), "Core").await, Err(Error::NotOrgMember)));
    let team = t.create_team(ADA, Some(org.id), "Core").await.unwrap();
    assert!(matches!(t.add_member(&team, BOB, TeamRole::Member).await, Err(Error::NotOrgMember)));

    t.add_org_member(&org, BOB, OrgRole::Member).await.unwrap();
    t.add_member(&team, BOB, TeamRole::Member).await.unwrap();

    // Leaving the organization leaves its teams.
    t.remove_org_member(&org, BOB).await.unwrap();
    assert_eq!(t.store().team_role(team.id, BOB).await.unwrap(), None);
    assert!(matches!(t.remove_org_member(&org, ADA).await, Err(Error::LastOwner)));
}

#[tokio::test]
async fn org_managers_own_every_team_in_the_org() {
    let t = Teams::new(common::Store::new());
    let org = t.create_org(ADA, "Acme").await.unwrap();
    t.add_org_member(&org, BOB, OrgRole::Admin).await.unwrap();
    t.add_org_member(&org, CY, OrgRole::Member).await.unwrap();
    let team = t.create_team(CY, Some(org.id), "Core").await.unwrap();

    let bob = t.team_access(team.id, BOB).await.unwrap().unwrap();
    assert_eq!((bob.role, bob.via_org), (TeamRole::Owner, true));
    assert_eq!(t.teams_of(BOB).await.unwrap().len(), 1, "listed without a team membership");

    let standalone = t.create_team(ADA, None, "Side").await.unwrap();
    assert!(t.team_access(standalone.id, BOB).await.unwrap().is_none(), "org roles reach only the org's teams");

    t.change_org_role(&org, BOB, OrgRole::Member).await.unwrap();
    assert!(t.team_access(team.id, BOB).await.unwrap().is_none());
    assert!(t.teams_of(BOB).await.unwrap().is_empty());

    t.delete_org(&org).await.unwrap();
    assert!(t.store().team(team.id).await.unwrap().is_none(), "deleting the org deletes its teams");
    assert!(t.teams_of(CY).await.unwrap().is_empty());
}

#[tokio::test]
async fn invitations_join_with_their_role_once() {
    let t = Teams::new(common::Store::new());
    let team = t.create_team(ADA, None, "Core").await.unwrap();
    let week = Duration::from_secs(7 * 86400);
    let new = t.invite(Target::Team { team: team.id, role: TeamRole::Admin }, "bob@example.com", ADA, week).await.unwrap();
    assert_eq!(t.store().team_invitations(team.id).await.unwrap().len(), 1);

    assert!(matches!(t.accept("wrong", BOB, "bob@example.com").await, Err(Error::NotFound)));
    assert!(matches!(t.accept(&new.token, BOB, "eve@example.com").await, Err(Error::EmailMismatch)));
    let target = t.accept(&new.token, BOB, "BOB@example.com").await.unwrap();
    assert_eq!(target, Target::Team { team: team.id, role: TeamRole::Admin });
    assert_eq!(t.store().team_role(team.id, BOB).await.unwrap(), Some(TeamRole::Admin));
    assert!(matches!(t.accept(&new.token, BOB, "bob@example.com").await, Err(Error::NotFound)), "used up");

    let expired = t.invite(Target::Team { team: team.id, role: TeamRole::Member }, "cy@example.com", ADA, Duration::ZERO).await.unwrap();
    assert!(matches!(t.accept(&expired.token, CY, "cy@example.com").await, Err(Error::InvitationExpired)));
    assert!(t.store().team_invitations(team.id).await.unwrap().is_empty(), "expired ones are deleted");
}

#[tokio::test]
async fn org_team_invitations_need_org_membership() {
    let t = Teams::new(common::Store::new());
    let org = t.create_org(ADA, "Acme").await.unwrap();
    let team = t.create_team(ADA, Some(org.id), "Core").await.unwrap();
    let day = Duration::from_secs(86400);

    let to_team = t.invite(Target::Team { team: team.id, role: TeamRole::Member }, "bob@example.com", ADA, day).await.unwrap();
    assert!(matches!(t.accept(&to_team.token, BOB, "bob@example.com").await, Err(Error::NotOrgMember)));

    let to_org = t.invite(Target::Org { org: org.id, role: OrgRole::Member }, "bob@example.com", ADA, day).await.unwrap();
    t.accept(&to_org.token, BOB, "bob@example.com").await.unwrap();
    t.accept(&to_team.token, BOB, "bob@example.com").await.unwrap();
    assert_eq!(t.store().team_role(team.id, BOB).await.unwrap(), Some(TeamRole::Member));

    let pending = t.invite(Target::Org { org: org.id, role: OrgRole::Admin }, "cy@example.com", ADA, day).await.unwrap().invitation.id;
    let other = t.create_org(BOB, "Other").await.unwrap();
    assert!(matches!(t.revoke_org_invitation(&other, pending).await, Err(Error::NotFound)), "only the org's own invitations");
    assert!(matches!(t.revoke_team_invitation(&team, pending).await, Err(Error::NotFound)));
    t.revoke_org_invitation(&org, pending).await.unwrap();
    assert!(t.store().org_invitations(org.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn one_pending_invitation_per_email_and_resend_rotates_it() {
    let t = Teams::new(common::Store::new());
    let team = t.create_team(ADA, None, "Core").await.unwrap();
    let day = Duration::from_secs(86400);
    let target = Target::Team { team: team.id, role: TeamRole::Member };

    let first = t.invite(target, "bob@example.com", ADA, day).await.unwrap();
    assert!(matches!(t.invite(target, "BOB@example.com", ADA, day).await, Err(Error::AlreadyInvited)), "ignores case");
    t.invite(Target::Team { team: t.create_team(ADA, None, "Ops").await.unwrap().id, role: TeamRole::Member }, "bob@example.com", ADA, day).await.unwrap();

    let again = t.resend_team_invitation(&team, first.invitation.id, day).await.unwrap();
    assert_ne!(again.token, first.token);
    assert_eq!(again.invitation.email, "bob@example.com");
    assert!(matches!(t.accept(&first.token, BOB, "bob@example.com").await, Err(Error::NotFound)), "the old link stops working");
    assert_eq!(t.store().team_invitations(team.id).await.unwrap().len(), 1);
    t.accept(&again.token, BOB, "bob@example.com").await.unwrap();

    t.invite(target, "cy@example.com", ADA, Duration::ZERO).await.unwrap();
    t.invite(target, "cy@example.com", ADA, day).await.unwrap();
    assert_eq!(t.store().team_invitations(team.id).await.unwrap().len(), 1, "an expired one is replaced");

    let other = t.create_team(BOB, None, "Other").await.unwrap();
    assert!(matches!(t.resend_team_invitation(&other, again.invitation.id, day).await, Err(Error::NotFound)));
}
