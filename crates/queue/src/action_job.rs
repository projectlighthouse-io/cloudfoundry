//! An `Action` as a job: its input is the payload, the worker builds the
//! action from its context.

use std::{sync::Arc, time::Duration};

use action::Action;
use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, JobError, JobId, NewJob, Queue, Store, Worker};

/// An action that can run on the queue. `NAME` is stored with the payload:
/// keep it stable.
pub trait ActionJob: Action + Send + Sync + 'static
where
    Self::Input: Serialize + DeserializeOwned + Send + 'static,
    Self::Output: Outcome,
{
    const NAME: &'static str;
    const QUEUE: &'static str = "default";
    const MAX_ATTEMPTS: u32 = 3;

    fn backoff(attempt: u32) -> Duration {
        crate::default_backoff(attempt)
    }
}

/// An action output a job can report: `Result<T, E>` with `E` an error.
pub trait Outcome {
    fn into_result(self) -> Result<(), JobError>;
}

impl<T, E: Into<JobError>> Outcome for Result<T, E> {
    fn into_result(self) -> Result<(), JobError> {
        self.map(drop).map_err(Into::into)
    }
}

impl<St: Store> Queue<St> {
    /// Queue `A` to run with `input`.
    pub async fn push_action<A>(&self, input: A::Input) -> Result<JobId, Error>
    where
        A: ActionJob,
        A::Input: Serialize + DeserializeOwned + Send + 'static,
        A::Output: Outcome,
    {
        self.store()
            .push(NewJob {
                queue: A::QUEUE.to_owned(),
                name: A::NAME.to_owned(),
                payload: serde_json::to_string(&input)?,
                max_attempts: A::MAX_ATTEMPTS,
                delay: Duration::ZERO,
            })
            .await
    }
}

impl<St: Store, C: Send + Sync + 'static> Worker<St, C> {
    /// Run queued `A`s. `build` makes the action from the worker's context,
    /// the same way the app builds it at startup.
    pub fn register_action<A>(self, build: impl Fn(&C) -> A + Send + Sync + 'static) -> Self
    where
        A: ActionJob,
        A::Input: Serialize + DeserializeOwned + Send + 'static,
        A::Output: Outcome,
    {
        let build = Arc::new(build);
        self.handle(A::NAME, A::backoff, move |cx: Arc<C>, input: A::Input| {
            let action = build(&cx);
            async move { action.run(input).await.into_result() }
        })
    }
}
