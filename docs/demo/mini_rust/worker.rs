#[derive(Default)]
pub struct Retry;

impl Retry {
    pub fn run(&self) {
        println!("Retry::run");
    }
}
