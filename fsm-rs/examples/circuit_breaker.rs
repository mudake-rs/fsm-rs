//! A circuit breaker driven by events arriving over a tokio mpsc channel —
//! the minimal "actor mailbox" pattern. The core library is runtime-agnostic;
//! the mailbox loop here is plain user code.
//!
//! Run with: cargo run --example circuit_breaker

use fsm_rs::state_machine;
use tokio::sync::mpsc;

struct Context {
    failures: u32,
}

state_machine! {
    name: CircuitBreaker,
    context: Context,

    states: { *Closed, Open, HalfOpen },
    events: { CallSucceeded, CallFailed, TimerFired },

    transitions: {
        Closed   + CallFailed [should_trip] / record_failure => Open,
        Closed   + CallFailed / record_failure => _,
        Open     + TimerFired => HalfOpen,
        Open     + CallFailed / record_failure => _,
        HalfOpen + CallSucceeded / reset => Closed,
        HalfOpen + CallFailed => Open,
        // Everything else is explicitly ignored in the current state.
        _ + TimerFired => _,
        _ + CallSucceeded => _,
    }

    on_transition: log_transition,
}

impl CircuitBreakerContext for Context {
    fn should_trip(&self) -> bool {
        self.failures >= 1
    }
    fn record_failure(&mut self) {
        self.failures += 1;
    }
    fn reset(&mut self) {
        self.failures = 0;
    }
    fn log_transition(&mut self, from: &State, to: &State, event: &Event) {
        println!("{event:?}: {from:?} -> {to:?}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (tx, mut rx) = mpsc::channel::<Event>(16);

    let driver = tokio::spawn(async move {
        let mut machine = CircuitBreaker::new(Context { failures: 0 });
        while let Some(event) = rx.recv().await {
            if let Err(err) = machine.process_event(event) {
                eprintln!("event rejected: {err}");
            }
        }
        println!("final state: {:?}", machine.state());
    });

    for event in [
        Event::CallFailed,    // failures = 1
        Event::CallFailed,    // trips: Closed -> Open
        Event::TimerFired,    // Open -> HalfOpen
        Event::CallSucceeded, // HalfOpen -> Closed, failures reset
    ] {
        tx.send(event).await.unwrap();
    }
    drop(tx);
    driver.await.unwrap();
}
