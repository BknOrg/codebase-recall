//! `Retry` has a method also called `run` — a name collision with
//! `scheduler::run` that name-only matching gets wrong.

#[derive(Default)]
pub struct Retry {
    attempts: u32,
}

impl Retry {
    pub fn run(&self) {
        println!("retry {}", self.attempts);
    }
}
