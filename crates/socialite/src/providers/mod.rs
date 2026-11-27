//! One file per provider, each behind its Cargo feature. Each implements
//! [`crate::Provider`].

#[cfg(feature = "auth0")]
mod auth0;
#[cfg(feature = "github")]
mod github;
#[cfg(feature = "gitlab")]
mod gitlab;
#[cfg(feature = "google")]
mod google;
#[cfg(feature = "oidc")]
mod oidc;
#[cfg(feature = "reddit")]
mod reddit;
#[cfg(feature = "twitter")]
mod twitter;

#[cfg(feature = "auth0")]
pub use auth0::Auth0;
#[cfg(feature = "github")]
pub use github::{GitHub, GitHubUser};
#[cfg(feature = "gitlab")]
pub use gitlab::{GitLab, GitLabUser};
#[cfg(feature = "google")]
pub use google::{Google, GoogleUser};
#[cfg(feature = "oidc")]
pub use oidc::{Oidc, OidcUser};
#[cfg(feature = "reddit")]
pub use reddit::{Reddit, RedditUser};
#[cfg(feature = "twitter")]
pub use twitter::{Twitter, TwitterUser};
