mod job;
mod scheduler;
mod worker;

fn main() {
    let j = job::Job::default();
    j.start();
}
