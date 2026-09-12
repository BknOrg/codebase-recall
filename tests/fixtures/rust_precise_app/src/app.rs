//! `h` gets its type from `Real::new()`, which no AST walk resolves. Scoring
//! then falls back to what this file has a `use` for — `Decoy` — and picks
//! `Decoy::handle`, which is the wrong `handle` entirely.

use crate::decoy::Decoy;

pub fn run() -> u32 {
    let h = crate::real::Real::new();
    h.handle()
}

pub fn poke(d: Decoy) -> u32 {
    d.handle()
}
