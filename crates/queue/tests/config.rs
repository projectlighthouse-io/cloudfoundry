use std::time::Duration;

use queue::WorkerConfig;

#[test]
fn check_rejects_broken_settings() {
    let d = WorkerConfig::default;
    assert!(d().check().is_ok());
    assert!(WorkerConfig { queues: vec![], ..d() }.check().is_err());
    assert!(WorkerConfig { queues: vec![String::new()], ..d() }.check().is_err());
    assert!(WorkerConfig { concurrency: 0, ..d() }.check().is_err());
    assert!(WorkerConfig { visibility: Duration::from_millis(999), ..d() }.check().is_err());
    assert!(WorkerConfig { poll: Duration::ZERO, ..d() }.check().is_err());
}

#[test]
fn worker_config_returns_the_error() {
    let worker = queue::Worker::new(queue::Queue::new(queue::MemoryStore::default()), ());
    assert!(worker.config(WorkerConfig { concurrency: 0, ..WorkerConfig::default() }).is_err());
}

#[cfg(feature = "configured")]
mod serde_tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct App {
        #[serde(default)]
        queue: WorkerConfig,
    }

    #[test]
    fn partial_keeps_defaults() {
        let app: App = toml::from_str("[queue]\nqueues = [\"high\", \"default\"]\nvisibility = \"10m\"\n").unwrap();
        let want = WorkerConfig { queues: vec!["high".into(), "default".into()], visibility: Duration::from_secs(600), ..WorkerConfig::default() };
        assert_eq!(app.queue, want);
        assert_eq!(toml::from_str::<App>("").unwrap().queue, WorkerConfig::default());
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(toml::from_str::<App>("[queue]\nconcurency = 2\n").is_err());
    }
}
