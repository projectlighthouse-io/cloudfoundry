//! One trait for a unit of application logic.
//!
//! Write the task once; run it directly, from a handler (`route::action`), a
//! command (`command::action`), and later a job or a listener. Dependencies
//! are fields on the struct, set once at startup.
//!
//! ```
//! use action::Action;
//!
//! struct Double;
//!
//! impl Action for Double {
//!     type Input = u32;
//!     type Output = u32;
//!
//!     async fn run(&self, input: u32) -> u32 {
//!         input * 2
//!     }
//! }
//! ```

use std::future::Future;

pub trait Action {
    type Input;
    /// An action that can fail sets this to a `Result`.
    type Output;

    fn run(&self, input: Self::Input) -> impl Future<Output = Self::Output> + Send;
}
