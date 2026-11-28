//! Published by `art publish socialite --with-ui`. App code: change it freely.
//!
//! The callback for every provider registered with `socialite::routes`.
//! `/auth/{provider}` redirects to the provider; it sends the user back here.

use std::collections::BTreeMap;

use auth::Auth;
use route::{Cx, Redirect};
use session::Session;
use socialite::SocialCallback;

use crate::{social::SocialError, state::App, users::{User, UserStore}};

pub async fn callback(cx: Cx<App>, auth: Auth<User>, session: Session, social: SocialCallback) -> Redirect {
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
    let users = UserStore { db: cx.db.clone() };
    match users.find_or_create_social(&social, auth.user()).await {
        Ok(user) => {
            // Already logged in as this user: a fresh provider login proves it is them
            // (sudo mode, for accounts without a password). Back to the page that asked.
            if auth.user().is_some_and(|u| u.id == user.id) {
                if let Err(e) = auth.mark_confirmed() {
                    tracing::error!("social login: confirming: {e}");
                }
                return Redirect::to(auth.intended("/"));
            }
            auth.login(&user);
            Redirect::to_route("home")
        }
        Err(SocialError::NoEmail) => failed("That provider does not share an email address. Log in first, then connect it."),
        Err(SocialError::UnverifiedEmail) => failed("That provider has not verified your email address. Log in first, then connect it."),
        Err(SocialError::Db(e)) => {
            tracing::error!("social login: {e}");
            failed("Something went wrong. Please try again.")
        }
    }
}

/// Props for `resources/js/pages/settings/connected-accounts.vue`.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ConnectedAccountsPage {
    email: String,
    /// Whether the user can sign in with a password, so unlinking the last provider is safe.
    has_password: bool,
    accounts: Vec<crate::social::SocialAccount>,
}

/// Linked providers, with "Connect" buttons for the others (the shared `social` prop).
pub async fn accounts(cx: Cx<App>, user: auth::Authed<User>) -> Result<inertia::Page<ConnectedAccountsPage>, route::http::StatusCode> {
    let users = UserStore { db: cx.db.clone() };
    let accounts = users.social_accounts(user.id).await.map_err(|e| {
        tracing::error!("social accounts: {e}");
        route::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(inertia::render("settings/connected-accounts", ConnectedAccountsPage { email: user.email.clone(), has_password: user.password_hash.is_some(), accounts }))
}

/// Unlink a provider, unless it is the last way to sign in. Needs a recent confirmation.
pub async fn unlink(cx: Cx<App>, user: auth::RecentlyConfirmed<User>, session: Session, route::Path(provider): route::Path<String>) -> Redirect {
    let users = UserStore { db: cx.db.clone() };
    let error = match users.unlink_social(&user, &provider).await {
        Ok(()) => None,
        Err(crate::social::UnlinkError::NotLinked) => Some("That provider is not connected."),
        Err(crate::social::UnlinkError::LastSignIn) => Some("This is your only way to sign in. Set a password first."),
        Err(crate::social::UnlinkError::Db(e)) => {
            tracing::error!("unlink {provider}: {e}");
            Some("Something went wrong. Please try again.")
        }
    };
    if let Some(message) = error {
        let _ = session.flash("errors", &BTreeMap::from([("provider", message)]));
    }
    Redirect::to_route("social.accounts")
}
