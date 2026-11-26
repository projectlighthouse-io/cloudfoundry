//! `queue:work` and `queue:retry` for the `command` crate.

use crate::{Error, JobId, Queue, Store, Worker, shutdown_signal};

#[derive(clap::Args)]
pub struct WorkArgs {
    /// Queues to take jobs from, in order. Default: the worker's setting.
    #[arg(long = "queue", value_delimiter = ',')]
    pub queues: Vec<String>,
    /// Jobs to run at once.
    #[arg(long)]
    pub concurrency: Option<usize>,
}

/// `queue:work`: run a worker until Ctrl-C or SIGTERM. `setup` registers jobs.
pub fn work<C, St, F>(queue: Queue<St>, setup: F) -> impl AsyncFnOnce(&C, WorkArgs) -> Result<(), Error> + use<C, St, F>
where
    C: Clone + Send + Sync + 'static,
    St: Store,
    F: FnOnce(Worker<St, C>) -> Worker<St, C> + 'static,
{
    async move |app: &C, args: WorkArgs| {
        let mut worker = setup(Worker::new(queue, app.clone()));
        if !args.queues.is_empty() {
            worker = worker.queues(args.queues);
        }
        if let Some(n) = args.concurrency {
            worker = worker.concurrency(n);
        }
        worker.run(shutdown_signal()).await;
        Ok(())
    }
}

#[derive(clap::Args)]
pub struct RetryArgs {
    /// Failed job id.
    pub id: i64,
}

/// `queue:retry <id>`: move a failed job back to its queue.
pub fn retry<C: 'static, St: Store>(queue: Queue<St>) -> impl AsyncFnOnce(&C, RetryArgs) -> Result<(), Error> + use<C, St> {
    async move |_: &C, args: RetryArgs| match queue.store().retry_failed(JobId(args.id)).await? {
        Some(id) => {
            println!("failed job {} queued again as {id}", args.id);
            Ok(())
        }
        None => Err(format!("no failed job {}", args.id).into()),
    }
}
