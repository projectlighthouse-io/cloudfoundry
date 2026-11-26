//! Dispatch typed events to listeners, in process.
//!
//! ```ignore
//! #[derive(Clone, Serialize, Deserialize)]
//! struct RunFinished { run_id: i64 }
//!
//! async fn update_commit_status(app: App, e: RunFinished) -> Result<(), event::Error> { ... }
//!
//! let events = Events::builder()
//!     .queue(&queue)                                                     // feature `queue`
//!     .on(update_commit_status)                        // inline, in order
//!     .on_queued("notify_owner", send_notification)  // becomes a job
//!     .build();
//!
//! events.emit(&app, RunFinished { run_id }).await?;
//! ```
//!
//! Listeners take the context by value (`App` is cheap to clone) and the event
//! by value. They are looked up by event type: no string event names.

#[cfg(feature = "command")]
pub mod command;

use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Inline<C> = Arc<dyn Fn(C, &(dyn Any + Send + Sync)) -> BoxFuture<Result<(), Error>> + Send + Sync>;

enum Kind<C> {
    Inline(Inline<C>),
    #[cfg(feature = "queue")]
    Queued(queued::Queued<C>),
}

struct Listener<C> {
    name: String,
    kind: Kind<C>,
}

/// One row of `event:list`.
#[derive(Clone, Debug)]
pub struct Row {
    pub event: &'static str,
    pub listener: String,
    pub queued: bool,
}

pub struct EventsBuilder<C> {
    listeners: HashMap<TypeId, (&'static str, Vec<Listener<C>>)>,
    #[cfg(feature = "queue")]
    push: Option<queued::Push>,
}

/// The frozen bus. Share it behind an `Arc`.
pub struct Events<C> {
    listeners: HashMap<TypeId, (&'static str, Vec<Listener<C>>)>,
    #[cfg(feature = "queue")]
    push: Option<queued::Push>,
}

impl<C: Clone + Send + Sync + 'static> Events<C> {
    pub fn builder() -> EventsBuilder<C> {
        EventsBuilder {
            listeners: HashMap::new(),
            #[cfg(feature = "queue")]
            push: None,
        }
    }

    /// Run inline listeners in registration order; enqueue queued ones.
    /// Stops at and returns the first error, like a function call.
    pub async fn emit<E: Clone + Send + Sync + 'static>(&self, cx: &C, event: E) -> Result<(), Error> {
        let Some((_, listeners)) = self.listeners.get(&TypeId::of::<E>()) else { return Ok(()) };
        for l in listeners {
            match &l.kind {
                Kind::Inline(f) => f(cx.clone(), &event).await?,
                #[cfg(feature = "queue")]
                Kind::Queued(q) => {
                    let push = self.push.as_ref().expect("checked at build");
                    push((q.new_job)(&event)?).await?;
                }
            }
        }
        Ok(())
    }

    /// Every event and its listeners, for `event:list`.
    pub fn table(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .listeners
            .values()
            .flat_map(|(event, ls)| {
                ls.iter().map(|l| Row { event, listener: l.name.clone(), queued: !matches!(l.kind, Kind::Inline(_)) })
            })
            .collect();
        rows.sort_by(|a, b| a.event.cmp(b.event));
        rows
    }
}

impl<C: Clone + Send + Sync + 'static> EventsBuilder<C> {
    /// An inline listener: runs inside `emit`, in registration order.
    pub fn on<E, F, Fut>(self, f: F) -> Self
    where
        E: Clone + Send + Sync + 'static,
        F: Fn(C, E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let inline: Inline<C> = Arc::new(move |cx, event| {
            let event = event.downcast_ref::<E>().expect("listeners are keyed by event type").clone();
            Box::pin(f(cx, event))
        });
        self.add::<E>(Listener { name: short(type_name::<F>()).to_owned(), kind: Kind::Inline(inline) })
    }

    fn add<E: 'static>(mut self, l: Listener<C>) -> Self {
        let entry = self.listeners.entry(TypeId::of::<E>()).or_insert_with(|| (short(type_name::<E>()), vec![]));
        entry.1.push(l);
        self
    }

    /// Freeze the bus. Panics if a queued listener has no queue: a startup bug.
    pub fn build(self) -> Events<C> {
        #[cfg(feature = "queue")]
        {
            let queued = self.listeners.values().flat_map(|(_, ls)| ls).any(|l| matches!(l.kind, Kind::Queued(_)));
            assert!(!queued || self.push.is_some(), "events: `on_queued` needs `.queue(&queue)` on the builder");
        }
        Events {
            listeners: self.listeners,
            #[cfg(feature = "queue")]
            push: self.push,
        }
    }
}

/// `crate::module::Type<..>` -> `Type`; a fn item keeps its name.
fn short(full: &'static str) -> &'static str {
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}

#[cfg(feature = "queue")]
mod queued {
    use std::{any::Any, future::Future, sync::Arc, time::Duration};

    use queue::{JobId, NewJob, Queue, Store, Worker};
    use serde::{Serialize, de::DeserializeOwned};

    use super::*;

    pub(super) type Push = Arc<dyn Fn(NewJob) -> BoxFuture<Result<JobId, Error>> + Send + Sync>;
    type Run<C> = Arc<dyn Fn(Arc<C>, String) -> BoxFuture<Result<(), Error>> + Send + Sync>;

    pub(super) struct Queued<C> {
        pub(super) job_name: String,
        pub(super) new_job: Box<dyn Fn(&(dyn Any + Send + Sync)) -> Result<NewJob, Error> + Send + Sync>,
        pub(super) run: Run<C>,
    }

    impl<C: Clone + Send + Sync + 'static> EventsBuilder<C> {
        /// Where queued listeners' jobs go.
        pub fn queue<St: Store>(mut self, q: &Queue<St>) -> Self {
            let q = q.clone();
            self.push = Some(Arc::new(move |job| {
                let q = q.clone();
                Box::pin(async move { q.store().push(job).await })
            }));
            self
        }

        /// A listener that runs as a job (`event:<name>`). `emit` only enqueues;
        /// retries are the queue's. `name` must stay stable, like `Job::NAME`.
        pub fn on_queued<E, F, Fut>(self, name: &'static str, f: F) -> Self
        where
            E: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
            F: Fn(C, E) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<(), Error>> + Send + 'static,
        {
            let job_name = format!("event:{name}");
            let jn = job_name.clone();
            let new_job = Box::new(move |event: &(dyn Any + Send + Sync)| -> Result<NewJob, Error> {
                let event = event.downcast_ref::<E>().expect("listeners are keyed by event type");
                Ok(NewJob { queue: "default".into(), name: jn.clone(), payload: serde_json::to_string(event)?, max_attempts: 3, delay: Duration::ZERO })
            });
            let f = Arc::new(f);
            let run: Run<C> = Arc::new(move |cx: Arc<C>, payload: String| {
                let f = f.clone();
                Box::pin(async move {
                    let event: E = serde_json::from_str(&payload)?;
                    f((*cx).clone(), event).await
                })
            });
            self.add::<E>(Listener { name: name.to_owned(), kind: Kind::Queued(Queued { job_name, new_job, run }) })
        }
    }

    impl<C: Clone + Send + Sync + 'static> Events<C> {
        /// Teach a worker to run this bus's queued listeners.
        pub fn register_jobs<St: Store>(&self, mut w: Worker<St, C>) -> Worker<St, C> {
            for (_, ls) in self.listeners.values() {
                for l in ls {
                    if let Kind::Queued(q) = &l.kind {
                        let run = q.run.clone();
                        w = w.handle_raw(&q.job_name, queue::default_backoff, move |cx, payload| run(cx, payload));
                    }
                }
            }
            w
        }
    }
}
