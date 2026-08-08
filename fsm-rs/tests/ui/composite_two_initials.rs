use fsm_rs::state_machine;

struct Ctx;

state_machine! {
    name: M,
    context: Ctx,

    states: { *A, P { *B, *C } },
    events: { Go },

    transitions: {
        _ + Go => _,
    }
}

fn main() {}
