//! Proc-macro implementation of the `fsm-rs` DSL.
//!
//! Do not depend on this crate directly; use `fsm-rs` and its re-exported
//! [`state_machine!`](macro@state_machine) macro.

mod expand;
mod model;
mod parse;
mod tree;

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
///         Composite { *ChildA, ChildB }, // nested states; one `*` per level
///     },
///     events: { EventA, EventB },
///
///     transitions: {
///         Initial + EventA [guard] / action => Other,
///         Other   + EventA | EventB => _,   // `_` target = stay (no hooks)
///         _       + EventB => Initial,      // `_` source = any state
///         Composite + EventA => Initial,    // composite source = every child leaf
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
/// (leaf state, event) pair — missing pairs are a compile error.
///
/// # Hierarchical states
///
/// A state with a braced child block is a *composite*; the generated `State`
/// enum contains leaf states only. Semantics follow UML statecharts:
///
/// * **Dispatch is child-first**: for a given (leaf, event) pair the row with
///   the deepest source wins; ties break by declaration order. A composite
///   source covers every leaf inside it.
/// * **Entering a composite** (as a transition target, or as the machine's
///   initial state) resolves to its initial child, recursively.
/// * **Entry/exit hooks** fire along the path through the least common
///   ancestor: exits innermost-first from the source up to (excluding) the
///   LCA, then the action, then entries outermost-first down to the target.
///   Shared ancestors never re-fire; a self-transition (`S => S`) exits and
///   re-enters only `S`.
#[proc_macro]
pub fn state_machine(input: TokenStream) -> TokenStream {
    let mut def = syn::parse_macro_input!(input as crate::model::MachineDef);
    match expand::expand(&mut def) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
