"""Shared settings and helpers for the build-0-2 step scripts.

The settings have environment overrides, so tests can point the scripts at a
scratch database, scratch repositories and fake `gh` and `claude` binaries.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # the orchestrator checkout
STATE = Path(os.environ.get("BUILD_STATE", ROOT / ".spore/build-state"))
# The `spore` on PATH, the one the worker runs, and the run's database: spore gives
# every step `RUNSPORE_DB`; outside a step it is the project folder's.
SPORE = "spore"
DB = os.environ.get("RUNSPORE_DB") or str(ROOT / ".spore/runspore.db")
WORKFLOWS = ROOT / ".spore/workflows"
HOME = Path(os.environ.get("BUILD_HOME", Path.home() / "git/webtree/build-0-2")).expanduser()
REPOS = json.loads(os.environ.get("BUILD_REPOS", json.dumps({
    "Web-tree/runspore": "~/git/webtree/runspore",
    "Max-Levitskiy/skills": "~/git/Max-Levitskiy/skills",
})))
POLL_S = float(os.environ.get("BUILD_POLL_S", "60"))


def log(msg):
    print(f"[{Path(sys.argv[0]).name}] {msg}", file=sys.stderr, flush=True)


def sh(*args, cwd=None, check=True, input=None):
    """Run a command; return stdout. On failure raise with the stderr tail."""
    p = subprocess.run(args, cwd=cwd, input=input, capture_output=True, text=True)
    if check and p.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, args))} exited {p.returncode}: {p.stderr.strip()[-2000:]}")
    return p.stdout


def gh_json(*args):
    out = sh("gh", *args)
    return json.loads(out) if out.strip() else None


def spore(*args):
    return json.loads(sh(SPORE, "--db", DB, "--json", *args))


def workflow(name):
    """What `spore start` takes for a workflow: its folder."""
    return WORKFLOWS / name


def runs():
    return spore("list")["runs"]


def split_issue(issue):
    repo, num = issue.split("#")
    return repo, int(num)


def state_file(name):
    STATE.mkdir(parents=True, exist_ok=True)
    return STATE / f"{name}.json"


def load_state(name):
    f = state_file(name)
    return json.loads(f.read_text()) if f.exists() else {}


def save_state(name, data):
    f = state_file(name)
    tmp = f.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=2))
    tmp.replace(f)
