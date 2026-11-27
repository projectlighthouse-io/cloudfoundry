//! Published by `art publish socialite --with-ui`. App code: change it freely.
//!
//! The callback for every provider registered with `socialite::routes`.
//! `/auth/{provider}` redirects to the provider; it sends the user back here.

use std::collections::BTreeMap;

use auth::Auth;
use route::{Cx, Redirect};
use session::Session;
use socialite::SocialCallback;

use crate::{social::SocialError, state::App, users::{User, Users}};

pub async fn callback(cx: Cx<App>, (auth, session, social): (Auth<User>, Session, SocialCallback)) -> Redirect {
    // The login page shows this under its email field (the Inertia error bag).
    let failed = |message: &str| {
        let _ = session.flash("errors", &BTreeMap::from([("email", message)]));
        Redirect::to_route("login")
    };
    let social = match social.0 {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("social login: {e}");
            return failed("That sign-in did not complete. Please try again.");
        }
    };
    let users = Users { db: cx.db.clone() };
    match users.find_or_create_social(&social, auth.user()).await {
        Ok(user) => {
            auth.login(&user);
            Redirect::to_route("home")
        }
        Err(SocialError::NoEmail) => failed("That provider does not share an email address. Log in first, then connect it."),
        Err(SocialError::Db(e)) => {
            tracing::error!("social login: {e}");
            failed("Something went wrong. Please try again.")
        }
    }
}
