//! Published by `art publish teams --with-ui`. App code: change it freely.

use auth::Authed;
use route::{Cx, Path, Redirect};
use serde::Serialize;
use ts_rs::TS;
use teams::Error;

use crate::{access, state::App, users::User};

/// Props for `resources/js/pages/invitations/show.vue`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct InvitationPage {
    email: String,
    problem: &'static str,
}

/// The link in the invitation: join, then go to the team or organization.
/// A link that cannot be used shows why.
pub async fn accept(cx: Cx<App>, user: Authed<User>, Path(token): Path<String>) -> Result<Redirect, inertia::Page<InvitationPage>> {
    let problem = match access::teams(&cx.db).accept(&token, user.id, &user.email).await {
        Ok(target) => match access::joined_path(&cx.db, target).await {
            Ok(path) => return Ok(Redirect::to(path)),
            Err(_) => "This team or organization no longer exists.",
        },
        Err(Error::NotFound) => "This invitation was already used or withdrawn.",
        Err(Error::InvitationExpired) => "This invitation has expired. Ask for a new one.",
        Err(Error::EmailMismatch) => "This invitation was sent to another email address. Log in with that one.",
        Err(Error::NotOrgMember) => "Join the organization first; ask for an invitation to it.",
        Err(e) => {
            tracing::error!("accepting an invitation: {e}");
            "Something went wrong. Try again later."
        }
    };
    Err(inertia::render("invitations/show", InvitationPage { email: user.email.clone(), problem }))
}
