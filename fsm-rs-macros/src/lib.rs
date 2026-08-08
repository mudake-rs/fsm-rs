//! Proc-macro implementation of the `fsm-rs` DSL.
//!
//! Do not depend on this crate directly; use `fsm-rs` and its re-exported
//! [`state_machine!`](macro@state_machine) macro.

mod expand;
mod model;
mod parse;

use proc_macro::TokenStream;

/// Defines a finite state machine from a transition table.
///
/// ```text
/// state_machine! {
///     name: MachineName,
///     context: ContextType,
///     serde: true,                     // optional; needs the `serde` feature
///
///     states: {
///         *Initial,                    // `*` marks the initial state
///         Other(entry: on_other, exit: off_other),
///     },
///     events: { EventA, EventB },
///
///     transitions: {
///         Initial + EventA [guard] / action => Other,
///         Other   + EventA | EventB => _,   // `_` target = stay (no hooks)
///         _       + EventB => Initial,      // `_` source = any state
///     }
///
///     unhandled: ignore,               // or: `unhandled: on_unhandled`
///     on_transition: log_transition,   // optional
/// }
/// ```
///
/// Guards, actions and hooks become methods on a generated
/// `MachineNameContext` trait that you implement for your context type.
/// Prefix a method name with `async` to make it (and `process_event`)
/// asynchronous, and implement the trait with `#[fsm_rs::async_trait]`.
///
/// Unless an `unhandled:` policy is given, the table must cover every
/// (state, event) pair — missing pairs are a compile error.
#[proc_macro]
pub fn state_machine(input: TokenStream) -> TokenStream {
    let def = syn::parse_macro_input!(input as crate::model::MachineDef);
    match expand::expand(&def) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
