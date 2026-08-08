use fsm_rs::state_machine;

struct Ctx;

state_machine! {
    name: M,
    context: Ctx,

    states: { *A, B },
    events: { Go, Stop },

    transitions: {
        A + Go => B,
        B + Stop => A,
    }
}

fn main() {}
