//! Persistence tests: serialize a machine, restore it, keep processing.
#![cfg(feature = "serde")]

use fsm_rs::state_machine;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Order {
    id: u64,
    refunds: u32,
    /// Pretend runtime-only handle: skipped, restored as `Default`.
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
        Bought + Refund / mark_refunded => Refunded,
        Refunded + Refund => _,
        _ + Review => _,
    }

    on_transition: audit,
}

impl PurchaseContext for Order {
    fn mark_refunded(&mut self) {
        self.refunds += 1;
    }
    fn audit(&mut self, from: &State, to: &State, _event: &Event) {
        // Would append to an audit log; here just record the count.
        let _ = (from, to);
    }
}

#[test]
fn serialize_restore_and_continue() {
    let purchase = Purchase::new(Order {
        id: 7,
        refunds: 0,
        gateway: Some("stripe".into()),
    });

    // "Database row": plain JSON. Skipped fields are not persisted.
    let stored = serde_json::to_value(&purchase).unwrap();
    assert_eq!(
        stored,
        serde_json::json!({"state": "Bought", "context": {"id": 7, "refunds": 0}})
    );

    // Later, a Refund arrives: restore the machine and apply the event.
    let mut restored: Purchase = serde_json::from_value(stored).unwrap();
    assert_eq!(restored.state(), State::Bought);
    assert_eq!(restored.context().id, 7);
    assert_eq!(restored.context().gateway, None); // skipped -> Default

    restored.process_event(Event::Refund).unwrap();
    assert_eq!(restored.state(), State::Refunded);
    assert_eq!(restored.context().refunds, 1);

    let stored_again = serde_json::to_value(&restored).unwrap();
    assert_eq!(stored_again["state"], serde_json::json!("Refunded"));
}
