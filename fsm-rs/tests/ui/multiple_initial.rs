use fsm_rs::state_machine;

struct Ctx;

state_machine! {
    name: M,
    context: Ctx,

    states: { *A, *B },
    events: { Go },

    transitions: {
        A + Go => B,
        B + Go => A,
    }
}

fn main() {}
