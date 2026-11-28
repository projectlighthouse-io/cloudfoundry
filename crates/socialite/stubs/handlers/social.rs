//! Published by `art publish socialite`. App code: change it freely.
//!
//! The callback for every provider registered with `socialite::routes`.
//! `/auth/{provider}` redirects to the provider; it sends the user back here.

use auth::Auth;
use route::{Cx, Redirect};
use socialite::SocialCallback;

use crate::{social::SocialError, state::App, users::{User, UserStore}};

/// Logs the user in and sends them to your frontend (`app_url` in config). On failure the
/// reason is in the `social_error` query parameter.
pub async fn callback(cx: Cx<App>, auth: Auth<User>, social: SocialCallback) -> Redirect {
    let base = &cx.cfg.app_url;
    let social = match social.0 {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("social login: {e}");
            return Redirect::to(format!("{base}/?social_error=declined"));
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
                return Redirect::to(auth.intended(base));
            }
            auth.login(&user);
            Redirect::to(base)
        }
        Err(SocialError::NoEmail) => Redirect::to(format!("{base}/?social_error=no_email")),
        Err(SocialError::UnverifiedEmail) => Redirect::to(format!("{base}/?social_error=unverified_email")),
        Err(SocialError::Db(e)) => {
            tracing::error!("social login: {e}");
            Redirect::to(format!("{base}/?social_error=server"))
        }
    }
}
