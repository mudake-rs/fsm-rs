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
