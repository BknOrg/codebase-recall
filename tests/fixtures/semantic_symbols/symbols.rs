pub union Bits { pub integer: u32, pub real: f32 }
pub enum State { Ready, Done }
pub const SIZE: usize = 4;
pub struct Engine;
impl Engine { pub fn new() -> Self { Self } pub fn run(&self) {} }
