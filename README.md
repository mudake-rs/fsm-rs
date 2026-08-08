# fsm-rs

An Akka-inspired finite state machine library for Rust with a table-style DSL,
**compile-time transition exhaustiveness**, first-class **async** support and
optional **serde persistence**.

```rust
use fsm_rs::state_machine;

struct Context {
    failures: u32,
}

state_machine! {
    name: CircuitBreaker,
    context: Context,

    states: { *Closed, Open, HalfOpen },   // `*` marks the initial state
    events: { CallSucceeded, CallFailed, TimerFired },

    transitions: {
        Closed   + CallFailed [should_trip] / record_failure => Open,
        Closed   + CallFailed / record_failure => _,   // `_` target = stay
        Open     + TimerFired => HalfOpen,
        HalfOpen + CallSucceeded / reset => Closed,
        HalfOpen + CallFailed => Open,
        _ + TimerFired => _,                            // `_` source = any state
        _ + CallSucceeded => _,
    }

    on_transition: log_transition,         // optional hooks
    unhandled: ignore,                     // only needed if the table isn't exhaustive
}

impl CircuitBreakerContext for Context {
    fn should_trip(&self) -> bool { self.failures >= 1 }
    fn record_failure(&mut self) { self.failures += 1; }
    fn reset(&mut self) { self.failures = 0; }
    fn log_transition(&mut self, from: &State, to: &State, event: &Event) {
        println!("{event:?}: {from:?} -> {to:?}");
    }
}

let mut machine = CircuitBreaker::new(Context { failures: 0 });
machine.process_event(Event::CallFailed)?;
```

## Why another FSM crate?

Existing Rust FSM libraries either punt undefined transitions to runtime
(`rust-fsm`, `smlang` return an `Err`), silently ignore them (`statig`), or
give you full typestate safety that evaporates the moment events arrive at
runtime over a channel. Because `state_machine!` sees the whole transition
table, it can prove coverage **at compile time**:

```
error: unhandled transitions in `CircuitBreaker`: (Open, CallSucceeded), (Open, CallFailed)
       add transitions, a wildcard arm (`_ + Event => ...`), or an `unhandled:` policy
```

The compiler also rejects transitions referencing undeclared states or events,
unreachable rows shadowed by earlier unguarded rows, and multiple initial
states.

## The generated API

The macro generates, in the current module (use one module per machine, since
the enum names are deliberately generic):

- `enum State` / `enum Event` — `Debug + Clone + Copy + PartialEq + Eq + Hash`.
- `trait <Name>Context` — every guard, action and hook you referenced. You
  implement it for your context type; missing implementations are compile
  errors.
- `struct <Name>` — the machine:

```rust
impl CircuitBreaker {
    fn new(context: Context) -> Self;          // starts in the initial state
    fn state(&self) -> State;
    fn context(&self) -> &Context;
    fn context_mut(&mut self) -> &mut Context;
    fn process_event(&mut self, event: Event)
        -> Result<(), fsm_rs::TransitionError<State, Event>>;  // async if any callback is async
}
```

## Semantics (Akka mapping)

All mutable data lives in the context — Akka's `Data`, mutated via `using(...)`,
maps to actions taking `&mut self`. For each (state, event) pair, rows are
tried with the **deepest source state first**, ties broken by **declaration
order** (for flat machines this is simply declaration order); the first row
whose source, event and guard all match fires.

| fsm-rs | Akka FSM |
|---|---|
| `State + Event / action => Target` | `case Event => goto(Target).using(...)` |
| `=> _` (stay) | `stay()` — no hooks fire |
| `=> SameState` | `goto(SameState)` — exit/entry/`on_transition` fire |
| `[guard]` | `if` in a `case` — failure tries the next row |
| `unhandled: on_unhandled` | `whenUnhandled` |
| `on_transition: method` | `onTransition` |
| `states: { S(entry: f, exit: g) }` | — (Akka FSM has no entry/exit actions) |

A transition with a target runs, in order (UML): source `exit` hook(s) →
action → state change → target `entry` hook(s) → `on_transition`.

If every matching row's guard fails and there is no fallback row and no
`unhandled:` policy, `process_event` returns
`Err(TransitionError::Unhandled { state, event })`. Without guards (or with a
policy) this is unreachable — the `Result` just keeps the API uniform.

## Hierarchical states

States can nest: a composite state wraps child states in braces and must mark
exactly one initial child with `*`. Hooks work on composites just like on leaf
states.

```rust
states: {
    *Idle,
    Active(entry: on_active, exit: off_active) {
        *Charging,
        Discharging,
    },
    Done,
},
transitions: {
    Idle        + PlugIn => Active,   // enters Active's initial: Charging
    Charging    + Full   => Discharging,
    Active      + Unplug => Idle,     // matches from ANY leaf inside Active
    Discharging + Empty  => Done,
    // ...
}
```

The generated `State` enum contains **leaf states only**; composites exist
purely at definition time. The macro flattens the hierarchy into the plain
transition table before checking it, so compile-time exhaustiveness, async and
serde all work exactly as with flat machines.

### Dispatch: child first

A transition source naming a composite covers every leaf inside it. When
several rows match the same (leaf, event) pair, the row with the **deepest
source** wins; ties are broken by declaration order. A wildcard `_` source is
the least specific. Given:

```text
Active   + Unplug => Idle,     // declared first
Charging + Unplug => Done,     // still wins for Charging
```

`Unplug` in `Charging` goes to `Done`, in `Discharging` to `Idle`. This is the
same child-first bubbling as XState or statig's `Super`, except it is resolved
at compile time. Guard failure composes with it: if the `Charging` row had a
guard that evaluates to `false`, the event falls through to the `Active` row,
then to any wildcard row, then to the `unhandled` policy (or
`TransitionError::Unhandled`).

### Entry/exit ordering (UML LCA rule)

Hooks fire along the path between source and target through their least common
ancestor composite: exit hooks from the source up to (excluding) the LCA,
innermost-first, then the action, then entry hooks from the LCA down to the
target, outermost-first. Shared ancestors never re-fire. Concretely, for the
machine above:

| Transition | Hooks fired |
|---|---|
| `Idle → Active` | `on_active` (enters initial `Charging`) |
| `Charging → Discharging` (siblings) | no composite hooks |
| `Charging → Done` (cross-boundary) | `exit_charging`, `off_active` |
| `Charging → Charging` (self) | exits/re-enters `Charging` only |

`examples/battery.rs` prints these sequences live.

### Targeting composites

A transition targeting a composite resolves to its initial leaf (recursively),
including from inside the composite itself (`Charging => Active` re-enters
`Charging` without touching `Active`'s hooks). The machine's own initial state
follows the same rule when the top-level `*` marks a composite.

### Persistence with hierarchies

The serialized format is unchanged — the leaf tag only
(`{"state": "Charging", ...}`), since ancestors are derivable from the
definition. One caveat: **re-parenting a leaf in a later version is a
migration hazard** — a stored `Charging` silently changes its entry/exit
behavior on restore if you move it to another composite.

### Not supported (yet)

History pseudostates (shallow or deep) and orthogonal/parallel regions are
deliberately out of scope: history changes the persisted format (one "last
leaf" per composite), and parallel regions break the single-active-state model
the exhaustiveness check is built on.

## Async

Prefix any callback with `async` — guards, actions, entry/exit hooks,
`unhandled` and `on_transition` methods all support it, and sync and async
callbacks can be mixed freely in one machine. If anything is async,
`process_event` becomes async:

```rust
state_machine! {
    name: Worker,
    context: Ctx,
    states: { *Idle, Busy(exit: async cleanup) },
    events: { Start, Finish, Ping },
    transitions: {
        Idle + Start [async capacity_available] / async begin => Busy,
        Busy + Finish => Idle,
        _ + Ping => _,
        Idle + Finish => _,
        Busy + Start => _,
    }
}

#[fsm_rs::async_trait]
impl WorkerContext for Ctx {
    async fn capacity_available(&self) -> bool { /* ... */ }
    async fn begin(&mut self) { /* ... */ }
    async fn cleanup(&mut self) { /* ... */ }
}

machine.process_event(Event::Start).await?;
```

The core is runtime-agnostic (no tokio dependency); async callbacks use the
re-exported [`async-trait`](https://docs.rs/async-trait), so futures are `Send`
and contexts behind `&self` must be `Sync`. See `examples/async_job.rs` for a
runnable version and `examples/circuit_breaker.rs` for driving a machine from
a `tokio::sync::mpsc` mailbox loop.

## Persistence (serde)

With the `serde` feature enabled and `serde: true` in the DSL header, the
machine and its `State` enum derive `Serialize`/`Deserialize`. A machine is
just `state + context`, so the format is exactly that:

```json
{ "state": "Bought", "context": { "id": 42, "amount_cents": 1999 } }
```

Typical flow — store on one side of a process boundary, restore and continue
on the other:

```rust
let row = serde_json::to_string(&purchase)?;        // "database"
let mut restored: Purchase = serde_json::from_str(&row)?;
restored.process_event(Event::Refund).await?;       // keep going
```

Not everything has to be serializable: the context is your own type, so use
standard serde attributes — `#[serde(skip)]` fields are simply not stored and
come back as `Default::default()` on restore (make sure the restored state
doesn't depend on them). Restoring is *not* a transition: no `entry`,
`exit` or `on_transition` hooks fire.

The serialized format has no versioning yet — treat it as internal to a
deployment, not as an archive format. See `examples/purchase_persist.rs`.

## DSL reference

```text
state_machine! {
    name: MachineName,              // required
    context: ContextType,           // required; plain (non-generic) type
    serde: true,                    // optional; requires the `serde` feature
    states: { *A, B(entry: f, exit: g), C { *D, E } },   // required; nested
                                                         // blocks allowed, one
                                                         // `*` per level
    events: { E1, E2 },             // required
    transitions: {                  // required
        A | B + E1 | E2 [async guard] / async action => Target,
        _     + _                              => _,   // wildcards
    }
    unhandled: ignore | async on_unhandled,   // optional if table is exhaustive
    on_transition: async method,              // optional
}
```

Callback signatures on the generated `<Name>Context` trait:

```rust
fn guard(&self) -> bool;
fn action(&mut self);                       // also entry/exit hooks
fn on_unhandled(&mut self, state: &State, event: &Event);
fn on_transition(&mut self, from: &State, to: &State, event: &Event);
```

## Not in iteration 1 (roadmap)

- Timers (`startSingleTimer`-style) — send events from your own tasks for now.
- Per-state and per-event payloads (data-carrying states/events), including
  per-state serde granularity.
- History pseudostates and parallel regions (see "Hierarchical states").
- A batteries-included actor/mailbox runner.
- Serialization schema evolution.

## Development

```sh
cargo test --workspace --all-features   # unit + behavior + serde + trybuild
cargo clippy --workspace --all-features -- -D warnings
cargo run --example circuit_breaker
cargo run --example async_job
cargo run --example battery
cargo run --example purchase_persist --features serde
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
