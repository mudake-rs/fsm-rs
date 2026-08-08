//! An async machine: guards, actions and hooks can be `async fn`s — here they
//! simulate talking to a remote scheduler with real (tiny) delays.
//!
//! Note the two ingredients:
//!   * `async` prefixes in the DSL — this makes `process_event` async too;
//!   * `#[fsm_rs::async_trait]` on the context trait implementation.
//!
//! Run with: `cargo run --example async_job`

use std::time::Duration;

use fsm_rs::state_machine;
use tokio::time::sleep;

struct Scheduler {
    slot_available: bool,
    running: u32,
}

state_machine! {
    name: Job,
    context: Scheduler,

    states: { *Queued, Running(exit: async on_exit), Done, Failed },
    events: { Start, Succeed, Fail, Reset },

    transitions: {
        Queued  + Start [async check_slot] / async launch => Running,
        Queued  + Start / async reject => Failed,
        Running + Succeed => Done,
        Running + Fail / async record_error => Failed,
        _       + Reset => Queued,
        Queued  + Succeed | Fail => _,
        Running + Start => _,
        Done    + Start | Succeed | Fail => _,
        Failed  + Start | Succeed | Fail => _,
    }

    on_transition: async trace,
}

#[fsm_rs::async_trait]
impl JobContext for Scheduler {
    /// Simulated remote call to the scheduler service.
    async fn check_slot(&self) -> bool {
        sleep(Duration::from_millis(50)).await;
        self.slot_available
    }

    async fn launch(&mut self) {
        sleep(Duration::from_millis(50)).await;
        self.running += 1;
        println!("job launched ({} running)", self.running);
    }

    async fn reject(&mut self) {
        println!("no slot available, job rejected");
    }

    async fn record_error(&mut self) {
        println!("job failure recorded");
    }

    /// Fires on both `Succeed` and `Fail` exits from `Running`.
    async fn on_exit(&mut self) {
        self.running -= 1;
        println!("job left Running ({} running)", self.running);
    }

    async fn trace(&mut self, from: &State, to: &State, event: &Event) {
        println!("{event:?}: {from:?} -> {to:?}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut job = Job::new(Scheduler {
        slot_available: true,
        running: 0,
    });

    println!("--- happy path ---");
    job.process_event(Event::Start).await?; // guard passes -> launch -> Running
    job.process_event(Event::Succeed).await?; // -> Done (exit hook fires)
    assert_eq!(job.state(), State::Done);

    println!("--- no slot available ---");
    job.process_event(Event::Reset).await?; // -> Queued
    job.context_mut().slot_available = false;
    job.process_event(Event::Start).await?; // guard fails -> reject -> Failed
    assert_eq!(job.state(), State::Failed);

    Ok(())
}
