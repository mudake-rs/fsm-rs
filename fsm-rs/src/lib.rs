//! # fsm-rs
//!
//! An Akka-inspired finite state machine library with a table-style DSL,
//! compile-time exhaustiveness checking and async support.
//!
//! See the [`state_machine!`](macro@state_machine) macro for the DSL reference.
//!
//! ## Quick example
//!
//! ```rust
//! use fsm_rs::state_machine;
//!
//! struct Context {
//!     failures: u32,
//! }
//!
//! state_machine! {
//!     name: CircuitBreaker,
//!     context: Context,
//!
//!     states: { *Closed, Open, HalfOpen },
//!     events: { CallSucceeded, CallFailed, TimerFired },
//!
//!     transitions: {
//!         Closed   + CallFailed [should_trip] => Open,
//!         Closed   + CallFailed / record_failure => Closed,
//!         Open     + TimerFired => HalfOpen,
//!         HalfOpen + CallSucceeded / reset => Closed,
//!         HalfOpen + CallFailed => Open,
//!     }
//!
//!     unhandled: ignore,
//! }
//!
//! impl CircuitBreakerContext for Context {
//!     fn should_trip(&self) -> bool {
//!         self.failures >= 1
//!     }
//!     fn record_failure(&mut self) {
//!         self.failures += 1;
//!     }
//!     fn reset(&mut self) {
//!         self.failures = 0;
//!     }
//! }
//!
//! let mut machine = CircuitBreaker::new(Context { failures: 0 });
//! machine.process_event(Event::CallFailed).unwrap();
//! assert_eq!(machine.state(), State::Closed); // guard failed, second row matched
//! machine.process_event(Event::CallFailed).unwrap();
//! assert_eq!(machine.state(), State::Open);
//! ```
//!
//! ## Hierarchical states
//!
//! States can nest; a composite state must mark one initial child with `*`.
//! The generated `State` enum contains leaf states only. Dispatch is
//! child-first (a row naming a composite covers every leaf inside it, but a
//! row naming the leaf itself wins), and entry/exit hooks fire along the path
//! through the least common ancestor (exits innermost-first, entries
//! outermost-first, shared ancestors never re-fire):
//!
//! ```rust
//! use fsm_rs::state_machine;
//!
//! struct Log(Vec<&'static str>);
//!
//! state_machine! {
//!     name: Battery,
//!     context: Log,
//!
//!     states: {
//!         *Idle,
//!         Active(entry: enter_active) {
//!             *Charging(exit: exit_charging),
//!             Discharging,
//!         },
//!         Done,
//!     },
//!     events: { PlugIn, Full, Empty },
//!
//!     transitions: {
//!         Idle + PlugIn => Active,   // enters the initial child: Charging
//!         Charging + Full => Discharging,
//!         Active + Empty => Done,    // matches from any leaf inside Active
//!         _ + PlugIn | Full | Empty => _,
//!     }
//! }
//!
//! impl BatteryContext for Log {
//!     fn enter_active(&mut self) { self.0.push("enter Active"); }
//!     fn exit_charging(&mut self) { self.0.push("exit Charging"); }
//! }
//!
//! let mut m = Battery::new(Log(Vec::new()));
//! m.process_event(Event::PlugIn).unwrap();
//! assert_eq!(m.state(), State::Charging);        // Active's initial child
//! assert_eq!(m.context().0, ["enter Active"]);
//!
//! m.process_event(Event::Empty).unwrap();        // cross-boundary move
//! assert_eq!(m.state(), State::Done);
//! assert_eq!(m.context().0, ["enter Active", "exit Charging"]);
//! // note: `enter Active` did NOT fire again on the way out
//! ```

pub use async_trait::async_trait;
pub use fsm_rs_macros::state_machine;

#[cfg(feature = "serde")]
pub use serde;

use core::fmt;

/// Error returned by `process_event` when no transition fired for an event.
///
/// This can only happen at runtime when every matching row had a guard that
/// evaluated to `false` and the table has no `unhandled:` policy. Machines
/// without guards and with an exhaustive table never produce this error (but
/// the `Result` is part of the uniform API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError<S, E> {
    /// The event had no matching transition in the current state, or all
    /// guards on matching rows evaluated to `false`.
    Unhandled {
        /// The state the machine was in.
        state: S,
        /// The event that could not be handled.
        event: E,
    },
}

impl<S: fmt::Debug, E: fmt::Debug> fmt::Display for TransitionError<S, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::Unhandled { state, event } => write!(
                f,
                "no transition matched event {event:?} in state {state:?}"
            ),
        }
    }
}

impl<S: fmt::Debug, E: fmt::Debug> std::error::Error for TransitionError<S, E> {}
