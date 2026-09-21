mod config;
mod runner;
mod unrelated;

fn main() {
    let args = config::RunArgs {
        mode: config::Mode::Fast,
    };
    let _ = runner::execute(&args);
}
