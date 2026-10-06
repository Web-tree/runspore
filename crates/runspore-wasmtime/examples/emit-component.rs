//! Writes the embedded kernel component to a file and prints its kernel digest.
//!
//! `cargo run -p runspore-wasmtime --example emit-component -- <path>`

use std::process::ExitCode;

use runspore_types::digest;
use runspore_wasmtime::COMPONENT;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: emit-component <path>");
        return ExitCode::from(2);
    };
    if let Err(error) = std::fs::write(&path, COMPONENT) {
        eprintln!("writing {}: {error}", path.to_string_lossy());
        return ExitCode::FAILURE;
    }
    println!("{}", digest::hash("kernel", &[COMPONENT]));
    ExitCode::SUCCESS
}
