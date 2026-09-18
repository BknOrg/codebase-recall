pub struct Meter {
    ticks: u64,
}

impl Meter {
    pub fn new() -> Meter {
        Meter { ticks: 0 }
    }

    pub fn walk(&self) -> u64 {
        self.ticks
    }

    pub fn now(&self) -> u64 {
        self.ticks
    }

    pub fn default() -> u32 {
        0
    }
}
