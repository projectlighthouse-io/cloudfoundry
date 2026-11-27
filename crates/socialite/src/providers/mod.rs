//! One file per provider. Each implements [`crate::Provider`].

mod github;
mod google;

pub use github::{GitHub, GitHubUser};
pub use google::{Google, GoogleUser};
