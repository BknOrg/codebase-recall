mod a;
mod b;

fn main() {
    let x = a::Walker { depth: 1 };
    x.run();
    let y = b::Walker { depth: 2 };
    y.run();
}
