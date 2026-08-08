//! Persisting a machine: serialize to JSON (as if to a database), restore
//! later, and keep processing events.
//!
//! Note that `gateway` is a runtime-only field marked `#[serde(skip)]`: not
//! every piece of context data has to be serializable. Skipped fields come
//! back as `Default::default()` after a restore.
//!
//! Run with: `cargo run --example purchase_persist --features serde`

use fsm_rs::state_machine;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Order {
    id: u64,
    amount_cents: u64,
    /// Runtime-only payment gateway handle; never stored.
    #[serde(skip)]
    gateway: Option<String>,
}

state_machine! {
    name: Purchase,
    context: Order,
    serde: true,

    states: { *Bought, Refunded },
    events: { Refund, Review },

    transitions: {
        Bought + Refund => Refunded,
        Refunded + Refund => _,
        _ + Review => _,
    }

    on_transition: audit,
}

impl PurchaseContext for Order {
    fn audit(&mut self, from: &State, to: &State, event: &Event) {
        println!("[audit] order {}: {event:?}: {from:?} -> {to:?}", self.id);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A purchase is made and stored.
    let purchase = Purchase::new(Order {
        id: 42,
        amount_cents: 19_99,
        gateway: Some("stripe".into()),
    });
    let row = serde_json::to_string_pretty(&purchase)?;
    println!("stored:\n{row}");

    // Later, in another process, a Refund event arrives: restore and apply.
    let mut restored: Purchase = serde_json::from_str(&row)?;
    assert_eq!(restored.state(), State::Bought);
    // The skipped runtime field came back as `Default`:
    println!("gateway after restore: {:?}", restored.context().gateway);
    restored.process_event(Event::Refund)?;

    let row = serde_json::to_string_pretty(&restored)?;
    println!("stored again:\n{row}");
    Ok(())
}
