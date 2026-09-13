mod scheduler;
mod worker_a;
mod worker_b;

use worker_a::RetryA;
use worker_b::RetryB;

struct Job {
    a: RetryA,
    b: RetryB,
}

impl Job {
    fn start(&self) {
        self.a.run();          // harus -> worker_a.rs::run (lewat field type RetryA)
        self.b.run();          // harus -> worker_b.rs::run (lewat field type RetryB)
        scheduler::run();      // bare path call ke fungsi bebas

        // `tick` di sini adalah closure LOKAL, bukan scheduler::tick, walau
        // scheduler::tick juga ada dan bisa diakses lewat scheduler::tick().
        let tick = || eprintln!("TRACE_CALL: main::local_tick_closure");
        tick();
    }
}

fn main() {
    let j = Job { a: RetryA, b: RetryB };
    j.start();
}
