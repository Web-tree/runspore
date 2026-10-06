//! Rendering of store views as CLI JSON objects and text. Counters and timestamps are
//! decimal strings.

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

/// The `status` object of spec/cli.md, plus `startKey` and the latest result per node.
pub fn status_json(view: &RunView) -> Value {
    let state = state(view);
    let field = |f: fn(&State) -> Value| state.as_ref().map_or(Value::Null, f);
    json!({
        "runId": view.key.run,
        "startKey": view.start_key,
        "status": view.status,
        "revision": view.revision.to_string(),
        "position": field(|s| json!(s.position)),
        "invocation": field(|s| json!(s.invocation)),
        "result": field(|s| json!(s.result)),
        "nodes": field(|s| json!(s.nodes)),
        "quarantine": view.quarantine.as_ref().map(|q| json!({
            "code": q.code, "details": q.details, "atMs": q.at_ms.to_string()})),
    })
}

pub fn status_text(view: &RunView) -> String {
    let s = status_json(view);
    let mut text = format!(
        "run        {}\nstart key  {}\nstatus     {}\nrevision   {}\n",
        view.key.run, view.start_key, view.status, view.revision
    );
    if let Some(p) = s["position"].as_object() {
        text += &format!("position   {} ({})\n", p["nodeId"], p["activationId"]);
    }
    if let Some(i) = s["invocation"].as_object() {
        text += &format!(
            "invocation {} attempt {} {}\n",
            i["invocationId"], i["attempt"], i["state"]
        );
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
