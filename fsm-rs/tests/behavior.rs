//! Each machine lives in its own module: generated `State`/`Event` enums are
//! deliberately generic names, so one module per machine.

// ---------------------------------------------------------------------------
// Sync machine with guards, actions, entry hook and on_transition.
// ---------------------------------------------------------------------------

mod breaker {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub failures: u32,
        pub log: Vec<String>,
    }

    state_machine! {
        name: Breaker,
        context: Ctx,

        states: { *Closed, Open(entry: on_open), HalfOpen },
        events: { Success, Fail, Tick },

        transitions: {
            Closed + Fail [should_trip] / record => Open,
            Closed + Fail / record => _,
            Open + Tick => HalfOpen,
            Open + Fail / record => _,
            HalfOpen + Success / reset => Closed,
            HalfOpen + Fail => Open,
            _ + Success => _,
            Closed | HalfOpen + Tick => _,
        }

        on_transition: note,
    }

    impl BreakerContext for Ctx {
        fn should_trip(&self) -> bool {
            self.failures >= 1
        }
        fn record(&mut self) {
            self.failures += 1;
            self.log.push("record".into());
        }
        fn reset(&mut self) {
            self.failures = 0;
            self.log.push("reset".into());
        }
        fn on_open(&mut self) {
            self.log.push("enter Open".into());
        }
        fn note(&mut self, from: &State, to: &State, _event: &Event) {
            self.log.push(format!("{from:?} -> {to:?}"));
        }
    }

    pub fn machine() -> Breaker {
        Breaker::new(Ctx {
            failures: 0,
            log: Vec::new(),
        })
    }
}

use breaker::{Event as BreakerEvent, State as BreakerState};

#[test]
fn guard_fallthrough_then_trip() {
    let mut m = breaker::machine();
    assert_eq!(m.state(), BreakerState::Closed);

    // Guard fails (failures == 0): second row fires, stay, no hooks.
    m.process_event(BreakerEvent::Fail).unwrap();
    assert_eq!(m.state(), BreakerState::Closed);
    assert_eq!(m.context().log, ["record"]);

    // Guard passes now: transition to Open with action, entry hook and
    // on_transition.
    m.process_event(BreakerEvent::Fail).unwrap();
    assert_eq!(m.state(), BreakerState::Open);
    assert_eq!(
        m.context().log,
        ["record", "record", "enter Open", "Closed -> Open"]
    );
}

#[test]
fn full_cycle_and_goto_self_fires_hooks() {
    let mut m = breaker::machine();
    m.process_event(BreakerEvent::Fail).unwrap();
    m.process_event(BreakerEvent::Fail).unwrap();
    assert_eq!(m.state(), BreakerState::Open);

    m.process_event(BreakerEvent::Tick).unwrap();
    assert_eq!(m.state(), BreakerState::HalfOpen);

    // HalfOpen + Fail => Open: entry hook fires again (goto, not stay).
    m.process_event(BreakerEvent::Fail).unwrap();
    assert_eq!(m.state(), BreakerState::Open);
    assert!(m.context().log.iter().any(|l| l == "HalfOpen -> Open"));

    // Back to Closed via HalfOpen + Success.
    m.process_event(BreakerEvent::Tick).unwrap();
    m.process_event(BreakerEvent::Success).unwrap();
    assert_eq!(m.state(), BreakerState::Closed);
    assert_eq!(m.context().failures, 0);
}

#[test]
fn stay_does_not_fire_hooks() {
    let mut m = breaker::machine();
    m.process_event(BreakerEvent::Success).unwrap(); // `_ + Success => _`
    assert_eq!(m.state(), BreakerState::Closed);
    assert!(m.context().log.is_empty());
}

// ---------------------------------------------------------------------------
// Unhandled policy: method receives anything without a matching row.
// ---------------------------------------------------------------------------

mod door {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub rejected: Vec<(State, Event)>,
    }

    state_machine! {
        name: Door,
        context: Ctx,

        states: { *Locked, Unlocked },
        events: { Coin, Push, Kick },

        transitions: {
            Locked + Coin => Unlocked,
            Unlocked + Push => Locked,
        }

        unhandled: on_unhandled,
    }

    impl DoorContext for Ctx {
        fn on_unhandled(&mut self, state: &State, event: &Event) {
            self.rejected.push((*state, *event));
        }
    }
}

#[test]
fn unhandled_method_receives_unmatched_events() {
    use door::{Event, State};

    let mut m = door::Door::new(door::Ctx {
        rejected: Vec::new(),
    });
    m.process_event(Event::Kick).unwrap();
    m.process_event(Event::Push).unwrap(); // only valid when Unlocked
    assert_eq!(m.state(), State::Locked);
    assert_eq!(
        m.context().rejected,
        [(State::Locked, Event::Kick), (State::Locked, Event::Push)]
    );

    m.process_event(Event::Coin).unwrap();
    assert_eq!(m.state(), State::Unlocked);
}

// ---------------------------------------------------------------------------
// Guard failure with no fallback row and no policy -> runtime error.
// ---------------------------------------------------------------------------

mod strict {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub allowed: bool,
    }

    state_machine! {
        name: Strict,
        context: Ctx,

        states: { *A, B },
        events: { Go, Stop },

        transitions: {
            A + Go [allowed] => B,
            B + Stop => A,
            A + Stop => _,
            B + Go => _,
        }
    }

    impl StrictContext for Ctx {
        fn allowed(&self) -> bool {
            self.allowed
        }
    }
}

#[test]
fn guard_failure_without_fallback_is_runtime_error() {
    use strict::{Event, State};

    let mut m = strict::Strict::new(strict::Ctx { allowed: false });
    let err = m.process_event(Event::Go).unwrap_err();
    assert_eq!(
        err,
        fsm_rs::TransitionError::Unhandled {
            state: State::A,
            event: Event::Go,
        }
    );
    assert_eq!(m.state(), State::A);

    m.context_mut().allowed = true;
    m.process_event(Event::Go).unwrap();
    assert_eq!(m.state(), State::B);

    m.process_event(Event::Stop).unwrap();
    assert_eq!(m.state(), State::A);
}

// ---------------------------------------------------------------------------
// Async machine: async guard, action and exit hook.
// ---------------------------------------------------------------------------

mod async_machine {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub can_start: bool,
        pub log: Vec<&'static str>,
    }

    state_machine! {
        name: AsyncMachine,
        context: Ctx,

        states: { *Idle, Working(exit: async cleanup) },
        events: { Start, Finish, Ping },

        transitions: {
            Idle + Start [async can_start_guard] / async begin => Working,
            Idle + Start => _,
            Working + Finish => Idle,
            _ + Ping => _,
            Working + Start => _,
            Idle + Finish => _,
        }
    }

    #[fsm_rs::async_trait]
    impl AsyncMachineContext for Ctx {
        async fn can_start_guard(&self) -> bool {
            self.can_start
        }
        async fn begin(&mut self) {
            self.log.push("begin");
        }
        async fn cleanup(&mut self) {
            self.log.push("cleanup");
        }
    }
}

#[tokio::test]
async fn async_machine() {
    use async_machine::{Event, State};

    let mut m = async_machine::AsyncMachine::new(async_machine::Ctx {
        can_start: true,
        log: Vec::new(),
    });

    m.process_event(Event::Ping).await.unwrap();
    assert_eq!(m.state(), State::Idle); // wildcard stay

    m.process_event(Event::Start).await.unwrap();
    assert_eq!(m.state(), State::Working);
    assert_eq!(m.context().log, ["begin"]);

    m.process_event(Event::Finish).await.unwrap();
    assert_eq!(m.state(), State::Idle);
    assert_eq!(m.context().log, ["begin", "cleanup"]);
}

#[tokio::test]
async fn async_guard_fallthrough() {
    use async_machine::{Event, State};

    let mut m = async_machine::AsyncMachine::new(async_machine::Ctx {
        can_start: false,
        log: Vec::new(),
    });
    m.process_event(Event::Start).await.unwrap();
    assert_eq!(m.state(), State::Idle); // second row: stay
    assert!(m.context().log.is_empty());
}

// ---------------------------------------------------------------------------
// Mixed machine: sync and async callbacks in one machine.
// ---------------------------------------------------------------------------

mod mixed {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub log: Vec<&'static str>,
    }

    state_machine! {
        name: Mixed,
        context: Ctx,

        states: { *A, B(exit: sync_exit) },
        events: { Go, Back },

        transitions: {
            A + Go [sync_guard] / async act => B,
            B + Back => A,
            A + Back => _,
            B + Go => _,
        }

        on_transition: sync_note,
    }

    #[fsm_rs::async_trait]
    impl MixedContext for Ctx {
        fn sync_guard(&self) -> bool {
            true
        }
        async fn act(&mut self) {
            self.log.push("act");
        }
        fn sync_exit(&mut self) {
            self.log.push("exit");
        }
        fn sync_note(&mut self, _from: &State, _to: &State, _event: &Event) {
            self.log.push("note");
        }
    }
}

#[tokio::test]
async fn mixed_sync_and_async_callbacks() {
    use mixed::{Event, State};

    let mut m = mixed::Mixed::new(mixed::Ctx { log: Vec::new() });
    m.process_event(Event::Go).await.unwrap();
    assert_eq!(m.state(), State::B);
    assert_eq!(m.context().log, ["act", "note"]);

    m.process_event(Event::Back).await.unwrap();
    assert_eq!(m.state(), State::A);
    assert_eq!(m.context().log, ["act", "note", "exit", "note"]);
}
