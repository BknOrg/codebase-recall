mod gauge;
mod meter;

use gauge::Gauge;
use meter::Meter;

fn stamp() -> u64 {
    let _t = std::time::SystemTime::now();
    let _d: u32 = Default::default();
    0
}

fn probe(g: &Gauge) -> u32 {
    let _cursor = g.walk();
    g.reading()
}

fn main() {
    stamp();
    let g = Gauge::new();
    probe(&g);
    let _m = Meter::new();
}
