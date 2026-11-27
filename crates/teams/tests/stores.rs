#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
//! The same checks against every store. Memory and SQLite run always;
//! Postgres and MySQL run when TEAMS_PG_URL / TEAMS_MYSQL_URL are set.

mod common;

use std::time::Duration;

use common::{OrgRole, TeamRole};
use teams::{Error, Member, Store, Target, Teams};

async fn check<S: Store<Role = TeamRole, OrgRole = OrgRole>>(t: Teams<S>) {
    let s = t.store();
    let acme = t.create_org(1, "Acme").await.unwrap();
    assert_eq!(s.org(acme.id).await.unwrap().unwrap().name, "Acme");
    t.rename_org(&acme, "Acme Inc").await.unwrap();
    assert_eq!(s.org(acme.id).await.unwrap().unwrap().name, "Acme Inc");
    t.add_org_member(&acme, 2, OrgRole::Member).await.unwrap();
    t.add_org_member(&acme, 2, OrgRole::Admin).await.unwrap();
    assert_eq!(s.org_members(acme.id).await.unwrap(), [Member { user_id: 1, role: OrgRole::Owner }, Member { user_id: 2, role: OrgRole::Admin }]);
    assert_eq!(s.orgs_of(2).await.unwrap().len(), 1);

    let core = t.create_team(1, Some(acme.id), "Core").await.unwrap();
    assert_eq!(core.org_id, Some(acme.id));
    assert!(matches!(t.create_team(2, Some(acme.id), "Core").await, Err(Error::NameTaken)));
    let web = t.create_team(2, Some(acme.id), "Web").await.unwrap();
    assert!(matches!(t.rename_team(&web, "Core").await, Err(Error::NameTaken)));
    let side = t.create_team(3, None, "Core").await.unwrap();
    let side2 = t.create_team(3, None, "Core").await.unwrap();
    assert_ne!(side.id, side2.id, "standalone teams share names");
    assert_eq!(s.team(side.id).await.unwrap().unwrap().org_id, None);

    t.add_member(&core, 2, TeamRole::Member).await.unwrap();
    t.change_role(&core, 2, TeamRole::Admin).await.unwrap();
    assert_eq!(s.team_role(core.id, 2).await.unwrap(), Some(TeamRole::Admin));
    assert!(matches!(t.change_role(&core, 1, TeamRole::Member).await, Err(Error::LastOwner)));
    assert_eq!(s.team_members(core.id).await.unwrap().len(), 2);
    assert_eq!(s.teams_of(2).await.unwrap().iter().map(|(t, _)| t.id).collect::<Vec<_>>(), [core.id, web.id]);
    assert_eq!(s.org_teams(acme.id).await.unwrap().len(), 2);

    // Invitations round-trip their typed target.
    let day = Duration::from_secs(86400);
    let new = t.invite(Target::Team { team: core.id, role: TeamRole::Member }, "cy@example.com", 1, day).await.unwrap();
    assert_eq!(s.team_invitations(core.id).await.unwrap(), std::slice::from_ref(&new.invitation));
    let org_invite = t.invite(Target::Org { org: acme.id, role: OrgRole::Member }, "cy@example.com", 1, day).await.unwrap();
    assert_eq!(s.org_invitations(acme.id).await.unwrap(), std::slice::from_ref(&org_invite.invitation));
    t.accept(&org_invite.token, 4, "cy@example.com").await.unwrap();
    t.accept(&new.token, 4, "cy@example.com").await.unwrap();
    assert_eq!(s.team_role(core.id, 4).await.unwrap(), Some(TeamRole::Member));
    assert!(s.team_invitations(core.id).await.unwrap().is_empty());

    // Leaving the organization leaves its teams; deleting cascades.
    t.remove_org_member(&acme, 4).await.unwrap();
    assert_eq!(s.team_role(core.id, 4).await.unwrap(), None);
    t.invite(Target::Team { team: side.id, role: TeamRole::Member }, "dee@example.com", 3, day).await.unwrap();
    t.delete_team(&side).await.unwrap();
    assert!(s.team(side.id).await.unwrap().is_none());
    assert!(s.team_invitations(side.id).await.unwrap().is_empty());
    t.invite(Target::Team { team: core.id, role: TeamRole::Member }, "dee@example.com", 1, day).await.unwrap();
    t.delete_org(&acme).await.unwrap();
    assert!(s.org(acme.id).await.unwrap().is_none());
    assert!(s.team(core.id).await.unwrap().is_none());
    assert!(s.teams_of(2).await.unwrap().is_empty());
    assert!(s.team_invitations(core.id).await.unwrap().is_empty());
    assert_eq!(s.teams_of(3).await.unwrap().len(), 1, "other teams untouched");
}

#[tokio::test]
async fn memory() {
    check(Teams::new(common::Store::new())).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite() {
    // One connection, so the in-memory database is shared.
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/sqlite/create_teams.up.sql")).execute(&pool).await.unwrap();
    check(Teams::new(teams::SqliteStore::new(pool))).await;
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
const DROP: &str = "drop table if exists team_invitations; drop table if exists team_members; drop table if exists teams; \
                    drop table if exists organization_members; drop table if exists organizations";

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres() {
    let Ok(url) = std::env::var("TEAMS_PG_URL") else { return eprintln!("TEAMS_PG_URL not set; skipping") };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(DROP).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/create_teams.up.sql")).execute(&pool).await.unwrap();
    check(Teams::new(teams::PostgresStore::new(pool))).await;
}

#[cfg(feature = "mysql")]
#[tokio::test(flavor = "multi_thread")]
async fn mysql() {
    let Ok(url) = std::env::var("TEAMS_MYSQL_URL") else { return eprintln!("TEAMS_MYSQL_URL not set; skipping") };
    let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
    sqlx::raw_sql(DROP).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../stubs/migrations/mysql/create_teams.up.sql")).execute(&pool).await.unwrap();
    check(Teams::new(teams::MysqlStore::new(pool))).await;
}
