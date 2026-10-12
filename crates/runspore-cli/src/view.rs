//! Rendering of store views as CLI JSON objects and text. Counters and timestamps are
//! decimal strings.

use runspore_host::{Arrival, Arrow};
use runspore_types::model::State;
use runspore_types::store::{EventRecord, RunView};
use serde_json::{json, Value};

/// The exit code a parked run maps to: quarantine first, then the run status.
pub fn exit_for(view: &RunView) -> u8 {
    if view.quarantine.is_some() {
        return 5;
    }
    match view.status.as_str() {
        "completed" => 0,
        "failed" => 1,
        "waiting" => 3,
        "needs-intervention" => 4,
        _ => 10,
    }
}

fn state(view: &RunView) -> Option<State> {
    serde_json::from_slice(view.snapshot.as_deref()?).ok()
}

fn arrow_json(arrow: Option<&Arrow>) -> Value {
    arrow.map_or(
        Value::Null,
        |a| json!({"nodeId": a.node_id, "outcome": a.outcome}),
    )
}

/// The `maxAttempts` of an action of a canonical package: its retry policy's, else 1.
fn max_attempts(package: &Value, action_id: &str) -> Value {
    let declared = package
        .pointer(&format!("/actions/{action_id}/retry/maxAttempts"))
        .filter(|n| n.is_u64());
    declared.cloned().unwrap_or(json!(1))
}

/// The `status` object of spec/cli.md, plus `startKey` and the latest result per node.
/// With the run's `package`, the invocation carries its action's `maxAttempts`.
pub fn status_json(view: &RunView, arrival: &Arrival, package: Option<&Value>) -> Value {
    let state = state(view);
    let field = |f: fn(&State) -> Value| state.as_ref().map_or(Value::Null, f);
    let mut position = field(|s| json!(s.position));
    if let Some(position) = position.as_object_mut() {
        position.insert("from".into(), arrow_json(arrival.from.as_ref()));
    }
    let mut invocation = field(|s| json!(s.invocation));
    if let (Some(package), Some(i)) = (package, invocation.as_object_mut()) {
        let action_id = i["actionId"].as_str().unwrap_or_default().to_string();
        i.insert("maxAttempts".into(), max_attempts(package, &action_id));
    }
    json!({
        "runId": view.key.run,
        "startKey": view.start_key,
        "status": view.status,
        "revision": view.revision.to_string(),
        "position": position,
        "invocation": invocation,
        "result": field(|s| json!(s.result)),
        "end": arrival.end.as_ref().map(|end| json!({
            "nodeId": end.node_id, "from": arrow_json(end.from.as_ref())})),
        "nodes": field(|s| json!(s.nodes)),
        "quarantine": view.quarantine.as_ref().map(|q| json!({
            "code": q.code, "details": q.details, "atMs": q.at_ms.to_string()})),
    })
}

/// `node on outcome`, the way an arrow reads in text.
fn arrow_text(arrow: &Value) -> String {
    match (arrow["nodeId"].as_str(), arrow["outcome"].as_str()) {
        (Some(node), Some(outcome)) => format!(", from {node} on {outcome}"),
        _ => String::new(),
    }
}

pub fn status_text(view: &RunView, arrival: &Arrival, package: Option<&Value>) -> String {
    let s = status_json(view, arrival, package);
    let mut text = format!(
        "run        {}\nstart key  {}\nstatus     {}\nrevision   {}\n",
        view.key.run, view.start_key, view.status, view.revision
    );
    if let Some(p) = s["position"].as_object() {
        text += &format!(
            "position   {} ({}{})\n",
            p["nodeId"].as_str().unwrap_or_default(),
            p["activationId"].as_str().unwrap_or_default(),
            arrow_text(&p["from"])
        );
    }
    if let Some(end) = s["end"].as_object() {
        text += &format!(
            "end        {}{}\n",
            end["nodeId"].as_str().unwrap_or_default(),
            arrow_text(&end["from"])
        );
    }
    if let Some(i) = s["invocation"].as_object() {
        let of = i
            .get("maxAttempts")
            .map_or_else(String::new, |m| format!(" of {m}"));
        text += &format!(
            "invocation {} attempt {}{of} {}\n",
            i["invocationId"].as_str().unwrap_or_default(),
            i["attempt"],
            i["state"].as_str().unwrap_or_default()
        );
        if let Some(error) = i.get("error") {
            text += &format!(
                "error      {}: {}\n",
                error["code"].as_str().unwrap_or_default(),
                error["message"].as_str().unwrap_or_default()
            );
        }
    }
    if !s["result"].is_null() {
        text += &format!("result     {}\n", s["result"]);
    }
    if let Some(q) = &view.quarantine {
        text += &format!("quarantine {}: {}\n", q.code, q.details);
    }
    text
}

pub fn event_json(record: &EventRecord) -> Value {
    let event = &record.event;
    let payload: Value = serde_json::from_slice(&event.body).unwrap_or(Value::Null);
    let diagnostics: Vec<Value> = record
        .diagnostics
        .iter()
        .map(|d| {
            json!({"code": d.code, "nodeId": d.node_id,
                   "details": serde_json::from_slice::<Value>(&d.details).unwrap_or(Value::Null)})
        })
        .collect();
    json!({
        "sequence": event.sequence.to_string(),
        "id": event.id,
        "kind": event.kind,
        "acceptedAtMs": event.accepted_at_ms.to_string(),
        "consumedRevision": record.consumed_revision.map(|r| r.to_string()),
        "diagnostics": diagnostics,
        "payload": payload,
    })
}

pub fn event_text(record: &EventRecord) -> String {
    let event = &record.event;
    let revision = record
        .consumed_revision
        .map_or_else(|| "pending".to_string(), |r| format!("rev {r}"));
    let codes: Vec<&str> = record.diagnostics.iter().map(|d| d.code.as_str()).collect();
    format!(
        "{:>4}  {:<22} {:<10} {}  {}",
        event.sequence,
        event.kind,
        revision,
        event.id,
        codes.join(",")
    )
}
