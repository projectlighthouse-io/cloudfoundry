#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test helpers
#![cfg_attr(not(feature = "queue"), allow(unused_imports, dead_code, clippy::unused_async))] // queued-listener helpers
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use event::Events;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct RunFinished(u32);

#[derive(Clone)]
struct App {
    log: Arc<Mutex<Vec<String>>>,
}

async fn first(app: App, e: RunFinished) -> Result<(), event::Error> {
    app.log.lock().unwrap().push(format!("first {}", e.0));
    Ok(())
}
async fn failing(_: App, _: RunFinished) -> Result<(), event::Error> {
    Err("stop".into())
}
async fn never(app: App, _: RunFinished) -> Result<(), event::Error> {
    app.log.lock().unwrap().push("never".into());
    Ok(())
}
async fn notify(app: App, e: RunFinished) -> Result<(), event::Error> {
    app.log.lock().unwrap().push(format!("queued {}", e.0));
    Ok(())
}

#[tokio::test]
async fn inline_in_order_and_stop_at_first_error() {
    let app = App { log: Default::default() };
    let bus = Events::builder().on(first).on(failing).on(never).build();
    let err = bus.emit(&app, RunFinished(1)).await.unwrap_err();
    assert_eq!(err.to_string(), "stop");
    assert_eq!(*app.log.lock().unwrap(), ["first 1"]);
    assert!(bus.emit(&app, 5u8).await.is_ok(), "an event nobody listens to is fine");
}

#[cfg(feature = "queue")]
#[tokio::test]
async fn queued_listener_runs_as_a_job() {
    let app = App { log: Default::default() };
    let q = queue::Queue::new(queue::MemoryStore::new());
    let bus = Events::builder().queue(&q).on(first).on_queued("notify", notify).build();

    bus.emit(&app, RunFinished(7)).await.unwrap();
    assert_eq!(*app.log.lock().unwrap(), ["first 7"], "queued listener did not run inline");
    assert_eq!(q.store().pending(), 1);

    let table = bus.table();
    assert_eq!(table.len(), 2);
    assert!(table.iter().any(|r| r.event == "RunFinished" && r.listener == "notify" && r.queued));
    assert!(table.iter().any(|r| r.listener == "first" && !r.queued));

    let store = q.clone();
    let worker = bus.register_jobs(queue::Worker::new(q.clone(), app.clone()).poll(Duration::from_millis(10)));
    worker
        .run(async move {
            while store.store().pending() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
    assert_eq!(*app.log.lock().unwrap(), ["first 7", "queued 7"]);
}
