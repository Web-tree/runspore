//! The handshake between a host and a workflow (spec/host.md §2, The handshake): what
//! the host offers, and how an action falls short of it.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The kind of an action an actor outside the worker does.
pub const EXTERNAL: &str = "external";
/// The kind of an action the worker runs as a program; with `async`, an async command.
pub const COMMAND: &str = "command";
/// The kind of an action whose start, probe and stop are a step runner, a WebAssembly
/// component sealed into the workflow (spec/host.md §3, `component`). Always async.
pub const COMPONENT_KIND: &str = "component";
/// The WIT world a step runner is a component of, with its version.
pub const RUNNER_WORLD: &str = "runspore:runner/runner@0.1.0";
/// The result channel that writes through the store: `step complete` and `step fail`.
pub const STORE_CHANNEL: &str = "store";
/// The async programs an action may declare, in the order they are checked.
pub const PROGRAMS: [&str; 3] = ["probe", "watch", "stop"];
/// The form of a program no host offers yet.
pub const COMPONENT: &str = "component";

/// What a host offers: the action kinds it runs, those it runs async, the result
/// channels an async action can report through, and the worlds of the step runners it
/// runs. Each list is sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCapabilities {
    pub action_kinds: Vec<String>,
    pub async_kinds: Vec<String>,
    pub result_channels: Vec<String>,
    #[serde(default)]
    pub component_worlds: Vec<String>,
}

/// One way an action asks for more than the host offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortfall {
    /// Its kind has no runner.
    Kind { kind: String, offered: Vec<String> },
    /// It carries `async`, and its kind does not run async here.
    NotAsync { kind: String, offered: Vec<String> },
    /// Its `async` program has the `component` form.
    Component { program: &'static str },
    /// None of its result channels is offered.
    NoChannel {
        listed: Vec<String>,
        offered: Vec<String>,
    },
    /// The channel the run recorded is not offered.
    ChannelGone {
        channel: String,
        offered: Vec<String>,
    },
}

impl Shortfall {
    /// The code a refusal at `validate_workflow` or `start` carries.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Kind { .. } => "action.kind-unregistered",
            Self::NotAsync { .. } => "action.async-unsupported",
            Self::Component { .. } => "action.component-unsupported",
            Self::NoChannel { .. } | Self::ChannelGone { .. } => "action.no-result-channel",
        }
    }
}

fn quoted(items: &[String]) -> String {
    if items.is_empty() {
        return "none".to_string();
    }
    let items: Vec<String> = items.iter().map(|i| format!("`{i}`")).collect();
    items.join(", ")
}

/// What the host lacks and what to configure, as one sentence.
impl fmt::Display for Shortfall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kind { kind, offered } => write!(
                f,
                "this host has no runner for kind `{kind}`; it runs {}: \
                 use a host that registers one, or one of those kinds",
                quoted(offered)
            ),
            Self::NotAsync { kind, offered } => write!(
                f,
                "this host does not run kind `{kind}` async; it runs {} async: \
                 remove `async`",
                quoted(offered)
            ),
            Self::Component { program } => write!(
                f,
                "this host runs no `{COMPONENT}` {program}: \
                 write it as {{\"command\": {{\"argv\": […]}}}}"
            ),
            Self::NoChannel { listed, offered } => write!(
                f,
                "it lists result channels {}; this host offers {}: \
                 add one of them to `async.channels`",
                quoted(listed),
                quoted(offered)
            ),
            Self::ChannelGone { channel, offered } => write!(
                f,
                "the run reports through result channel `{channel}`; this host offers {}: \
                 use a host that offers it",
                quoted(offered)
            ),
        }
    }
}

/// An action's `async` object, or `None`.
fn async_spec(action: &Value) -> Option<&Value> {
    action.get("async").filter(|a| !a.is_null())
}

/// Whether the action is async: `external` or `component`, or carrying `async`.
pub fn is_async(action: &Value) -> bool {
    [EXTERNAL, COMPONENT_KIND].contains(&kind(action)) || async_spec(action).is_some()
}

fn kind(action: &Value) -> &str {
    action
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// An async action's result channels, in order of preference: `async.channels`, or
/// `["store"]`. Entries that are not strings are skipped; the settings schema refuses
/// them first.
pub fn channels(action: &Value) -> Vec<String> {
    match async_spec(action).and_then(|a| a.get("channels")) {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => vec![STORE_CHANNEL.to_string()],
    }
}

impl HostCapabilities {
    /// The set of a host whose registry runs `kinds`: those and `external`; `external`
    /// async, `command` when it runs commands (an async command) and `component` when
    /// it runs step runners; the `store` channel; the runner world with `component`.
    pub fn of<'a>(kinds: impl IntoIterator<Item = &'a str>) -> Self {
        let mut action_kinds: Vec<String> = kinds.into_iter().map(String::from).collect();
        action_kinds.push(EXTERNAL.to_string());
        action_kinds.sort();
        action_kinds.dedup();
        let async_kinds = action_kinds
            .iter()
            .filter(|k| [COMMAND, COMPONENT_KIND, EXTERNAL].contains(&k.as_str()))
            .cloned()
            .collect();
        let component_worlds = action_kinds
            .iter()
            .filter(|k| *k == COMPONENT_KIND)
            .map(|_| RUNNER_WORLD.to_string())
            .collect();
        Self {
            action_kinds,
            async_kinds,
            result_channels: vec![STORE_CHANNEL.to_string()],
            component_worlds,
        }
    }

    /// Every way the action falls short, in the order of spec/host.md §2: its kind, its
    /// async use, its programs' forms, its channels.
    pub fn shortfalls(&self, action: &Value) -> Vec<Shortfall> {
        let kind = kind(action);
        let mut found = Vec::new();
        if !self.action_kinds.iter().any(|k| k == kind) {
            found.push(Shortfall::Kind {
                kind: kind.to_string(),
                offered: self.action_kinds.clone(),
            });
        }
        if !is_async(action) {
            return found;
        }
        if let Some(spec) = async_spec(action) {
            if !self.async_kinds.iter().any(|k| k == kind) {
                found.push(Shortfall::NotAsync {
                    kind: kind.to_string(),
                    offered: self.async_kinds.clone(),
                });
            }
            for program in PROGRAMS {
                if spec.get(program).and_then(|p| p.get(COMPONENT)).is_some() {
                    found.push(Shortfall::Component { program });
                }
            }
        }
        if self.pick(action).is_none() {
            found.push(Shortfall::NoChannel {
                listed: channels(action),
                offered: self.result_channels.clone(),
            });
        }
        found
    }

    /// The channel `start` records for an async action: the first of its channels the
    /// host offers.
    pub fn pick(&self, action: &Value) -> Option<String> {
        channels(action)
            .into_iter()
            .find(|c| self.result_channels.contains(c))
    }

    /// Whether the host can act on a step of `action` whose run recorded `channel`
    /// (`None`: nothing recorded, which counts `store`). The first shortfall, if any; the
    /// recorded channel is checked, never a new pick.
    pub fn recheck(&self, action: &Value, channel: Option<&str>) -> Option<Shortfall> {
        let shortfall = self
            .shortfalls(action)
            .into_iter()
            .find(|s| !matches!(s, Shortfall::NoChannel { .. }));
        if shortfall.is_some() || !is_async(action) {
            return shortfall;
        }
        let channel = channel.unwrap_or(STORE_CHANNEL);
        (!self.result_channels.iter().any(|c| c == channel)).then(|| Shortfall::ChannelGone {
            channel: channel.to_string(),
            offered: self.result_channels.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spore() -> HostCapabilities {
        HostCapabilities::of(["native", "command"])
    }

    #[test]
    fn the_set_adds_external_and_sorts() {
        let set = spore();
        assert_eq!(set.action_kinds, ["command", "external", "native"]);
        assert_eq!(set.async_kinds, ["command", "external"]);
        assert_eq!(set.result_channels, ["store"]);
        let no_commands = HostCapabilities::of(["native"]);
        assert_eq!(no_commands.async_kinds, ["external"]);
        assert!(set.component_worlds.is_empty());
        let runners = HostCapabilities::of(["command", "component"]);
        assert_eq!(runners.async_kinds, ["command", "component", "external"]);
        assert_eq!(runners.component_worlds, [RUNNER_WORLD]);
        assert!(runners
            .shortfalls(&json!({"kind": "component", "module": "r.wasm"}))
            .is_empty());
        assert!(is_async(&json!({"kind": "component"})));
    }

    #[test]
    fn a_plain_action_falls_short_only_by_its_kind() {
        let set = spore();
        assert!(set.shortfalls(&json!({"kind": "command"})).is_empty());
        let found = set.shortfalls(&json!({"kind": "wasm"}));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code(), "action.kind-unregistered");
        assert!(found[0].to_string().contains("`wasm`"), "{}", found[0]);
    }

    #[test]
    fn async_use_programs_and_channels_are_checked() {
        let set = spore();
        let action = json!({"kind": "native", "async": {
            "probe": {"component": {"ref": "./p.wasm", "export": "probe"}},
            "watch": {"command": {"argv": ["w"]}},
            "channels": ["nats"]}});
        let codes: Vec<&str> = set
            .shortfalls(&action)
            .iter()
            .map(Shortfall::code)
            .collect();
        assert_eq!(
            codes,
            [
                "action.async-unsupported",
                "action.component-unsupported",
                "action.no-result-channel"
            ]
        );
        let external = json!({"kind": "external", "async": {"channels": ["nats", "store"]}});
        assert!(set.shortfalls(&external).is_empty());
        let command = json!({"kind": "command", "async": {"timeoutMs": "60000"}});
        assert!(set.shortfalls(&command).is_empty());
        assert_eq!(set.pick(&external).as_deref(), Some("store"));
        assert_eq!(
            set.pick(&json!({"kind": "external"})).as_deref(),
            Some("store")
        );
        assert_eq!(
            set.pick(&json!({"kind": "external", "async": {"channels": []}})),
            None
        );
    }

    #[test]
    fn a_recheck_uses_the_recorded_channel() {
        let set = spore();
        let external = json!({"kind": "external"});
        assert_eq!(set.recheck(&external, None), None);
        assert_eq!(set.recheck(&external, Some("store")), None);
        let gone = set
            .recheck(&external, Some("nats"))
            .expect("nats is not offered");
        assert!(gone.to_string().contains("`nats`"), "{gone}");
        assert_eq!(set.recheck(&json!({"kind": "command"}), Some("nats")), None);
        let lost = HostCapabilities::of(["command"]);
        assert_eq!(
            lost.recheck(&json!({"kind": "native"}), None)
                .map(|s| s.code()),
            Some("action.kind-unregistered")
        );
    }
}
