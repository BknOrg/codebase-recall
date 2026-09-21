/// Only ever used as a parameter and a field type — never called. Before type
/// references existed this looked like dead code to the whole tool suite.
pub struct RunArgs {
    pub mode: Mode,
}

pub enum Mode {
    Fast,
    Careful,
}

pub struct RunError;

/// Shares its name with `std::process::Command` on purpose.
pub enum Command {
    Run(RunArgs),
}
