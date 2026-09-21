use crate::config::RunArgs;
use crate::config::RunError;

pub trait Runner {
    fn run(&self) -> u32;
}

pub struct LocalRunner;

impl Runner for LocalRunner {
    fn run(&self) -> u32 {
        7
    }
}

/// `args` and the error type are the only mentions of either struct.
pub fn execute(args: &RunArgs) -> Result<u32, RunError> {
    let _ = args;
    Ok(0)
}

/// A generic placeholder must not be confused with a project type of the same
/// name, so this signature yields no edge to `Mode`.
pub fn passthrough<Mode>(value: Mode) -> Mode {
    value
}
