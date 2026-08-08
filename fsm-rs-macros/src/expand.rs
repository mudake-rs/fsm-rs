use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::Ident;

use crate::model::{
    Callable, EventPattern, MachineDef, Row, SourcePattern, StateDef, Target, Unhandled,
};
use crate::tree::{find_mut, Tree};

pub fn expand(def: &mut MachineDef) -> syn::Result<TokenStream> {
    apply_row_initial_markers(def);
    validate(def)?;
    Ok(generate(def))
}

/// `*` markers on row sources are another way to designate initial states;
/// fold them into the state definitions so the tree has a single source of
/// truth.
fn apply_row_initial_markers(def: &mut MachineDef) {
    for row in &mut def.transitions {
        if let SourcePattern::States(refs) = &row.sources {
            for r in refs {
                if r.initial_marker {
                    if let Some(s) = find_mut(&mut def.states, &r.name.to_string()) {
                        s.initial = true;
                    }
                    // Unknown names are reported by validation; nothing to do.
                }
            }
        }
    }
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

    let tree = Tree::new(&def.states);
    validate_names(def, &tree, &mut errors);
    check_initial_levels(&def.states, None, &mut errors);
    validate_callable_kinds(def, &tree, &mut errors);
    validate_coverage(def, &tree, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(combine(errors))
    }
}

/// Duplicate names (states share one namespace across the whole tree) and
/// references to unknown states/events in rows.
fn validate_names(def: &MachineDef, tree: &Tree, errors: &mut Vec<syn::Error>) {
    let mut seen_states: HashSet<String> = HashSet::new();
    for s in &tree.all {
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

    for row in &def.transitions {
        if let SourcePattern::States(refs) = &row.sources {
            for r in refs {
                if tree.get(&r.name.to_string()).is_none() {
                    errors.push(syn::Error::new(
                        r.name.span(),
                        format!("unknown state `{}`", r.name),
                    ));
                }
            }
        }
        if let EventPattern::Events(evs) = &row.events {
            for e in evs {
                if !seen_events.contains(&e.to_string()) {
                    errors.push(syn::Error::new(e.span(), format!("unknown event `{e}`")));
                }
            }
        }
        if let Target::State(t) = &row.target {
            if tree.get(&t.to_string()).is_none() {
                errors.push(syn::Error::new(t.span(), format!("unknown state `{t}`")));
            }
        }
    }
}

/// Callable name/kind consistency: the same method name may not be used in
/// two roles with different signatures.
fn validate_callable_kinds(def: &MachineDef, tree: &Tree, errors: &mut Vec<syn::Error>) {
    let mut callable_kinds: Vec<(String, &'static str, proc_macro2::Span)> = Vec::new();
    {
        let mut note = |c: &Callable, kind: &'static str| {
            callable_kinds.push((c.name.to_string(), kind, c.name.span()));
        };
        for s in &tree.all {
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
}

/// Unreachable-row detection and the exhaustiveness check over the flattened
/// (leaf x event) matrix.
fn validate_coverage(def: &MachineDef, tree: &Tree, errors: &mut Vec<syn::Error>) {
    let rows = row_infos(def, tree);
    let n_leaves = tree.leaves.len();
    let n_events = def.events.len();

    let mut covered_any: HashSet<(usize, usize)> = HashSet::new();
    for info in &rows {
        for (l, _) in &info.leaf_specs {
            for e in &info.ev_idxs {
                covered_any.insert((*l, *e));
            }
        }
    }

    // Unreachable rows: within each pair's effective order (specificity desc,
    // then declaration order), everything after the first unguarded row is
    // dead for that pair. A row is unreachable when it is dead for every
    // pair it covers.
    let mut live_pairs: Vec<HashSet<(usize, usize)>> =
        rows.iter().map(|_| HashSet::new()).collect();
    for l in 0..n_leaves {
        for e in 0..n_events {
            let mut order: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, info)| info.covers(l, e))
                .map(|(i, _)| i)
                .collect();
            order.sort_by_key(|&i| std::cmp::Reverse(rows[i].specificity(l)));
            for &i in &order {
                live_pairs[i].insert((l, e));
                if rows[i].row.guard.is_none() {
                    break;
                }
            }
        }
    }
    for (i, info) in rows.iter().enumerate() {
        let pairs: Vec<(usize, usize)> = info.pairs();
        if !pairs.is_empty() && pairs.iter().all(|p| !live_pairs[i].contains(p)) {
            errors.push(syn::Error::new(
                info.row.span,
                "unreachable transition: all of its (state, event) pairs are already \
                 covered by earlier, more specific rows without guards",
            ));
        }
    }

    if def.unhandled.is_none() {
        let missing: Vec<String> = (0..n_leaves)
            .flat_map(|l| (0..n_events).map(move |e| (l, e)))
            .filter(|p| !covered_any.contains(p))
            .map(|(l, e)| format!("({}, {})", tree.leaves[l].name, def.events[e]))
            .collect();
        if !missing.is_empty() {
            let shown = missing.len().min(8);
            let mut list = missing[..shown].join(", ");
            if missing.len() > shown {
                list = format!("{list} and {} more", missing.len() - shown);
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
}

/// A transition row with its expansion over the leaf set precomputed.
struct RowInfo<'a> {
    row: &'a Row,
    /// (leaf index, specificity) — specificity is the depth of the source
    /// that covers the leaf; wildcard sources count as -1.
    leaf_specs: Vec<(usize, i32)>,
    ev_idxs: Vec<usize>,
}

impl RowInfo<'_> {
    fn covers(&self, leaf: usize, event: usize) -> bool {
        self.leaf_specs.iter().any(|(l, _)| *l == leaf) && self.ev_idxs.contains(&event)
    }

    fn specificity(&self, leaf: usize) -> i32 {
        self.leaf_specs
            .iter()
            .filter(|(l, _)| *l == leaf)
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(i32::MIN)
    }

    fn pairs(&self) -> Vec<(usize, usize)> {
        self.leaf_specs
            .iter()
            .flat_map(|(l, _)| self.ev_idxs.iter().map(move |e| (*l, *e)))
            .collect()
    }
}

fn row_infos<'a>(def: &'a MachineDef, tree: &Tree) -> Vec<RowInfo<'a>> {
    def.transitions
        .iter()
        .map(|row| {
            let mut leaf_specs: Vec<(usize, i32)> = Vec::new();
            let mut add =
                |idx: usize, spec: i32| match leaf_specs.iter_mut().find(|(l, _)| *l == idx) {
                    Some(slot) => slot.1 = slot.1.max(spec),
                    None => leaf_specs.push((idx, spec)),
                };
            match &row.sources {
                SourcePattern::Any => {
                    for i in 0..tree.leaves.len() {
                        add(i, -1);
                    }
                }
                SourcePattern::States(refs) => {
                    for r in refs {
                        let name = r.name.to_string();
                        for leaf in tree.leaves_under(&name) {
                            if let Some(i) = tree.leaves.iter().position(|l| l.name == leaf.name) {
                                add(i, tree.depth(&name));
                            }
                        }
                    }
                }
            }
            let ev_idxs = match &row.events {
                EventPattern::Any => (0..def.events.len()).collect(),
                EventPattern::Events(ids) => ids
                    .iter()
                    .filter_map(|e| def.events.iter().position(|x| x == e))
                    .collect(),
            };
            RowInfo {
                row,
                leaf_specs,
                ev_idxs,
            }
        })
        .collect()
}

/// Initial-state rules, checked per level: exactly one `*` child per
/// composite; at most one `*` at the top level (default: first state).
fn check_initial_levels(level: &[StateDef], parent: Option<&Ident>, errors: &mut Vec<syn::Error>) {
    let starred: Vec<&Ident> = level
        .iter()
        .filter(|s| s.initial)
        .map(|s| &s.name)
        .collect();
    let first = starred.first().map(ToString::to_string);
    for s in &starred[starred.len().min(1)..] {
        if Some(s.to_string()) != first {
            let where_ = match parent {
                Some(p) => format!("composite `{p}`"),
                None => "the top level".to_string(),
            };
            errors.push(syn::Error::new(
                s.span(),
                format!(
                    "multiple initial states in {where_}: `{}` and `{s}`; \
                     only one state may be marked with `*`",
                    first.as_deref().unwrap_or_default()
                ),
            ));
        }
    }
    if let Some(p) = parent {
        if starred.is_empty() {
            errors.push(syn::Error::new(
                p.span(),
                format!("composite state `{p}` must mark exactly one initial child with `*`"),
            ));
        }
    }
    for s in level {
        if !s.children.is_empty() {
            check_initial_levels(&s.children, Some(&s.name), errors);
        }
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
    let tree = Tree::new(&def.states);
    let rows = row_infos(def, &tree);

    let name = &def.name;
    let context_ty = &def.context;
    let trait_name = format_ident!("{}Context", name);
    let initial = &tree.initial_leaf(&def.states).name;
    let any_async = def.callables().iter().any(|c| c.is_async);

    let serde_attr = def.serde.then(|| {
        quote! {
            #[derive(::fsm_rs::serde::Serialize, ::fsm_rs::serde::Deserialize)]
            #[serde(crate = "::fsm_rs::serde")]
        }
    });
    let async_trait_attr = any_async.then(|| quote!(#[::fsm_rs::async_trait]));
    let asyncness = any_async.then(|| quote!(async));

    let state_variants: Vec<&Ident> = tree.leaves.iter().map(|s| &s.name).collect();
    let event_variants: Vec<&Ident> = def.events.iter().collect();
    let methods = trait_methods(def, &tree);
    let arms = dispatch_arms(def, &tree, &rows);

    quote! {
        /// States of the generated state machine (leaf states of the
        /// hierarchy).
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

            /// The current state (a leaf state of the hierarchy).
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

            /// Processes an event: runs the first matching transition and
            /// moves the machine to the target state.
            #[allow(unused_variables)]
            pub #asyncness fn process_event(
                &mut self,
                event: Event,
            ) -> ::core::result::Result<(), ::fsm_rs::TransitionError<State, Event>> {
                let from = self.state;
                match from {
                    #(#arms)*
                }
            }
        }
    }
}

/// The generated context-trait method signatures, deduplicated by name
/// (validation guarantees consistent roles).
fn trait_methods(def: &MachineDef, tree: &Tree) -> Vec<TokenStream> {
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
    for s in &tree.all {
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
    methods
}

/// What happens when no row fires: the `unhandled` policy or an error.
fn fallback_tokens(def: &MachineDef) -> TokenStream {
    match &def.unhandled {
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
    }
}

/// Per-leaf dispatch arms: rows covering the leaf, in effective order (more
/// specific sources first, then declaration order).
fn dispatch_arms(def: &MachineDef, tree: &Tree, rows: &[RowInfo]) -> Vec<TokenStream> {
    let fallback = fallback_tokens(def);
    tree.leaves
        .iter()
        .enumerate()
        .map(|(leaf_idx, leaf)| {
            let leaf_name = &leaf.name;
            let mut covering: Vec<&RowInfo> = rows
                .iter()
                .filter(|info| info.leaf_specs.iter().any(|(l, _)| *l == leaf_idx))
                .collect();
            covering.sort_by_key(|info| std::cmp::Reverse(info.specificity(leaf_idx)));
            let blocks: Vec<TokenStream> = covering
                .iter()
                .map(|info| row_block(def, tree, leaf, info.row))
                .collect();
            quote! {
                State::#leaf_name => {
                    #(#blocks)*
                    #fallback
                }
            }
        })
        .collect()
}

fn await_tokens(c: &Callable) -> Option<TokenStream> {
    c.is_async.then(|| quote!(.await))
}

/// The dispatch block for one row within one leaf state's arm. The source
/// leaf is statically known here, so entry/exit hook sequences are fully
/// expanded.
fn row_block(def: &MachineDef, tree: &Tree, leaf: &StateDef, row: &Row) -> TokenStream {
    let ev_cond = match &row.events {
        EventPattern::Any => None,
        EventPattern::Events(ids) => {
            let pats: Vec<TokenStream> = ids.iter().map(|e| quote!(Event::#e)).collect();
            Some(quote!(matches!(event, #(#pats)|*)))
        }
    };

    let action = row.action.as_ref().map(|c| {
        let m = &c.name;
        let aw = await_tokens(c);
        quote!(self.context.#m()#aw;)
    });

    let body = match &row.target {
        Target::Stay => quote!(#action),
        Target::State(target) => {
            let target_leaf = tree.resolve_initial(&target.to_string());
            let target_name = &target_leaf.name;

            let exit_calls: Vec<TokenStream> = tree
                .exit_path(&leaf.name.to_string(), &target_name.to_string())
                .iter()
                .filter_map(|s| s.exit.as_ref())
                .map(|c| {
                    let m = &c.name;
                    let aw = await_tokens(c);
                    quote!(self.context.#m()#aw;)
                })
                .collect();
            let entry_calls: Vec<TokenStream> = tree
                .entry_path(&leaf.name.to_string(), &target_name.to_string())
                .iter()
                .filter_map(|s| s.entry.as_ref())
                .map(|c| {
                    let m = &c.name;
                    let aw = await_tokens(c);
                    quote!(self.context.#m()#aw;)
                })
                .collect();
            let on_transition = def.on_transition.as_ref().map(|c| {
                let m = &c.name;
                let aw = await_tokens(c);
                quote!(self.context.#m(&from, &self.state, &event)#aw;)
            });
            quote! {
                #(#exit_calls)*
                #action
                self.state = State::#target_name;
                #(#entry_calls)*
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

    match ev_cond {
        Some(cond) => quote! {
            if #cond {
                #inner
            }
        },
        None => inner,
    }
}
