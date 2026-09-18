pub struct Walker {
    depth: u32,
}

impl Walker {
    pub fn walk(&self) {
        self.step();
    }

    pub fn step(&self) {
        let _ = self.depth;
    }

    pub fn run(&self) {
        self.walk();
    }
}
