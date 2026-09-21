pub async fn run_inner(x: u32) -> u32 {
    x
}

pub fn finish(x: u32) -> u32 {
    x
}

/// Passed around as a value, never called directly.
pub fn dispatch_handler(x: u32) -> u32 {
    x
}
