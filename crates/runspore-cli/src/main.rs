//! `spore`, the Runspore command line: the engine, the SQLite store and the kernel component in one process.
//! It never listens on a network. Commands, options, exit codes and JSON output follow
//! `spec/cli.md`.

mod cli;
mod project;
mod view;

use std::process::ExitCode;

use clap::Parser;
use serde_json::{json, Value};

use cli::{Cli, Fail, FORMAT};

fn main() -> ExitCode {
    if let Some(code) = runspore_host::guard::main_if_requested() {
        return code;
    }
    let json_mode = std::env::args().any(|a| a == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) if json_mode => {
            let message = e.kind().to_string();
            return report_error(true, &Fail::usage("usage", message));
        }
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return report_error(json_mode, &Fail::internal("runtime", e.to_string())),
    };
    match runtime.block_on(cli::execute(&cli)) {
        Ok(out) => {
            if cli.json {
                let mut object = out.json;
                if let Value::Object(map) = &mut object {
                    map.insert("format".into(), json!(FORMAT));
                }
                println!("{object}");
            } else if let Some(raw) = &out.raw {
                use std::io::Write;
                let mut stdout = std::io::stdout().lock();
                if let Err(e) = stdout.write_all(raw).and_then(|()| stdout.flush()) {
                    return report_error(false, &Fail::internal("stdout", e.to_string()));
                }
            } else if !out.text.is_empty() {
                println!("{}", out.text.trim_end());
            }
            ExitCode::from(out.exit)
        }
        Err(fail) => report_error(cli.json, &fail),
    }
}

/// In JSON mode an error is one object on stdout; otherwise one line on stderr.
fn report_error(json_mode: bool, fail: &Fail) -> ExitCode {
    if json_mode {
        let mut error = json!({"code": fail.code, "message": fail.message});
        if let Some(details) = &fail.details {
            error["details"] = details.clone();
        }
        let object = json!({"format": FORMAT, "error": error});
        println!("{object}");
    } else {
        eprintln!("spore: {}: {}", fail.code, fail.message);
    }
    ExitCode::from(fail.exit)
}
