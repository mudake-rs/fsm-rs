//! Hierarchical (nested) states: `Active` is a composite state containing
//! `Charging` and `Discharging`. Watch the hook firing order in the output:
//!
//! * entering `Active` enters its initial child `Charging` (entry hooks fire
//!   outermost-first);
//! * sibling moves (`Charging -> Discharging`) never touch `Active`'s hooks;
//! * the composite row `Active + Unplug` matches from any child — exits fire
//!   innermost-first up to the boundary.
//!
//! Run with: `cargo run --example battery`

use fsm_rs::state_machine;

struct Log {
    lines: Vec<String>,
}

state_machine! {
    name: Battery,
    context: Log,

    states: {
        *Idle,
        Active(entry: enter_active, exit: exit_active) {
            *Charging(exit: exit_charging),
            Discharging(entry: enter_discharging),
        },
        Done,
    },
    events: { PlugIn, Unplug, Full, Empty },

    transitions: {
        Idle + PlugIn => Active,        // resolves to Active's initial: Charging
        Charging + Full => Discharging, // sibling move: no Active hooks
        Active + Unplug => Idle,        // composite row: matches from any child
        Charging + Empty => Done,       // cross-boundary
        Discharging + Empty => Done,
        Idle + Unplug | Full | Empty => _,
        Done + PlugIn | Unplug | Full | Empty => _,
        Charging + PlugIn => _,
        Discharging + PlugIn | Full => _,
    }

    on_transition: note,
}

impl BatteryContext for Log {
    fn enter_active(&mut self) {
        self.lines.push("    enter Active".into());
    }
    fn exit_active(&mut self) {
        self.lines.push("    exit Active".into());
    }
    fn exit_charging(&mut self) {
        self.lines.push("    exit Charging".into());
    }
    fn enter_discharging(&mut self) {
        self.lines.push("    enter Discharging".into());
    }
    fn note(&mut self, from: &State, to: &State, _event: &Event) {
        self.lines
            .push(format!("    on_transition: {from:?} -> {to:?}"));
    }
}

fn main() {
    let mut m = Battery::new(Log { lines: Vec::new() });
    let fire = |m: &mut Battery, event: Event, label: &str| {
        m.context_mut().lines.clear();
        m.process_event(event).unwrap();
        println!("{label}:");
        for line in &m.context().lines {
            println!("{line}");
        }
        println!("  => now in {:?}\n", m.state());
    };

    fire(&mut m, Event::PlugIn, "PlugIn (Idle -> Active)");
    fire(
        &mut m,
        Event::Full,
        "Full (Charging -> Discharging, sibling)",
    );
    fire(
        &mut m,
        Event::Unplug,
        "Unplug (composite row, from Discharging)",
    );
    fire(&mut m, Event::PlugIn, "PlugIn again");
    fire(
        &mut m,
        Event::Empty,
        "Empty (Charging -> Done, cross-boundary)",
    );
}
