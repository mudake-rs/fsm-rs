use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::Ident;

use crate::model::*;

pub fn expand(def: &MachineDef) -> syn::Result<TokenStream> {
    validate(def)?;
    Ok(generate(def))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate(def: &MachineDef) -> syn::Result<()> {
    let mut errors: Vec<syn::Error> = Vec::new();

    if def.states.is_empty() {
        errors.push(syn::Error::new(
            def.name.span(),
            "at least one state is required",
        ));
    }
    if def.events.is_empty() {
        errors.push(syn::Error::new(
            def.name.span(),
            "at least one event is required",
        ));
    }
    if def.states.is_empty() || def.events.is_empty() {
        return Err(combine(errors));
    }

    // Context must be a plain (non-generic) type path in iteration 1.
    let plain_path = match &def.context {
        syn::Type::Path(p) => {
            p.qself.is_none()
                && p.path
                    .segments
                    .iter()
                    .all(|s| matches!(s.arguments, syn::PathArguments::None))
        }
        _ => false,
    };
    if !plain_path {
        errors.push(syn::Error::new(
            def.context.span(),
            "context must be a plain, non-generic type (generic contexts are not supported yet)",
        ));
    }

    // Duplicate states / events.
    let mut seen_states: HashSet<String> = HashSet::new();
    for s in &def.states {
        if !seen_states.insert(s.name.to_string()) {
            errors.push(syn::Error::new(
                s.name.span(),
                format!("duplicate state `{}`", s.name),
            ));
        }
    }
    let mut seen_events: HashSet<String> = HashSet::new();
    for e in &def.events {
        if !seen_events.insert(e.to_string()) {
            errors.push(syn::Error::new(e.span(), format!("duplicate event `{e}`")));
        }
    }

    // Initial state markers: `*` in the states list or on a row source.
    let mut starred: Vec<&Ident> = def
        .states
        .iter()
        .filter(|s| s.initial)
        .map(|s| &s.name)
        .collect();
    for row in &def.transitions {
        if let SourcePattern::States(refs) = &row.sources {
            starred.extend(refs.iter().filter(|r| r.initial_marker).map(|r| &r.name));
        }
    }
    let first_star = starred.first().map(|i| i.to_string());
    for s in &starred[starred.len().min(1)..] {
        if Some(s.to_string()) != first_star {
            errors.push(syn::Error::new(
                s.span(),
                format!(
                    "multiple initial states: `{}` and `{s}`; only one state may be marked with `*`",
                    first_star.as_deref().unwrap_or_default()
                ),
            ));
        }
    }

    // Unknown states / events in rows.
    let state_names: HashSet<String> = def.states.iter().map(|s| s.name.to_string()).collect();
    let event_names: HashSet<String> = def.events.iter().map(|e| e.to_string()).collect();
    for row in &def.transitions {
        if let SourcePattern::States(refs) = &row.sources {
            for r in refs {
                if !state_names.contains(&r.name.to_string()) {
                    errors.push(syn::Error::new(
                        r.name.span(),
                        format!("unknown state `{}`", r.name),
                    ));
                }
            }
        }
        if let EventPattern::Events(evs) = &row.events {
            for e in evs {
                if !event_names.contains(&e.to_string()) {
                    errors.push(syn::Error::new(e.span(), format!("unknown event `{e}`")));
                }
            }
        }
        if let Target::State(t) = &row.target {
            if !state_names.contains(&t.to_string()) {
                errors.push(syn::Error::new(t.span(), format!("unknown state `{t}`")));
            }
        }
    }

    // Callable name/kind consistency: the same method name may not be used in
    // two roles with different signatures.
    let mut callable_kinds: Vec<(String, &'static str, proc_macro2::Span)> = Vec::new();
    {
        let mut note = |c: &Callable, kind: &'static str| {
            callable_kinds.push((c.name.to_string(), kind, c.name.span()));
        };
        for s in &def.states {
            if let Some(c) = &s.entry {
                note(c, "entry");
            }
            if let Some(c) = &s.exit {
                note(c, "exit");
            }
        }
        for row in &def.transitions {
            if let Some(c) = &row.guard {
                note(c, "guard");
            }
            if let Some(c) = &row.action {
                note(c, "action");
            }
        }
        if let Some(Unhandled::Method(c)) = &def.unhandled {
            note(c, "unhandled");
        }
        if let Some(c) = &def.on_transition {
            note(c, "on_transition");
        }
    }
    for i in 0..callable_kinds.len() {
        for j in (i + 1)..callable_kinds.len() {
            let (n1, k1, _) = &callable_kinds[i];
            let (n2, k2, span2) = &callable_kinds[j];
            if n1 == n2 && k1 != k2 {
                errors.push(syn::Error::new(
                    *span2,
                    format!(
                        "method `{n1}` is used both as a {k1} and as a {k2}; \
                         these roles have different signatures"
                    ),
                ));
            }
        }
    }

    // Coverage: unreachable rows and exhaustiveness.
    let state_idx: Vec<String> = def.states.iter().map(|s| s.name.to_string()).collect();
    let event_idx: Vec<String> = def.events.iter().map(|e| e.to_string()).collect();
    let mut covered_any: HashSet<(usize, usize)> = HashSet::new();
    let mut covered_unguarded: HashSet<(usize, usize)> = HashSet::new();

    for row in &def.transitions {
        let srcs: Vec<usize> = match &row.sources {
            SourcePattern::Any => (0..state_idx.len()).collect(),
            SourcePattern::States(refs) => refs
                .iter()
                .filter_map(|r| state_idx.iter().position(|s| r.name == s.as_str()))
                .collect(),
        };
        let evs: Vec<usize> = match &row.events {
            EventPattern::Any => (0..event_idx.len()).collect(),
            EventPattern::Events(ids) => ids
                .iter()
                .filter_map(|e| event_idx.iter().position(|x| *e == x.as_str()))
                .collect(),
        };
        let pairs: Vec<(usize, usize)> = srcs
            .iter()
            .flat_map(|s| evs.iter().map(move |e| (*s, *e)))
            .collect();

        if !pairs.is_empty() && pairs.iter().all(|p| covered_unguarded.contains(p)) {
            errors.push(syn::Error::new(
                row.span,
                "unreachable transition: all of its (state, event) pairs are already \
                 covered by earlier rows without guards",
            ));
        }

        if row.guard.is_none() {
            covered_unguarded.extend(pairs.iter().copied());
        }
        covered_any.extend(pairs);
    }

    if def.unhandled.is_none() {
        let missing: Vec<String> = (0..state_idx.len())
            .flat_map(|s| (0..event_idx.len()).map(move |e| (s, e)))
            .filter(|p| !covered_any.contains(p))
            .map(|(s, e)| format!("({}, {})", state_idx[s], event_idx[e]))
            .collect();
        if !missing.is_empty() {
            let shown = missing.len().min(8);
            let mut list = missing[..shown].join(", ");
            if missing.len() > shown {
                list.push_str(&format!(" and {} more", missing.len() - shown));
            }
            errors.push(syn::Error::new(
                def.name.span(),
                format!(
                    "unhandled transitions in `{}`: {list}\n\
                     add transitions, a wildcard arm (`_ + Event => ...`), \
                     or an `unhandled:` policy",
                    def.name
                ),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(combine(errors))
    }
}

fn combine(errors: Vec<syn::Error>) -> syn::Error {
    let mut iter = errors.into_iter();
    let mut acc = iter.next().expect("at least one error");
    for e in iter {
        acc.combine(e);
    }
    acc
}

fn initial_state(def: &MachineDef) -> &Ident {
    if let Some(s) = def.states.iter().find(|s| s.initial) {
        return &s.name;
    }
    for row in &def.transitions {
        if let SourcePattern::States(refs) = &row.sources {
            if let Some(r) = refs.iter().find(|r| r.initial_marker) {
                return &r.name;
            }
        }
    }
    &def.states[0].name
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

/// Trait method signature roles.
#[derive(Clone, Copy)]
enum MethodRole {
    Guard,
    /// Actions and entry/exit hooks share `fn(&mut self)`.
    Mutation,
    Unhandled,
    OnTransition,
}

fn generate(def: &MachineDef) -> TokenStream {
    let name = &def.name;
    let context_ty = &def.context;
    let trait_name = format_ident!("{}Context", name);
    let initial = initial_state(def);
    let any_async = def.callables().iter().any(|c| c.is_async);

    let serde_attr = def.serde.then(|| {
        quote! {
            #[derive(::fsm_rs::serde::Serialize, ::fsm_rs::serde::Deserialize)]
            #[serde(crate = "::fsm_rs::serde")]
        }
    });
    let async_trait_attr = any_async.then(|| quote!(#[::fsm_rs::async_trait]));
    let asyncness = any_async.then(|| quote!(async));

    let state_variants: Vec<&Ident> = def.states.iter().map(|s| &s.name).collect();
    let event_variants: Vec<&Ident> = def.events.iter().collect();

    // Trait methods, deduplicated by name (validation guarantees consistent roles).
    let mut seen: HashSet<String> = HashSet::new();
    let mut methods: Vec<TokenStream> = Vec::new();
    let mut push_method = |c: &Callable, role: MethodRole| {
        if !seen.insert(c.name.to_string()) {
            return;
        }
        let m = &c.name;
        let aw = c.is_async.then(|| quote!(async));
        let sig = match role {
            MethodRole::Guard => quote!(#aw fn #m(&self) -> bool;),
            MethodRole::Mutation => quote!(#aw fn #m(&mut self);),
            MethodRole::Unhandled => {
                quote!(#aw fn #m(&mut self, state: &State, event: &Event);)
            }
            MethodRole::OnTransition => {
                quote!(#aw fn #m(&mut self, from: &State, to: &State, event: &Event);)
            }
        };
        methods.push(sig);
    };
    for s in &def.states {
        if let Some(c) = &s.entry {
            push_method(c, MethodRole::Mutation);
        }
        if let Some(c) = &s.exit {
            push_method(c, MethodRole::Mutation);
        }
    }
    for row in &def.transitions {
        if let Some(c) = &row.guard {
            push_method(c, MethodRole::Guard);
        }
        if let Some(c) = &row.action {
            push_method(c, MethodRole::Mutation);
        }
    }
    if let Some(Unhandled::Method(c)) = &def.unhandled {
        push_method(c, MethodRole::Unhandled);
    }
    if let Some(c) = &def.on_transition {
        push_method(c, MethodRole::OnTransition);
    }

    let row_blocks: Vec<TokenStream> = def
        .transitions
        .iter()
        .map(|row| row_block(def, row))
        .collect();

    let fallback = match &def.unhandled {
        Some(Unhandled::Ignore) => quote!(Ok(())),
        Some(Unhandled::Method(c)) => {
            let m = &c.name;
            let aw = await_tokens(c);
            quote!({
                self.context.#m(&from, &event)#aw;
                Ok(())
            })
        }
        None => quote!(Err(::fsm_rs::TransitionError::Unhandled {
            state: from,
            event,
        })),
    };

    quote! {
        /// States of the generated state machine.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #serde_attr
        pub enum State {
            #(#state_variants),*
        }

        /// Events accepted by the generated state machine.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Event {
            #(#event_variants),*
        }

        /// Callbacks (guards, actions and hooks) of the state machine.
        ///
        /// Implement this trait on the context type.
        #async_trait_attr
        pub trait #trait_name {
            #(#methods)*
        }

        /// The generated state machine.
        #serde_attr
        pub struct #name {
            state: State,
            context: #context_ty,
        }

        impl #name {
            /// Creates a new machine in the initial state.
            pub fn new(context: #context_ty) -> Self {
                Self {
                    state: State::#initial,
                    context,
                }
            }

            /// The current state.
            pub fn state(&self) -> State {
                self.state
            }

            /// The machine's context (state data).
            pub fn context(&self) -> &#context_ty {
                &self.context
            }

            /// Mutable access to the machine's context (state data).
            pub fn context_mut(&mut self) -> &mut #context_ty {
                &mut self.context
            }

            /// Processes an event: runs the first matching transition's action
            /// and moves the machine to the target state.
            #[allow(unused_variables)]
            pub #asyncness fn process_event(
                &mut self,
                event: Event,
            ) -> ::core::result::Result<(), ::fsm_rs::TransitionError<State, Event>> {
                let from = self.state;
                #(#row_blocks)*
                #fallback
            }
        }
    }
}

fn await_tokens(c: &Callable) -> Option<TokenStream> {
    c.is_async.then(|| quote!(.await))
}

fn row_block(def: &MachineDef, row: &Row) -> TokenStream {
    let src_cond = match &row.sources {
        SourcePattern::Any => None,
        SourcePattern::States(refs) => {
            let pats: Vec<TokenStream> = refs
                .iter()
                .map(|r| {
                    let n = &r.name;
                    quote!(State::#n)
                })
                .collect();
            Some(quote!(matches!(from, #(#pats)|*)))
        }
    };
    let ev_cond = match &row.events {
        EventPattern::Any => None,
        EventPattern::Events(ids) => {
            let pats: Vec<TokenStream> = ids.iter().map(|e| quote!(Event::#e)).collect();
            Some(quote!(matches!(event, #(#pats)|*)))
        }
    };
    let cond = match (src_cond, ev_cond) {
        (Some(s), Some(e)) => quote!(#s && #e),
        (Some(s), None) => quote!(#s),
        (None, Some(e)) => quote!(#e),
        (None, None) => quote!(true),
    };

    let action = row.action.as_ref().map(|c| {
        let m = &c.name;
        let aw = await_tokens(c);
        quote!(self.context.#m()#aw;)
    });

    let body = match &row.target {
        Target::Stay => quote!(#action),
        Target::State(target) => {
            let exit = exit_code(def, row);
            let entry = def
                .states
                .iter()
                .find(|s| s.name == *target)
                .and_then(|s| s.entry.as_ref())
                .map(|c| {
                    let m = &c.name;
                    let aw = await_tokens(c);
                    quote!(self.context.#m()#aw;)
                });
            let on_transition = def.on_transition.as_ref().map(|c| {
                let m = &c.name;
                let aw = await_tokens(c);
                quote!(self.context.#m(&from, &self.state, &event)#aw;)
            });
            quote! {
                #action
                #exit
                self.state = State::#target;
                #entry
                #on_transition
            }
        }
    };

    let inner = match &row.guard {
        None => quote! {
            #body
            return Ok(());
        },
        Some(g) => {
            let m = &g.name;
            let aw = await_tokens(g);
            quote! {
                if self.context.#m()#aw {
                    #body
                    return Ok(());
                }
            }
        }
    };

    quote! {
        if #cond {
            #inner
        }
    }
}

/// Exit-hook dispatch for a transition leaving its source state(s).
fn exit_code(def: &MachineDef, row: &Row) -> TokenStream {
    let candidates: Vec<&StateDef> = match &row.sources {
        SourcePattern::Any => def.states.iter().collect(),
        SourcePattern::States(refs) => def
            .states
            .iter()
            .filter(|s| refs.iter().any(|r| r.name == s.name))
            .collect(),
    };
    let with_exit: Vec<&&StateDef> = candidates.iter().filter(|s| s.exit.is_some()).collect();
    if with_exit.is_empty() {
        return quote!();
    }
    if candidates.len() == 1 {
        let c = with_exit[0].exit.as_ref().expect("filtered above");
        let m = &c.name;
        let aw = await_tokens(c);
        return quote!(self.context.#m()#aw;);
    }
    let arms = with_exit.iter().map(|s| {
        let state_name = &s.name;
        let c = s.exit.as_ref().expect("filtered above");
        let m = &c.name;
        let aw = await_tokens(c);
        quote!(State::#state_name => self.context.#m()#aw,)
    });
    quote! {
        match from {
            #(#arms)*
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}
