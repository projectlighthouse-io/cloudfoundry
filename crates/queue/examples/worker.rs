//! `cargo run -p queue --example worker`: push jobs to an in-memory queue,
//! run a worker until they are done, then stop it gracefully.

use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use queue::{Job, JobError, MemoryStore, Queue, Worker};
use serde::{Deserialize, Serialize};

struct App {
    sent: AtomicU32,
}

#[derive(Serialize, Deserialize)]
struct SendEmail {
    to: String,
}

impl Job for SendEmail {
    const NAME: &'static str = "send_email";
    type Context = &'static App;

    async fn run(self, app: &&'static App) -> Result<(), JobError> {
        if self.to.is_empty() {
            return Err("no recipient".into()); // retried, then moved to failed jobs
        }
        app.sent.fetch_add(1, Ordering::SeqCst);
        println!("sent to {}", self.to);
        Ok(())
    }

    fn backoff(_: u32) -> Duration {
        Duration::from_millis(10)
    }
}

#[tokio::main]
async fn main() {
    static APP: App = App { sent: AtomicU32::new(0) };
    let queue = Queue::new(MemoryStore::new());
    for to in ["ada@example.com", "grace@example.com", ""] {
        queue.push(SendEmail { to: to.into() }).await.expect("push");
    }

    let watch = queue.clone();
    let done = async move {
        while watch.store().pending() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    Worker::new(queue.clone(), &APP).register::<SendEmail>().poll(Duration::from_millis(20)).run(done).await;

    println!("sent: {}, failed: {:?}", APP.sent.load(Ordering::SeqCst), queue.store().failed());
}
