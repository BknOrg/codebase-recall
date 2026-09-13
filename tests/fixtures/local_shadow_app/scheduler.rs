pub fn run() {
    eprintln!("TRACE_CALL: scheduler::run");
}

pub fn tick() {
    eprintln!("TRACE_CALL: scheduler::tick (SHOULD NEVER FIRE IN THIS TEST)");
}
