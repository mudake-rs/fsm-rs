use proc_macro2::Span;
use syn::{Ident, Type};

/// A reference to a user-implemented method, optionally `async`.
pub struct Callable {
    pub is_async: bool,
    pub name: Ident,
}

pub struct StateDef {
    pub name: Ident,
    /// Marked with `*` in the `states:` list.
    pub initial: bool,
    pub entry: Option<Callable>,
    pub exit: Option<Callable>,
}

pub struct StateRef {
    pub name: Ident,
    /// Marked with `*` in a transition row.
    pub initial_marker: bool,
}

pub enum SourcePattern {
    /// `_`
    Any,
    States(Vec<StateRef>),
}

pub enum EventPattern {
    /// `_`
    Any,
    Events(Vec<Ident>),
}

pub enum Target {
    /// `=> _` — Akka's `stay()`: no hooks fire.
    Stay,
    State(Ident),
}

pub struct Row {
    pub sources: SourcePattern,
    pub events: EventPattern,
    pub guard: Option<Callable>,
    pub action: Option<Callable>,
    pub target: Target,
    pub span: Span,
}

pub enum Unhandled {
    Ignore,
    Method(Callable),
}

pub struct MachineDef {
    pub name: Ident,
    pub context: Type,
    pub serde: bool,
    pub states: Vec<StateDef>,
    pub events: Vec<Ident>,
    pub transitions: Vec<Row>,
    pub unhandled: Option<Unhandled>,
    pub on_transition: Option<Callable>,
}

impl MachineDef {
    /// All callables referenced anywhere in the definition.
    pub fn callables(&self) -> Vec<&Callable> {
        let mut out: Vec<&Callable> = Vec::new();
        for s in &self.states {
            out.extend(s.entry.iter());
            out.extend(s.exit.iter());
        }
        for r in &self.transitions {
            out.extend(r.guard.iter());
            out.extend(r.action.iter());
        }
        if let Some(Unhandled::Method(c)) = &self.unhandled {
            out.push(c);
        }
        out.extend(self.on_transition.iter());
        out
    }
}
