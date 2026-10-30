//! A test-only step runner (spec/host.md §3, `component`): a component of world
//! `runner` whose `start` does what the step's input asks, so host tests can drive
//! every host import and every ending. Built for `wasm32-unknown-unknown` by
//! `runspore-wasmtime`'s build script; on any other target the crate is empty.
//!
//! `start` reads the input's first key it knows:
//!
//! | Input | `start` |
//! | --- | --- |
//! | `{"spawn": [argv…], "env": {…}, "cwd": "…"}` | starts the program; the handle is `{"process": …}` |
//! | `{"request": {"socket", "body"}}` | one request; the handle is `{"reply": "<text>"}` |
//! | `{"read": "<path>"}` | reads a file; the handle is `{"read": "<text>"}` |
//! | `{"list": "<path>"}` | lists a folder; the handle is `{"list": [names]}` |
//! | `{"write": {"path", "text", "inWork"}}` | writes the text as the file, `path` under the work folder when `inWork`; the handle is `{"wrote": "<path>"}` |
//! | `{"fail": {"code", "message", "retryable"}}` | returns that failure |
//! | `{"handle": "<text>"}` | returns the text as the handle as it is |
//! | `{"sleep": ms}` | sleeps, then returns `{}` |
//! | `{"trap": true}` / `{"spin": true}` | traps / never returns |
//!
//! `probe` answers `{"probe": "alive" | "dead"}` handles as they say, checks a
//! `{"process": …}` handle with `process-alive`, reads the path of a `{"read": …}`
//! handle, writes `probed` to the path of a `{"wrote": …}` handle and says `alive`
//! when it could, and says `unknown` of anything else. `stop` ends a `{"process": …}`
//! handle's group, SIGTERM then SIGKILL after 3 s.

#[cfg(target_arch = "wasm32")]
mod guest {
    use serde_json::{json, Value};

    use exports::runspore::runner::step::{self, Context, Failure, Holder};
    use runspore::runner::host::{self, Liveness, Process, Signal, Spawn};

    wit_bindgen::generate!({
        path: "../../contracts/wit/runner",
        world: "runner",
    });

    struct Runner;

    export!(Runner);

    fn failure(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            code: code.to_string(),
            message: message.into(),
            retryable: false,
        }
    }

    fn strings(value: Option<&Value>) -> Vec<String> {
        value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn process_json(p: &Process) -> Value {
        json!({"program": p.program, "host": p.host, "boot": p.boot,
               "pgid": p.pgid, "started": p.started})
    }

    fn process_of(handle: &Value) -> Option<Process> {
        let p = handle.get("process")?;
        Some(Process {
            program: p.get("program")?.as_str()?.to_string(),
            host: p.get("host")?.as_str()?.to_string(),
            boot: p.get("boot")?.as_str()?.to_string(),
            pgid: i32::try_from(p.get("pgid")?.as_i64()?).ok()?,
            started: p.get("started")?.as_str()?.to_string(),
        })
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn start(input: &Value, work_dir: Option<&str>) -> Result<Value, Failure> {
        let fail = |e: String| failure("runner.host-failed", e);
        if let Some(argv) = input.get("spawn") {
            let env = input
                .get("env")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let spawn = Spawn {
                argv: strings(Some(argv)),
                cwd: input.get("cwd").and_then(Value::as_str).map(String::from),
                env,
            };
            let process = host::start_process(&spawn).map_err(fail)?;
            return Ok(json!({"process": process_json(&process)}));
        }
        if let Some(request) = input.get("request") {
            let socket = request.get("socket").and_then(Value::as_str).unwrap_or("");
            let body = request.get("body").and_then(Value::as_str).unwrap_or("");
            let reply = host::request(socket, body.as_bytes(), 5_000).map_err(fail)?;
            return Ok(json!({"reply": text(reply)}));
        }
        if let Some(path) = input.get("read").and_then(Value::as_str) {
            let bytes = host::read_file(path).map_err(fail)?;
            return Ok(json!({"read": text(bytes)}));
        }
        if let Some(path) = input.get("list").and_then(Value::as_str) {
            return Ok(json!({"list": host::list_folder(path).map_err(fail)?}));
        }
        if let Some(write) = input.get("write") {
            let mut path = write
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if write.get("inWork").and_then(Value::as_bool) == Some(true) {
                path = format!("{}/{path}", work_dir.unwrap_or(""));
            }
            let text = write.get("text").and_then(Value::as_str).unwrap_or("");
            host::write_file(&path, text.as_bytes()).map_err(fail)?;
            return Ok(json!({"wrote": path}));
        }
        if let Some(f) = input.get("fail") {
            return Err(Failure {
                code: f.get("code").and_then(Value::as_str).unwrap_or("").into(),
                message: f
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                retryable: f.get("retryable").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        if let Some(ms) = input.get("sleep").and_then(Value::as_u64) {
            host::sleep(u32::try_from(ms).unwrap_or(u32::MAX));
            return Ok(json!({}));
        }
        if input.get("trap").is_some() {
            panic!("asked to trap");
        }
        if input.get("spin").is_some() {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        }
        Ok(json!({}))
    }

    impl step::Guest for Runner {
        fn start(context: Context) -> Result<String, Failure> {
            let input: Value = serde_json::from_str(&context.input).unwrap_or(Value::Null);
            if let Some(handle) = input.get("handle").and_then(Value::as_str) {
                return Ok(handle.to_string());
            }
            start(&input, context.work_dir.as_deref()).map(|handle| handle.to_string())
        }

        fn probe(_context: Context, handle: String) -> Holder {
            let handle: Value = serde_json::from_str(&handle).unwrap_or(Value::Null);
            match handle.get("probe").and_then(Value::as_str) {
                Some("alive") => return Holder::Alive,
                Some("dead") => return Holder::Dead,
                _ => {}
            }
            if let Some(path) = handle.get("read").and_then(Value::as_str) {
                return match host::read_file(path) {
                    Ok(_) => Holder::Alive,
                    Err(_) => Holder::Unknown,
                };
            }
            if let Some(path) = handle.get("wrote").and_then(Value::as_str) {
                return match host::write_file(path, b"probed") {
                    Ok(()) => Holder::Alive,
                    Err(_) => Holder::Unknown,
                };
            }
            match process_of(&handle).map(|p| host::process_alive(&p)) {
                Some(Liveness::Alive) => Holder::Alive,
                Some(Liveness::Dead) => Holder::Dead,
                _ => Holder::Unknown,
            }
        }

        fn stop(_context: Context, handle: String) -> Result<String, String> {
            let handle: Value = serde_json::from_str(&handle).unwrap_or(Value::Null);
            let Some(process) = process_of(&handle) else {
                return Err("the handle names no process".into());
            };
            if !host::signal_process(&process, Signal::Term)? {
                return Ok("gone".into());
            }
            for _ in 0..60 {
                if host::process_alive(&process) == Liveness::Dead {
                    return Ok(format!("stopped {}", process.pgid));
                }
                host::sleep(50);
            }
            host::signal_process(&process, Signal::Kill)?;
            Ok(format!("killed {}", process.pgid))
        }
    }
}
