//! Behavior tests for hierarchical (nested) states.

// ---------------------------------------------------------------------------
// LCA entry/exit ordering, entering composites, composite rows, self-transitions.
// ---------------------------------------------------------------------------

mod battery {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub log: Vec<String>,
    }

    state_machine! {
        name: Battery,
        context: Ctx,

        states: {
            *Idle,
            Active(entry: enter_active, exit: exit_active) {
                *Charging(exit: exit_charging),
                Discharging(entry: enter_discharging),
            },
            Done,
        },
        events: { PlugIn, Unplug, Full, Empty, Poke },

        transitions: {
            Idle + PlugIn => Active,            // enters initial: Charging
            Charging + Full => Discharging,     // sibling move
            Active + Unplug => Idle,            // composite row: any leaf of Active
            Charging + Empty => Done,
            Discharging + Empty => Done,
            Charging + PlugIn => Charging,      // explicit self-transition
            Discharging + PlugIn | Full => _,
            Idle + Unplug | Full | Empty => _,
            Done + PlugIn | Unplug | Full | Empty => _,
            _ + Poke => _,
        }

        on_transition: note,
    }

    impl BatteryContext for Ctx {
        fn enter_active(&mut self) {
            self.log.push("+Active".into());
        }
        fn exit_active(&mut self) {
            self.log.push("-Active".into());
        }
        fn exit_charging(&mut self) {
            self.log.push("-Charging".into());
        }
        fn enter_discharging(&mut self) {
            self.log.push("+Discharging".into());
        }
        fn note(&mut self, from: &State, to: &State, _event: &Event) {
            self.log.push(format!("{from:?}->{to:?}"));
        }
    }

    pub fn machine() -> Battery {
        Battery::new(Ctx { log: Vec::new() })
    }
}

use battery::{Event as B, State as S};

#[test]
fn entering_composite_enters_initial_leaf_with_entry_chain() {
    let mut m = battery::machine();
    assert_eq!(m.state(), S::Idle);

    m.process_event(B::PlugIn).unwrap();
    assert_eq!(m.state(), S::Charging);
    // exit(Idle: none) -> entry path [Active, Charging] -> on_transition
    assert_eq!(m.context().log, ["+Active", "Idle->Charging"]);
}

#[test]
fn sibling_transition_does_not_touch_composite_hooks() {
    let mut m = battery::machine();
    m.process_event(B::PlugIn).unwrap();
    m.context_mut().log.clear();

    m.process_event(B::Full).unwrap();
    assert_eq!(m.state(), S::Discharging);
    assert_eq!(
        m.context().log,
        ["-Charging", "+Discharging", "Charging->Discharging"]
    );
}

#[test]
fn composite_row_fires_from_any_leaf_and_exits_innermost_first() {
    let mut m = battery::machine();
    m.process_event(B::PlugIn).unwrap();
    m.process_event(B::Full).unwrap(); // now Discharging
    m.context_mut().log.clear();

    // Active + Unplug covers Discharging too; exit path = [Discharging, Active].
    m.process_event(B::Unplug).unwrap();
    assert_eq!(m.state(), S::Idle);
    assert_eq!(m.context().log, ["-Active", "Discharging->Idle"]);

    // From Charging the leaf exit hook runs first.
    let mut m = battery::machine();
    m.process_event(B::PlugIn).unwrap();
    m.context_mut().log.clear();
    m.process_event(B::Unplug).unwrap();
    assert_eq!(m.state(), S::Idle);
    assert_eq!(m.context().log, ["-Charging", "-Active", "Charging->Idle"]);
}

#[test]
fn cross_boundary_exit_order_is_innermost_first() {
    let mut m = battery::machine();
    m.process_event(B::PlugIn).unwrap();
    m.context_mut().log.clear();

    // Charging -> Done crosses the Active boundary.
    m.process_event(B::Empty).unwrap();
    assert_eq!(m.state(), S::Done);
    assert_eq!(m.context().log, ["-Charging", "-Active", "Charging->Done"]);
}

#[test]
fn self_transition_reenters_leaf_only() {
    let mut m = battery::machine();
    m.process_event(B::PlugIn).unwrap();
    m.context_mut().log.clear();

    m.process_event(B::PlugIn).unwrap(); // Charging => Charging
    assert_eq!(m.state(), S::Charging);
    assert_eq!(m.context().log, ["-Charging", "Charging->Charging"]);

    m.process_event(B::Poke).unwrap(); // wildcard stay: no hooks
    assert_eq!(m.state(), S::Charging);
    assert_eq!(m.context().log.len(), 2);
}

// ---------------------------------------------------------------------------
// Shadowing: leaf rows beat composite rows; guard fallthrough to composite.
// ---------------------------------------------------------------------------

mod shadow {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub flag: bool,
    }

    state_machine! {
        name: Shadow,
        context: Ctx,

        states: { *P { *A, B }, C },
        events: { E, F },

        transitions: {
            P + E => C,                  // declared first, but less specific than:
            A + E [flag_on] => B,        // leaf row wins for A when the guard passes
            C + E => P,                  // enters initial: A
            _ + F => _,
        }
    }

    impl ShadowContext for Ctx {
        fn flag_on(&self) -> bool {
            self.flag
        }
    }

    pub fn machine(flag: bool) -> Shadow {
        Shadow::new(Ctx { flag })
    }
}

#[test]
fn leaf_row_shadows_composite_row_regardless_of_declaration_order() {
    use shadow::{Event, State};

    let mut m = shadow::machine(true);
    assert_eq!(m.state(), State::A);
    m.process_event(Event::E).unwrap();
    assert_eq!(m.state(), State::B); // leaf row fired, not the composite row

    m.process_event(Event::F).unwrap(); // wildcard stay
    assert_eq!(m.state(), State::B);
}

#[test]
fn guard_failure_falls_through_to_composite_row() {
    use shadow::{Event, State};

    let mut m = shadow::machine(false);
    m.process_event(Event::E).unwrap();
    assert_eq!(m.state(), State::C); // leaf guard failed -> composite row fired

    // C + E => P enters the initial leaf.
    m.process_event(Event::E).unwrap();
    assert_eq!(m.state(), State::A);
}

// ---------------------------------------------------------------------------
// Partial shadowing stays legal: the composite row is still reachable.
// ---------------------------------------------------------------------------

mod partial_shadow {
    use fsm_rs::state_machine;

    pub struct Ctx;

    state_machine! {
        name: Partial,
        context: Ctx,

        states: { *P { *A, B }, C },
        events: { G, H },

        transitions: {
            P + G => C,      // shadowed for A by the row below, live for B
            A + G => B,
            C + G => _,
            _ + H => _,
        }
    }

    impl PartialContext for Ctx {}
}

#[test]
fn composite_row_remains_reachable_from_unshadowed_leaves() {
    use partial_shadow::{Event, State};

    let mut m = partial_shadow::Partial::new(partial_shadow::Ctx);
    m.process_event(Event::G).unwrap();
    assert_eq!(m.state(), State::B); // leaf row

    m.process_event(Event::G).unwrap();
    assert_eq!(m.state(), State::C); // composite row from B

    m.process_event(Event::H).unwrap(); // wildcard stay
    assert_eq!(m.state(), State::C);
}

// ---------------------------------------------------------------------------
// Async hooks in a hierarchy; UML ordering: exit before action.
// ---------------------------------------------------------------------------

mod async_hierarchy {
    use fsm_rs::state_machine;

    pub struct Ctx {
        pub log: Vec<&'static str>,
    }

    state_machine! {
        name: AsyncH,
        context: Ctx,

        states: { *Outer { *A(exit: async cleanup), B }, C },
        events: { Go, Back, Ping },

        transitions: {
            A + Go / async act => C,   // cross-boundary: async exit, then action
            C + Back => Outer,         // enters initial: A
            B + Back => C,
            A + Back => _,
            B + Go => _,
            C + Go => _,
            A + Ping => B,
            B | C + Ping => _,
        }
    }

    #[fsm_rs::async_trait]
    impl AsyncHContext for Ctx {
        async fn cleanup(&mut self) {
            self.log.push("cleanup");
        }
        async fn act(&mut self) {
            self.log.push("act");
        }
    }
}

#[tokio::test]
async fn async_exit_hook_runs_before_async_action() {
    use async_hierarchy::{Event, State};

    let mut m = async_hierarchy::AsyncH::new(async_hierarchy::Ctx { log: Vec::new() });
    m.process_event(Event::Go).await.unwrap();
    assert_eq!(m.state(), State::C);
    assert_eq!(m.context().log, ["cleanup", "act"]); // exit -> action (UML order)

    m.process_event(Event::Back).await.unwrap();
    assert_eq!(m.state(), State::A); // entering Outer resolves to initial leaf

    m.process_event(Event::Ping).await.unwrap();
    assert_eq!(m.state(), State::B);
    m.process_event(Event::Ping).await.unwrap(); // stay
    assert_eq!(m.state(), State::B);
}

// ---------------------------------------------------------------------------
// Serde: nested machines persist as their leaf state tag.
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_hierarchy {
    use fsm_rs::state_machine;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct Ctx {
        pub cycles: u32,
    }

    state_machine! {
        name: Dimmer,
        context: Ctx,
        serde: true,

        states: { *Off, On { *Low, High } },
        events: { Toggle, Up, Down },

        transitions: {
            Off + Toggle => On,      // -> Low
            Low + Up => High,
            High + Down => Low,
            On + Toggle => Off,
            Off + Up | Down => _,
            Low + Down => _,
            High + Up => _,
        }
    }

    impl DimmerContext for Ctx {}
}

#[cfg(feature = "serde")]
#[test]
fn nested_machine_persists_as_leaf_tag() {
    use serde_hierarchy::{Event, State};

    let mut m = serde_hierarchy::Dimmer::new(serde_hierarchy::Ctx { cycles: 0 });
    m.process_event(Event::Toggle).unwrap();
    assert_eq!(m.state(), State::Low);
    m.process_event(Event::Up).unwrap();
    assert_eq!(m.state(), State::High);

    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"state": "High", "context": {"cycles": 0}})
    );

    let mut restored: serde_hierarchy::Dimmer = serde_json::from_value(json).unwrap();
    assert_eq!(restored.state(), State::High);
    restored.process_event(Event::Down).unwrap();
    assert_eq!(restored.state(), State::Low);
}
