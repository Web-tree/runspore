//! Drives the compiled `spore` binary as a child process, one temp directory each.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

pub const BIN: &str = env!("CARGO_BIN_EXE_spore");

pub struct Dir {
    pub path: PathBuf,
}

impl Dir {
    pub fn new(name: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("runspore-cli-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn write(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.file(name);
        std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        path
    }

    /// Writes `package` as the workflow folder `name` (spec/dot.md) and returns its path.
    pub fn folder(&self, name: &str, package: &Value) -> PathBuf {
        let path = self.file(name);
        std::fs::create_dir_all(&path).unwrap();
        let (dot, settings) = split(package);
        std::fs::write(path.join("workflow.dot"), dot).unwrap();
        std::fs::write(
            path.join("settings.json"),
            serde_json::to_vec_pretty(&settings).unwrap(),
        )
        .unwrap();
        path
    }

    /// A command in this directory on its own database, with an effects log, outside
    /// any step.
    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.current_dir(&self.path)
            .env("RUNSPORE_DB", self.file("runspore.db"))
            .env("EFFECTS_LOG", self.file("effects.log"))
            .env_remove("RUNSPORE_FAILPOINTS")
            .env_remove("RUNSPORE_RUN_ID")
            .env_remove("RUNSPORE_NODE_ID")
            .env_remove("RUNSPORE_PARENT_RUN_ID")
            .args(args)
            .stdin(Stdio::null());
        cmd
    }

    pub fn spawn(&self, args: &[&str]) -> Child {
        self.cmd(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Runs to exit; returns the status and stdout parsed as JSON when it is JSON.
    pub fn run(&self, args: &[&str]) -> Out {
        run(self.cmd(args))
    }

    pub fn effects(&self) -> Vec<Effect> {
        effects(&self.file("effects.log"))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub struct Out {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("not one JSON object ({e}): {}", self.stdout))
    }
}

pub fn run(mut cmd: Command) -> Out {
    let out = cmd.output().unwrap();
    Out {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A 0.2 package as the two files of its workflow folder: the name, start, kinds and
/// outcomes as `workflow.dot`, everything else as `settings.json`.
pub fn split(package: &Value) -> (String, Value) {
    let mut settings = package.clone();
    let top = settings.as_object_mut().unwrap();
    let (name, start) = (top.remove("name").unwrap(), top.remove("start").unwrap());
    let mut dot = format!("digraph {name} {{\n  dialect = \"runspore/0.2\";\n  start = {start};\n");
    let mut edges = String::new();
    let nodes = top.get_mut("nodes").unwrap().as_object_mut().unwrap();
    for (id, node) in nodes {
        let node = node.as_object_mut().unwrap();
        let kind = node.remove("kind").unwrap();
        dot += &format!("  \"{id}\" [kind={kind}];\n");
        if let Some(Value::Object(outcomes)) = node.remove("outcomes") {
            for (on, to) in outcomes {
                edges += &format!("  \"{id}\" -> {to} [on={}];\n", Value::String(on));
            }
        }
    }
    dot += &edges;
    dot += "}\n";
    (dot, settings)
}

/// One line of the effects log: what really executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub node: String,
    pub attempt: u32,
    pub key: String,
}

pub fn effects(path: &Path) -> Vec<Effect> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let mut parts = line.split(' ');
            Effect {
                node: parts.next().unwrap().to_string(),
                attempt: parts.next().unwrap().parse().unwrap(),
                key: parts.next().unwrap_or_default().to_string(),
            }
        })
        .collect()
}

/// The command that appends `<node> <attempt> <effect key>` to the effects log, then
/// runs `then`.
pub fn effect_argv(node: &str, then: &str) -> Value {
    let script = format!(
        "echo \"{node} $RUNSPORE_ATTEMPT $RUNSPORE_EFFECT_KEY\" >> \"$EFFECTS_LOG\"; {then}"
    );
    serde_json::json!(["/bin/sh", "-c", script])
}
