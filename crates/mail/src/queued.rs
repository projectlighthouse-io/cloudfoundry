//! Send mail from a worker (feature `queue`).

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::{Mailer, Message};

/// App state that has a mailer, for [`SendMail`] to send with.
pub trait HasMailer: Send + Sync + 'static {
    fn mailer(&self) -> &dyn Mailer;
}

/// A queued mail. Register it on the worker (`.register::<mail::SendMail<App>>()`)
/// and push it with [`send_later`]. Failed sends retry with the queue's backoff
/// (3 attempts), then land in failed jobs.
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub struct SendMail<C> {
    pub message: Message,
    #[serde(skip)]
    cx: PhantomData<fn() -> C>,
}

impl<C> SendMail<C> {
    pub fn new(message: Message) -> Self {
        Self { message, cx: PhantomData }
    }
}

impl<C: HasMailer> queue::Job for SendMail<C> {
    const NAME: &'static str = "mail:send";
    type Context = C;

    async fn run(self, cx: &C) -> Result<(), queue::JobError> {
        cx.mailer().send(self.message).await.map_err(Into::into)
    }
}

/// Queue `message` for a worker to send. Returns once it is stored.
pub async fn send_later<C: HasMailer, St: queue::Store>(queue: &queue::Queue<St>, message: Message) -> Result<queue::JobId, queue::Error> {
    queue.push(SendMail::<C>::new(message)).await
}
