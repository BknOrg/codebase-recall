use crate::scheduler;
use crate::worker::Retry;

#[derive(Default)]
pub struct Job {
    retry: Retry,
}

impl Job {
    pub fn start(&self) {
        // Bare path call: the free function in `scheduler.rs`.
        scheduler::run();
        // Field method call: resolved through `retry: Retry` -> `Retry::run`.
        self.retry.run();
    }
}
