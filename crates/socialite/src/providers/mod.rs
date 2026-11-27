//! One file per provider. Each implements [`crate::Provider`].

mod github;

pub use github::{GitHub, GitHubUser};
