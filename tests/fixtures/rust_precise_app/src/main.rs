mod app;
mod decoy;
mod real;

fn main() {
    let total = app::run() + app::poke(decoy::Decoy);
    println!("{total}");
}
