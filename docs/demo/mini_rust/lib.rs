//! One file, four resolution layers. Run:
//!   code-rcl graph --project docs/demo/mini_rust --format json -o /tmp/g.json
//! and look at the `calls` edges.

mod scheduler;
mod worker;

use crate::worker::Retry;

pub struct Job {
    retry: Retry,
    // a second field of a DIFFERENT type that also has a `run` method
    fallback: worker::Retry,
}

impl Job {
    pub fn start(&self, scheduler: u32) {
        // L1 (scope): bare calls resolve in-file. `scheduler` as a bare name
        // would be the PARAMETER here, not the module -- shadowing is honored.
        helper();
        pick();

        // L2 (import binding): `crate::scheduler::run` is the free function in
        // scheduler.rs, NOT `Retry::run`, even though the method names collide.
        crate::scheduler::run();

        // L3 (type composition): `self.retry` has declared type `Retry`, so
        // `.run()` resolves to `Retry::run` exactly -- and `self.fallback.run()`
        // would resolve the same way through its own field type.
        self.retry.run();

        let _ = scheduler; // silence unused
    }
}

fn helper() {
    println!("helper");
}

fn pick() {
    println!("pick");
}
