//! The "real" free function that `job::start` means to call.

pub fn run() {
    tick();
}

fn tick() {
    println!("scheduler tick");
}
