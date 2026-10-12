#!/usr/bin/env python3
"""Track the build-0-2 workflow until spore 0.2 can run it itself.

Reads .spore/workflows/build-0-2 (the plan, written as a runspore 0.2 workflow),
the GitHub state of every ticket it names, and checkpoints.json (the answers given
at each checkpoint). Prints where the build is in the shape of `spore status --json`,
or renders the build-order page with the workflow drawn and the position highlighted.

    python3 work/build-0-2/track.py            # status JSON
    python3 work/build-0-2/track.py --page OUT.html

Once spore 0.2 has external steps (R19) and `continue` (R22), start a real run of
the workflow and read `spore status --json` instead; this script then retires.
"""
import html, json, re, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
WF = REPO / ".spore/workflows/build-0-2"

# Placeholder colours in the display DOT; the page's CSS maps them to theme tokens.
DONE, CURRENT, ARROW = "#010101", "#020202", "#030303"


def gh_json(*args):
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def load():
    dot = (WF / "workflow.dot").read_text()
    settings = json.loads((WF / "settings.json").read_text())
    checkpoints = json.loads((HERE / "checkpoints.json").read_text())
    return dot, settings, checkpoints


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
    repos = {t["issue"].split("#")[0] for n in settings["nodes"].values()
             for t in n.get("input", {}).get("tickets", {}).get("$literal", [])}
    closed, in_pr = {}, set()
    for repo in repos:
        for i in gh_json("issue", "list", "-R", repo, "--state", "all", "--limit", "300", "--json", "number,state"):
            closed[f"{repo}#{i['number']}"] = i["state"] == "CLOSED"
        for pr in gh_json("pr", "list", "-R", repo, "--state", "open", "--limit", "100", "--json", "title,body"):
            for m in re.finditer(r"(?:([\w.-]+/[\w.-]+))?#(\d+)", f"{pr['title']} {pr['body']}"):
                in_pr.add(f"{m.group(1) or repo}#{m.group(2)}")
    def state(issue):
        return "done" if closed.get(issue) else "in-progress" if issue in in_pr else "open"
    return state


def status():
    dot, settings, checkpoints = load()
    seq, nxt = path_of(dot)
    state = ticket_states(settings)
    nodes, position, prev, frm = {}, None, None, None
    # Feedback tickets made from a checkpoint's `changes` answer belong to the wave before it.
    extra = {}
    for a, b in zip(seq, seq[1:]):
        answer = checkpoints.get(settings["nodes"].get(b, {}).get("signal", ""), {})
        if answer.get("outcome") == "changes":
            extra[a] = [{"id": "fix", "issue": i, "title": "Checkpoint feedback"} for i in answer.get("tickets", [])]
    for node in seq:
        cfg = settings["nodes"].get(node, {})
        tickets = cfg.get("input", {}).get("tickets", {}).get("$literal", [])
        if tickets:
            ts = [{**t, "state": state(t["issue"])} for t in tickets + extra.get(node, [])]
            done = all(t["state"] == "done" for t in ts)
            nodes[node] = {"kind": "wave", "tickets": ts, "done": done}
        elif "signal" in cfg:
            answer = checkpoints.get(cfg["signal"], {})
            done = answer.get("outcome") == "go"
            nodes[node] = {"kind": "checkpoint", "signal": cfg["signal"], "answer": answer or None, "done": done}
        else:
            done = False
            nodes[node] = {"kind": "end", "done": False}
        if position is None and not done:
            position = node
            frm = {"nodeId": prev, "outcome": nxt[prev][1]} if prev else None
        prev = node
    return {"format": "build-0-2.track/0.1", "workflow": "build-0-2",
            "status": "completed" if position == seq[-1] else "running",
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
    dot, settings, _ = load()
    st = status()
    template = (HERE / "page.html").read_text()
    pos = st["position"]["nodeId"]
    cur = st["nodes"].get(pos, {})
    if cur.get("kind") == "wave":
        n = sum(t["state"] == "done" for t in cur["tickets"])
        now = f"{pos.replace('_', ' ').capitalize()}: {n} of {len(cur['tickets'])} tickets merged"
    elif cur.get("kind") == "checkpoint":
        now = f"{pos.replace('_', ' ').capitalize()}: waiting for your answer"
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
