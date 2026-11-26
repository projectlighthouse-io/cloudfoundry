#![cfg(feature = "action")]

use std::sync::{Arc, Mutex};

use action::Action;
use event::Events;

#[derive(Clone)]
struct RunFinished(u32);

struct Notify {
    sent: Arc<Mutex<Vec<u32>>>,
}

struct NotifyInput(u32);
impl From<RunFinished> for NotifyInput {
    fn from(e: RunFinished) -> Self {
        NotifyInput(e.0)
    }
}

impl Action for Notify {
    type Input = NotifyInput;
    type Output = Result<(), std::io::Error>;

    async fn run(&self, input: NotifyInput) -> Result<(), std::io::Error> {
        self.sent.lock().unwrap().push(input.0);
        Ok(())
    }
}

#[tokio::test]
async fn action_as_listener() {
    let sent = Arc::new(Mutex::new(vec![]));
    let bus = Events::<()>::builder().on_action::<RunFinished, _>(Notify { sent: sent.clone() }).build();
    bus.emit(&(), RunFinished(3)).await.unwrap();
    assert_eq!(*sent.lock().unwrap(), [3]);
    assert_eq!(bus.table()[0].listener, "Notify");
}
