//! The guard on its own, for programs that embed the engine but cannot run its hidden
//! mode themselves, such as test binaries. `spore` carries the same mode.

use std::process::ExitCode;

fn main() -> ExitCode {
    runspore_host::guard::main_if_requested().unwrap_or_else(|| {
        eprintln!("runspore-guard: started by the engine only");
        ExitCode::from(2)
    })
}
