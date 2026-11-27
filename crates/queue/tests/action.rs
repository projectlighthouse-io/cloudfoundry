#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
#![cfg(feature = "action")]

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering::SeqCst},
    },
    time::Duration,
};

use action::Action;
use queue::{ActionJob, MemoryStore, Queue, Worker};
use serde::{Deserialize, Serialize};

struct App {
    total: Arc<AtomicU32>,
}

struct AddUp {
    total: Arc<AtomicU32>,
}

#[derive(Serialize, Deserialize)]
struct AddInput(u32);

impl Action for AddUp {
    type Input = AddInput;
    type Output = Result<u32, std::io::Error>;

    async fn run(&self, AddInput(n): AddInput) -> Result<u32, std::io::Error> {
        Ok(self.total.fetch_add(n, SeqCst) + n)
    }
}

impl ActionJob for AddUp {
    const NAME: &'static str = "add_up";
}

#[tokio::test(flavor = "multi_thread")]
async fn action_runs_as_a_job() {
    let total = Arc::new(AtomicU32::new(0));
    let q = Queue::new(MemoryStore::new());
    q.push_action::<AddUp>(AddInput(2)).await.unwrap();
    q.push_action::<AddUp>(AddInput(3)).await.unwrap();

    let store = q.clone();
    Worker::new(q.clone(), App { total: total.clone() })
        .poll(Duration::from_millis(10))
        .register_action(|app: &App| AddUp { total: app.total.clone() })
        .run(async move {
            while store.store().pending() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
    assert_eq!(total.load(SeqCst), 5);
}
