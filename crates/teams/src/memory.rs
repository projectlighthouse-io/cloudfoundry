use std::{
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::{Error, Invitation, Member, Org, OrgRole, Role, Store, Target, Team};

/// Everything in memory, for tests and prototypes. Lost on restart.
pub struct MemoryStore<R, O> {
    state: Mutex<State<R, O>>,
    roles: PhantomData<fn() -> (R, O)>,
}

struct State<R, O> {
    next: i64,
    orgs: BTreeMap<i64, Org>,
    teams: BTreeMap<i64, Team>,
    org_members: BTreeMap<(i64, i64), O>,
    team_members: BTreeMap<(i64, i64), R>,
    invitations: BTreeMap<i64, (Invitation<R, O>, String)>,
    current: BTreeMap<i64, i64>,
}

impl<R, O> Default for MemoryStore<R, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R, O> MemoryStore<R, O> {
    pub fn new() -> Self {
        let state = State {
            next: 0,
            orgs: BTreeMap::new(),
            teams: BTreeMap::new(),
            org_members: BTreeMap::new(),
            team_members: BTreeMap::new(),
            invitations: BTreeMap::new(),
            current: BTreeMap::new(),
        };
        Self { state: Mutex::new(state), roles: PhantomData }
    }

    /// The state, even if a panicking caller poisoned the lock: each critical
    /// section is a few plain map updates, so the data is still consistent.
    fn lock(&self) -> MutexGuard<'_, State<R, O>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<R, O> State<R, O> {
    fn id(&mut self) -> i64 {
        self.next += 1;
        self.next
    }

    fn name_taken(&self, org: Option<i64>, name: &str, except: Option<i64>) -> bool {
        org.is_some() && self.teams.values().any(|t| t.org_id == org && t.name == name && Some(t.id) != except)
    }

    fn drop_team(&mut self, id: i64) {
        self.teams.remove(&id);
        self.team_members.retain(|(t, _), _| *t != id);
        self.invitations.retain(|_, (i, _)| !matches!(i.target, Target::Team { team, .. } if team == id));
    }
}

impl<R: Role, O: OrgRole> Store for MemoryStore<R, O> {
    type Role = R;
    type OrgRole = O;

    async fn create_org(&self, owner: i64, name: &str) -> Result<Org, Error> {
        let mut s = self.lock();
        let org = Org { id: s.id(), name: name.to_owned() };
        s.orgs.insert(org.id, org.clone());
        s.org_members.insert((org.id, owner), O::OWNER);
        Ok(org)
    }

    async fn org(&self, id: i64) -> Result<Option<Org>, Error> {
        Ok(self.lock().orgs.get(&id).cloned())
    }

    async fn rename_org(&self, id: i64, name: &str) -> Result<(), Error> {
        if let Some(org) = self.lock().orgs.get_mut(&id) {
            name.clone_into(&mut org.name);
        }
        Ok(())
    }

    async fn delete_org(&self, id: i64) -> Result<(), Error> {
        let mut s = self.lock();
        let teams: Vec<i64> = s.teams.values().filter(|t| t.org_id == Some(id)).map(|t| t.id).collect();
        for team in teams {
            s.drop_team(team);
        }
        s.orgs.remove(&id);
        s.org_members.retain(|(o, _), _| *o != id);
        s.invitations.retain(|_, (i, _)| !matches!(i.target, Target::Org { org, .. } if org == id));
        Ok(())
    }

    async fn org_role(&self, org: i64, user: i64) -> Result<Option<O>, Error> {
        Ok(self.lock().org_members.get(&(org, user)).copied())
    }

    async fn set_org_role(&self, org: i64, user: i64, role: O) -> Result<(), Error> {
        self.lock().org_members.insert((org, user), role);
        Ok(())
    }

    async fn remove_org_member(&self, org: i64, user: i64) -> Result<(), Error> {
        let mut s = self.lock();
        s.org_members.remove(&(org, user));
        let teams: Vec<i64> = s.teams.values().filter(|t| t.org_id == Some(org)).map(|t| t.id).collect();
        s.team_members.retain(|(t, u), _| *u != user || !teams.contains(t));
        Ok(())
    }

    async fn org_members(&self, org: i64) -> Result<Vec<Member<O>>, Error> {
        Ok(self.lock().org_members.iter().filter(|((o, _), _)| *o == org).map(|(&(_, user_id), &role)| Member { user_id, role }).collect())
    }

    async fn orgs_of(&self, user: i64) -> Result<Vec<(Org, O)>, Error> {
        let s = self.lock();
        Ok(s.org_members.iter().filter(|((_, u), _)| *u == user).filter_map(|(&(o, _), &role)| Some((s.orgs.get(&o)?.clone(), role))).collect())
    }

    async fn create_team(&self, owner: i64, org: Option<i64>, name: &str) -> Result<Team, Error> {
        let mut s = self.lock();
        if s.name_taken(org, name, None) {
            return Err(Error::NameTaken);
        }
        let team = Team { id: s.id(), org_id: org, name: name.to_owned() };
        s.teams.insert(team.id, team.clone());
        s.team_members.insert((team.id, owner), R::OWNER);
        Ok(team)
    }

    async fn team(&self, id: i64) -> Result<Option<Team>, Error> {
        Ok(self.lock().teams.get(&id).cloned())
    }

    async fn rename_team(&self, id: i64, name: &str) -> Result<(), Error> {
        let mut s = self.lock();
        let Some(org) = s.teams.get(&id).map(|t| t.org_id) else { return Ok(()) };
        if s.name_taken(org, name, Some(id)) {
            return Err(Error::NameTaken);
        }
        if let Some(team) = s.teams.get_mut(&id) {
            name.clone_into(&mut team.name);
        }
        Ok(())
    }

    async fn delete_team(&self, id: i64) -> Result<(), Error> {
        self.lock().drop_team(id);
        Ok(())
    }

    async fn team_role(&self, team: i64, user: i64) -> Result<Option<R>, Error> {
        Ok(self.lock().team_members.get(&(team, user)).copied())
    }

    async fn set_team_role(&self, team: i64, user: i64, role: R) -> Result<(), Error> {
        self.lock().team_members.insert((team, user), role);
        Ok(())
    }

    async fn remove_team_member(&self, team: i64, user: i64) -> Result<(), Error> {
        self.lock().team_members.remove(&(team, user));
        Ok(())
    }

    async fn team_members(&self, team: i64) -> Result<Vec<Member<R>>, Error> {
        Ok(self.lock().team_members.iter().filter(|((t, _), _)| *t == team).map(|(&(_, user_id), &role)| Member { user_id, role }).collect())
    }

    async fn teams_of(&self, user: i64) -> Result<Vec<(Team, R)>, Error> {
        let s = self.lock();
        Ok(s.team_members.iter().filter(|((_, u), _)| *u == user).filter_map(|(&(t, _), &role)| Some((s.teams.get(&t)?.clone(), role))).collect())
    }

    async fn org_teams(&self, org: i64) -> Result<Vec<Team>, Error> {
        Ok(self.lock().teams.values().filter(|t| t.org_id == Some(org)).cloned().collect())
    }

    async fn create_invitation(&self, email: &str, target: Target<R, O>, invited_by: i64, expires_at: i64, token_hash: &str) -> Result<Invitation<R, O>, Error> {
        let mut s = self.lock();
        let invitation = Invitation { id: s.id(), email: email.to_owned(), target, invited_by, expires_at };
        s.invitations.insert(invitation.id, (invitation.clone(), token_hash.to_owned()));
        Ok(invitation)
    }

    async fn invitation(&self, token_hash: &str) -> Result<Option<Invitation<R, O>>, Error> {
        Ok(self.lock().invitations.values().find(|(_, h)| h == token_hash).map(|(i, _)| i.clone()))
    }

    async fn team_invitations(&self, team: i64) -> Result<Vec<Invitation<R, O>>, Error> {
        let s = self.lock();
        Ok(s.invitations.values().filter(|(i, _)| matches!(i.target, Target::Team { team: t, .. } if t == team)).map(|(i, _)| i.clone()).collect())
    }

    async fn org_invitations(&self, org: i64) -> Result<Vec<Invitation<R, O>>, Error> {
        let s = self.lock();
        Ok(s.invitations.values().filter(|(i, _)| matches!(i.target, Target::Org { org: o, .. } if o == org)).map(|(i, _)| i.clone()).collect())
    }

    async fn delete_invitation(&self, id: i64) -> Result<(), Error> {
        self.lock().invitations.remove(&id);
        Ok(())
    }

    async fn current_team_id(&self, user: i64) -> Result<Option<i64>, Error> {
        Ok(self.lock().current.get(&user).copied())
    }

    async fn set_current_team_id(&self, user: i64, team: Option<i64>) -> Result<(), Error> {
        let mut s = self.lock();
        match team {
            Some(team) => s.current.insert(user, team),
            None => s.current.remove(&user),
        };
        Ok(())
    }
}
