#!/usr/bin/env python3
"""Show where the build-0-2 run is.

Reads the workflow folder (.spore/workflows/build-0-2), the run itself from the
spore database (`spore status` of the run keyed `build-0-2` and of its ticket runs),
and the GitHub state of each ticket. Prints the position as JSON, or renders the
build-order page with the workflow drawn from its DOT and the position highlighted.

    python3 work/build-0-2/track.py            # status JSON
    python3 work/build-0-2/track.py --page OUT.html

Before the run is started it reads the position from GitHub alone: the first wave
with an open ticket. Once the plugin's live view (P7) lands, it draws this instead.
"""
import html, json, re, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
WF = REPO / ".spore/workflows/build-0-2"
sys.path.insert(0, str(HERE / "bin"))
from buildlib import DB, SPORE, STATE  # noqa: E402  the spore and database the build runs on

# Placeholder colours in the display DOT; the page's CSS maps them to theme tokens.
DONE, CURRENT, ARROW = "#010101", "#020202", "#030303"


def gh_json(*args):
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def load():
    dot = (WF / "workflow.dot").read_text()
    settings = json.loads((WF / "settings.json").read_text())
    return dot, settings


def spore(*args):
    out = subprocess.run([SPORE, "--db", DB, "--json", *args], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def edges(dot):
    """(from, to, on) for every arrow; no `on` means `ok`."""
    return [(a, b, (re.search(r'on="([^"]+)"', at or "") or [None, "ok"])[1])
            for a, b, at in re.findall(r"^\s*(\w+)\s*->\s*(\w+)\s*(\[[^\]]*\])?;", dot, re.M)]


def path_of(dot):
    """The main path: start node, then each node's `ok` or `go` arrow."""
    start = re.search(r'start\s*=\s*"([^"]+)"', dot).group(1)
    nxt = {}
    for a, b, attrs in re.findall(r"^\s*(\w+)\s*->\s*(\w+)\s*(\[[^\]]*\])?;", dot, re.M):
        on = re.search(r'on="([^"]+)"', attrs or "")
        on = on.group(1) if on else "ok"
        if on in ("ok", "go"):
            nxt[a] = (b, on)
    seq, node = [start], start
    while node in nxt:
        node = nxt[node][0]
        seq.append(node)
    return seq, nxt


def ticket_states(settings):
    """issue -> done (closed as completed), dropped (closed as not planned),
    in-progress (an open PR closes it), or open."""
    repos = {t["issue"].split("#")[0] for n in settings["nodes"].values()
             for t in n.get("input", {}).get("tickets", {}).get("$literal", [])}
    closed, in_pr = {}, set()
    for repo in repos:
        for i in gh_json("issue", "list", "-R", repo, "--state", "all", "--limit", "300",
                         "--json", "number,state,stateReason"):
            if i["state"] == "CLOSED":
                closed[f"{repo}#{i['number']}"] = "dropped" if i.get("stateReason") == "NOT_PLANNED" else "done"
        # Only GitHub's closing links count: a PR that merely mentions a ticket is not building it.
        for pr in gh_json("pr", "list", "-R", repo, "--state", "open", "--limit", "100",
                          "--json", "closingIssuesReferences"):
            for ref in pr.get("closingIssuesReferences") or []:
                m = re.search(r"github\.com/([\w.-]+/[\w.-]+)/issues/(\d+)", ref.get("url", ""))
                in_pr.add(f"{m.group(1)}#{m.group(2)}" if m else f"{repo}#{ref['number']}")
    def state(issue):
        return closed.get(issue) or ("in-progress" if issue in in_pr else "open")
    return state


def wave_tickets(settings, node):
    return settings["nodes"].get(node, {}).get("input", {}).get("tickets", {}).get("$literal", [])


def feedback_tickets(run_id):
    """Tickets made from `changes` answers in this run, from the step's own records."""
    out = {}
    for f in sorted(STATE.glob("feedback-*.json")) if STATE.exists() else []:
        for e in json.loads(f.read_text()).values():
            if e.get("run") != run_id:
                continue
            k = e["title"].split()[1].lower()
            out.setdefault(f"feedback_{k}", []).append(
                {"id": f"F{k.upper()}-{e['issue'].split('#')[1]}", "issue": e["issue"], "title": e["title"]})
    return out


def status():
    dot, settings = load()
    seq, _ = path_of(dot)
    state = ticket_states(settings)
    all_runs = spore("list")["runs"] if Path(DB).exists() else []
    tries = sorted((int(m.group(1) or 1), r) for r in all_runs
                   for m in [re.fullmatch(r"build-0-2(?::(\d+))?", r["startKey"])] if m)
    build = tries[-1][1] if tries else None
    if build is None:
        return planned(dot, settings, seq, state)
    run = spore("status", build["runId"])
    ticket_runs = {}
    for r in all_runs:
        m = re.match(r"ticket:(.+):(\d+)$", r["startKey"])
        if m and int(m.group(2)) >= ticket_runs.get(m.group(1), (0,))[0]:
            ticket_runs[m.group(1)] = (int(m.group(2)), r)

    def ticket(t):
        s = state(t["issue"])
        if s not in ("done", "dropped") and t["issue"] in ticket_runs:
            r = ticket_runs[t["issue"]][1]
            s = r["status"] if r["status"] in ("failed",) else spore("status", r["runId"])["position"]["nodeId"] or r["status"]
        return {**t, "state": s}

    pos = run["position"]["nodeId"] if run["status"] in ("running", "waiting") else None
    visited = run["nodes"]
    fb = feedback_tickets(build["runId"])
    nodes = {}
    order = seq + sorted(n for n in visited if n not in seq)
    for node in order:
        cfg = settings["nodes"].get(node, {})
        if node.startswith("feedback_") or wave_tickets(settings, node):
            ts = [ticket(t) for t in (fb.get(node, []) if node.startswith("feedback_") else wave_tickets(settings, node))]
            nodes[node] = {"kind": "wave", "tickets": ts, "done": node in visited and node != pos}
        elif "signal" in cfg:
            nodes[node] = {"kind": "checkpoint", "signal": cfg["signal"],
                           "answer": {"outcome": visited[node]["outcome"]} if node in visited else None,
                           "done": node in visited and node != pos}
        else:
            nodes[node] = {"kind": "end", "done": run["status"] == "completed"}
    frm = None
    if pos:
        came = [(a, on) for a, b, on in edges(dot) if b == pos and a in visited and visited[a]["outcome"] == on]
        if len(came) > 1 and run["position"]["visit"] > 1:
            came = [c for c in came if c[0].startswith("feedback_")] or came
        frm = {"nodeId": came[0][0], "outcome": came[0][1]} if came else None
    return {"format": "build-0-2.track/0.2", "workflow": "build-0-2", "runId": build["runId"],
            "status": run["status"], "position": {"nodeId": pos, "from": frm}, "path": order, "nodes": nodes}


def planned(dot, settings, seq, state):
    """No run yet: the first wave with an open ticket."""
    nodes, position, prev, frm = {}, None, None, None
    for node in seq:
        cfg = settings["nodes"].get(node, {})
        if wave_tickets(settings, node):
            ts = [{**t, "state": state(t["issue"])} for t in wave_tickets(settings, node)]
            done = all(t["state"] in ("done", "dropped") for t in ts)
            nodes[node] = {"kind": "wave", "tickets": ts, "done": done}
        elif "signal" in cfg:
            done = False
            nodes[node] = {"kind": "checkpoint", "signal": cfg["signal"], "answer": None, "done": False}
        else:
            done = False
            nodes[node] = {"kind": "end", "done": False}
        if position is None and not done:
            position = node
            frm = {"nodeId": prev, "outcome": "ok"} if prev else None
        prev = node
    return {"format": "build-0-2.track/0.2", "workflow": "build-0-2", "runId": None, "status": "not started",
            "position": {"nodeId": position, "from": frm}, "path": seq, "nodes": nodes}


def display_dot(dot, st):
    """The pinned DOT plus render-only attributes: arrow labels from `on`, done and current highlights."""
    def label(m):
        a, b, attrs = m.group(1), m.group(2), m.group(3) or ""
        on = re.search(r'on="([^"]+)"', attrs)
        extra = [f'label="{on.group(1)}"'] if on else []
        if st["position"]["from"] and a == st["position"]["from"]["nodeId"] and b == st["position"]["nodeId"]:
            extra += [f'color="{ARROW}"', f'fontcolor="{ARROW}"', "penwidth=2.5"]
        if not extra:
            return m.group(0)
        inner = attrs[1:-1].strip() if attrs else ""
        return f"  {a} -> {b} [{', '.join(filter(None, [inner] + extra))}];"
    out = re.sub(r"^\s*(\w+)\s*->\s*(\w+)\s*(\[[^\]]*\])?;", label, dot, flags=re.M)
    marks = [f'  {n} [style=filled, fillcolor="{DONE}"];' for n, v in st["nodes"].items() if v["done"]]
    if st["position"]["nodeId"]:
        marks.append(f'  {st["position"]["nodeId"]} [style="filled,bold", fillcolor="{CURRENT}", penwidth=2.5];')
    marks.append('  node [fontname="Helvetica", fontsize=11]; edge [fontname="Helvetica", fontsize=10];')
    return out.rstrip().rstrip("}") + "\n" + "\n".join(marks) + "\n}\n"


def svg(display):
    out = subprocess.run(["dot", "-Tsvg"], input=display, capture_output=True, text=True, check=True).stdout
    return out[out.index("<svg"):]


def page(out_path):
    dot, settings = load()
    st = status()
    template = (HERE / "page.html").read_text()
    pos = st["position"]["nodeId"]
    cur = st["nodes"].get(pos, {})
    if cur.get("kind") == "wave":
        n = sum(t["state"] in ("done", "dropped") for t in cur["tickets"])
        now = f"{pos.replace('_', ' ').capitalize()}: {n} of {len(cur['tickets'])} tickets finished"
    elif cur.get("kind") == "checkpoint":
        now = f"Checkpoint {pos[-1].upper()}: waiting for your answer"
    elif st["status"] == "failed":
        now = "The build run failed: start a new try with work/build-0-2/bin/build --restart"
    else:
        now = "0.2 shipped"
    waves = []
    for node in st["path"]:
        v = st["nodes"][node]
        here = " here" if node == pos else ""
        if v["kind"] == "wave":
            chips = "".join(
                f'<a class="chip {t["id"][0].lower()} {t["state"]}" href="https://github.com/{t["issue"].replace("#", "/issues/")}">'
                f'<span class="id">{t["id"]}</span><span class="nm">{html.escape(t["title"])}</span>'
                f'<span class="st">{t["state"].replace("-", " ")}</span></a>' for t in v["tickets"])
            waves.append(f'<div class="wave{here}"><div class="wl">{node.replace("_", " ")}</div><div class="chips">{chips}</div></div>')
        elif v["kind"] == "checkpoint":
            k = v["signal"].split("-")[-1]
            title = (WF / f"steps/checkpoint-{k}.md").read_text().splitlines()[0].split(": ", 1)[1]
            ans = v["answer"]["outcome"] if v["answer"] else "not answered"
            waves.append(f'<div class="cp{here}"><span class="cpk">{k.upper()}</span><span>{html.escape(title)}</span><span class="ans">{ans}</span></div>')
    steps = "".join(
        f'<details><summary>{p.name}</summary><pre>{html.escape(p.read_text())}</pre></details>'
        for p in sorted((WF / "steps").glob("*.md")))
    filled = (template.replace("{{NOW}}", html.escape(now)).replace("{{SVG}}", svg(display_dot(dot, st)))
              .replace("{{WAVES}}", "\n".join(waves)).replace("{{DOT}}", html.escape(dot))
              .replace("{{SETTINGS}}", html.escape((WF / "settings.json").read_text()))
              .replace("{{STEPS}}", steps).replace("{{STATUS}}", html.escape(json.dumps(
                  {k: st[k] for k in ("status", "position")}, indent=2))))
    Path(out_path).write_text(filled)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--page":
        page(sys.argv[2])
    else:
        print(json.dumps(status(), indent=2))
