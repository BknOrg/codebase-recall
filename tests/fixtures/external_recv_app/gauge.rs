pub struct Gauge {
    level: u32,
}

impl Gauge {
    pub fn new() -> Gauge {
        Gauge { level: 0 }
    }

    pub fn reading(&self) -> u32 {
        self.level
    }
}
