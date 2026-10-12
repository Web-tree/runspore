"""Shared settings and helpers for the build-0-2 step scripts.

Every setting has an environment override, so tests can point the scripts at a
scratch database, scratch repositories and fake `gh` and `claude` binaries.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # the orchestrator checkout
STATE = Path(os.environ.get("BUILD_STATE", ROOT / ".spore/build-state"))
# The spore this build runs on, and its database: set by `build --upgrade`
# (state file `engine.json`), else a fixed spore 0.1.
_ENGINE = json.loads((STATE / "engine.json").read_text()) if (STATE / "engine.json").exists() else {}
SPORE = os.environ.get("SPORE") or _ENGINE.get("spore") or str(Path.home() / ".local/share/runspore-0.1/bin/spore")
DB = os.environ.get("RUNSPORE_DB") or _ENGINE.get("db") or str(ROOT / ".spore/runspore.db")
GEN = ROOT / ".spore/gen"
# `bin/build` sets this to the workflow folders when the spore reads them, else
# to the 0.1 documents converted into GEN.
WORKFLOWS = Path(os.environ.get("BUILD_WORKFLOWS", GEN))
HOME = Path(os.environ.get("BUILD_HOME", Path.home() / "git/webtree/build-0-2")).expanduser()
REPOS = json.loads(os.environ.get("BUILD_REPOS", json.dumps({
    "Web-tree/runspore": "~/git/webtree/runspore",
    "Max-Levitskiy/skills": "~/git/Max-Levitskiy/skills",
})))
POLL_S = float(os.environ.get("BUILD_POLL_S", "60"))
BUILD_KEY = "build-0-2"


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
    """What `spore start` takes for a workflow: its folder, or its converted 0.1 document."""
    return WORKFLOWS / name if (WORKFLOWS / name).is_dir() else WORKFLOWS / f"{name}.json"


def runs():
    return spore("list")["runs"]


def build_try(key):
    """`build-0-2` is try 1, `build-0-2:2` try 2; None for any other key."""
    if key == BUILD_KEY:
        return 1
    if key.startswith(BUILD_KEY + ":") and key[len(BUILD_KEY) + 1:].isdigit():
        return int(key[len(BUILD_KEY) + 1:])
    return None


def build_runs():
    """(try, run) for every run of the build, oldest first."""
    return sorted((build_try(r["startKey"]), r) for r in runs() if build_try(r["startKey"]))


def build_run_id():
    b = build_runs()
    if not b:
        sys.exit(f"no build run yet; start it with work/build-0-2/bin/build")
    return b[-1][1]["runId"]


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
