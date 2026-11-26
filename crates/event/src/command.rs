//! `event:list` for the `command` crate.

use std::convert::Infallible;

use crate::{Events, Row};

#[derive(clap::Args)]
pub struct ListArgs {}

/// `event:list`: every event and its listeners, inline or queued.
pub fn list<C: Clone + Send + Sync + 'static>(events: &Events<C>) -> impl AsyncFnOnce(&C, ListArgs) -> Result<(), Infallible> + use<C> {
    let rows = events.table();
    async move |_: &C, _: ListArgs| {
        print!("{}", format(&rows));
        Ok(())
    }
}

fn format(rows: &[Row]) -> String {
    let w = rows.iter().map(|r| r.event.len()).max().unwrap_or(0).max(5);
    let mut out = format!("{:w$}  LISTENER\n", "EVENT");
    for r in rows {
        let kind = if r.queued { " (queued)" } else { "" };
        out.push_str(&format!("{:w$}  {}{kind}\n", r.event, r.listener));
    }
    out
}
