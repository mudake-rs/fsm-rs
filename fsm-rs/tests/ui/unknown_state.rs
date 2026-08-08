use fsm_rs::state_machine;

struct Ctx;

state_machine! {
    name: M,
    context: Ctx,

    states: { *A, B },
    events: { Go, Stop },

    transitions: {
        A + Go => Nowhere,
        B + Stop => A,
        A + Stop => _,
        B + Go => _,
    }
}

fn main() {}
