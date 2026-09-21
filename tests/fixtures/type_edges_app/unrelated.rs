// Deliberately NOT importing crate::config. `Command` here means the standard
// library's, exactly as `std::process::Command` does across a real workspace.
use std::process::Command;

pub fn spawn_tool(cmd: &Command) -> u32 {
    let _ = cmd;
    0
}
