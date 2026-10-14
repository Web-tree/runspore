#!/usr/bin/env bash
# End-to-end test of the build-0-2 workflows with a real spore and fake gh and claude.
#
#   work/build-0-2/test/e2e.sh [scratch-dir]   # SPORE=<binary> to test that spore, else this
#                                              # checkout's spore is built; SPORE_NEW=<binary>
#                                              # is the newer spore it upgrades onto (default:
#                                              # a copy of SPORE)
#
# It drives the build the way the README does, with plain `spore` commands: start, worker,
# list, status, signal and resolve. The build runs from a scratch clone of this checkout
# (with its uncommitted changes), on the clone's own database `.spore/runspore.db`.
# Scratch git repos stand in for both GitHub repos (fake `gh pr merge` squash-merges into
# their bare origins). Checks: the fast lane runs, a step creates the labels, and
# checkpoint 0 asks; `go` runs waves 1-2, and wave 1 closes checkpoint 0's issue (R9 goes
# red once and is fixed; R10's implement step can't recover, is stuck, and `spore resolve
# --retry` gets it merged; R12's checks stay red past the fix loop's limit, its run waits
# at the stuck step, and `spore signal ... keep-going` gets it merged) and checkpoint A
# asks; spore refuses answers that do not fit its schema; a `changes` answer, sent twice
# with one message ID, builds two feedback tickets and asks again, with the issue showing
# the step's current text; `go` moves on to checkpoint B and closes A's issue; a newer
# spore's worker goes on with the same database and the same run, and its `go` on B
# closes B's issue and starts wave 4.
set -euo pipefail
# Run from inside a step (a ticket's test, say), RUNSPORE_DB names the real build's
# database. Set below to the scratch one, so nothing here reaches the caller's.
for v in $(compgen -e); do case "$v" in RUNSPORE_*) unset "$v" ;; esac; done
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
E="${1:-$(mktemp -d)}"
if [ -z "${SPORE:-}" ]; then
  cargo build --quiet --locked --manifest-path "$root/Cargo.toml" -p runspore-cli
  SPORE="${CARGO_TARGET_DIR:-$root/target}/debug/spore"
fi
rm -rf "$E" && mkdir -p "$E/spore-old" "$E/spore-new"
E="$(cd "$E" && pwd)"
# Two installs: the one the build starts on, and the newer one it moves onto.
cp "$SPORE" "$E/spore-old/spore"
cp "${SPORE_NEW:-$SPORE}" "$E/spore-new/spore"
for r in runspore skills; do
  git init -q --bare -b main "$E/bare-$r.git"
  git clone -q "$E/bare-$r.git" "$E/$r" 2>/dev/null
  (cd "$E/$r" && echo seed > README && git add README \
    && git -c user.name=seed -c user.email=seed@x commit -qm seed && git push -q origin main)
done
orch="$E/orch"
git clone -q "$root" "$orch"
git -C "$root" diff HEAD --binary | git -C "$orch" apply --allow-empty
git -C "$root" ls-files --others --exclude-standard -z | rsync -a --from0 --files-from=- "$root/" "$orch/"
path="$here/fakebin:$PATH"
export PATH="$E/spore-old:$path" RUNSPORE_DB="$orch/.spore/runspore.db"
export BUILD_STATE="$E/state" BUILD_HOME="$E/wt" BUILD_POLL_S=0.3
export BUILD_REPOS="{\"Web-tree/runspore\": \"$E/runspore\", \"Max-Levitskiy/skills\": \"$E/skills\"}"
export BUILD_CHECKS='{"Web-tree/runspore": [["bash", "-c", "test ! -f BROKEN"]], "Max-Levitskiy/skills": [["true"]]}'
export FAKE_GH_STATE="$E/gh.json"
export FAKE_CLAUDE_STUCK=R10 FAKE_CLAUDE_UNSTICK="$E/r10-unstick"
export FAKE_CLAUDE_RED=R12 FAKE_CLAUDE_UNRED="$E/r12-unred"
export FAKE_GH_BARES="{\"Web-tree/runspore\": \"$E/bare-runspore.git\", \"Max-Levitskiy/skills\": \"$E/bare-skills.git\"}"
cd "$orch"
worker=""
stop() { [ -n "$worker" ] && kill "$worker" 2>/dev/null; wait 2>/dev/null || true; }
trap stop EXIT
fail() { echo "FAIL: $*"; exit 1; }

# `py <code> [args]`: run <code> with the JSON on stdin as `d`.
py() { local code="$1"; shift; python3 -c "import json, sys; d = json.load(sys.stdin)
$code" "$@"; }
where() { spore --json status "$run" 2>/dev/null | py 'p = d["position"]
print(d["status"], p["nodeId"] if p else "-", p["visit"] if p else "-")' || echo "unknown"; }
wait_for() {  # status node visit
  for _ in $(seq 1 300); do
    now="$(where)"
    [ "$now" = "$*" ] && return 0
    case "$now" in failed*|completed*) fail "run is $now, wanted $*";; esac
    sleep 1
  done
  fail "still $now, wanted $*"
}
gh_is() { python3 -c "import json,sys; s=json.load(open('$FAKE_GH_STATE')); sys.exit(0 if ($1) else 1)"; }
check() { gh_is "$1" || fail "$1"; }
wait_gh() { for _ in $(seq 1 300); do gh_is "$1" 2>/dev/null && return 0; sleep 1; done; fail "never: $1"; }
ticket_run() {  # the first ticket run of issue $1; no second one started
  spore --json list | py 'base = "ticket:" + sys.argv[1] + ":"
mine = [r for r in d["runs"] if r["startKey"].startswith(base)]
assert [r["startKey"] for r in mine] == [base + "1"], mine
print(mine[0]["runId"])' "$1"
}
checkpoint() {  # the fake issue of checkpoint $1, as a Python expression
  echo "[i for i in s['issues'].values() if i['title'].startswith('Checkpoint $1:')][0]"
}

run="$(spore --json start build-0-2 --key build-0-2 | py 'print(d["runId"])')"
again="$(spore --json start build-0-2 --key build-0-2 | py 'print(d["runId"])')"
test "$again" = "$run" || fail "the same key started a second run"
listed="$(spore --json list | py 'print(*[r["runId"] for r in d["runs"] if r["startKey"] == "build-0-2"])')"
test "$listed" = "$run" || fail "spore list shows $listed, not $run"
spore worker > "$E/worker.log" 2>&1 & worker=$!
wait_for waiting checkpoint_0 1
check "sum(1 for p in s['prs'].values() if p['state'] == 'MERGED') == 2"
check "s['labels'] == {'Web-tree/runspore': ['checkpoint'], 'Max-Levitskiy/skills': ['ready-for-agent']}"
check "'spore signal' in $(checkpoint 0)['body'] and 'bin/' not in $(checkpoint 0)['body']"
spore signal "$run" checkpoint-0 --outcome go > /dev/null
echo "ok: spore start and spore worker ran the fast lane, a step created the labels, checkpoint 0 asked"

r10="Web-tree/runspore#60"
wait_gh "any('is stuck' in c for c in s['issues'].get('$r10', {}).get('comments', []))"
check "any('spore resolve' in c and '--retry' in c for c in s['issues']['$r10']['comments'])"
stuck_run="$(ticket_run "$r10")" || fail "R10 has another ticket run"
spore --json status "$stuck_run" | py 'i = d["invocation"]
assert d["status"] == "needs-intervention" and i["state"] == "stuck" and i["attempt"] == i["maxAttempts"] == 3, d
assert i["error"]["code"] == "command.exit", d' || fail "R10's ticket run is not stuck"
check "any('$stuck_run' in c and 'step is stuck' in c for c in s['issues']['$r10']['comments'])"
touch "$FAKE_CLAUDE_UNSTICK"
spore --json resolve "$stuck_run" --retry | py 'assert d["disposition"] == "applied", d'
echo "ok: R10's implement step could not recover, so its ticket run is stuck and its onIntervention told the issue; resolve --retry"

r12="Web-tree/runspore#62"
wait_gh "any('loop limit' in c for c in s['issues'].get('$r12', {}).get('comments', []))"
check "any('spore signal' in c and 'keep-going' in c for c in s['issues']['$r12']['comments'])"
red_run="$(ticket_run "$r12")" || fail "R12 has another ticket run"
spore --json status "$red_run" | py 'p = d["position"]
assert d["status"] == "waiting" and p["nodeId"] == "stuck", d
assert p["from"] == {"nodeId": "test", "outcome": "exhausted"}, d
assert d["nodes"]["test"]["outcome"] == "red" and d["nodes"]["test"]["visit"] == 4, d
assert d["streaks"] == {}, d' || fail "R12's ticket run did not take its exhausted arrow"
touch "$FAKE_CLAUDE_UNRED"
spore signal "$red_run" stuck --outcome keep-going > /dev/null
echo "ok: R12's checks stayed red, so its fix loop took the exhausted arrow to the stuck step and the wave asked; keep-going"

wait_for waiting checkpoint_a 1
check "sum(1 for p in s['prs'].values() if p['state'] == 'MERGED') == 11"
check "all(__import__('re').match(r'(feat|fix)\\([A-Z0-9-]+\\): [a-z]', p['title']) for p in s['prs'].values())"
check "any(p['title'].startswith('fix(R9): ') for p in s['prs'].values())"
check "s['issues']['Web-tree/runspore#59']['state'] == 'CLOSED'"
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^fix(R9): ')" = 1 || fail "R9 not merged once"
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^feat(R10): ')" = 1 || fail "R10 not merged once"
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^feat(R12): ')" = 1 || fail "R12 not merged once"
check "$(checkpoint 0)['state'] == 'CLOSED' and $(checkpoint 0)['comments'] == ['Answered \`go\`.']"
check "$(checkpoint A)['state'] == 'OPEN'"
echo "ok: waves 1-2 merged, wave 1 closed checkpoint 0's issue, checkpoint A asked"

refused="$(spore signal "$run" checkpoint-a --outcome changes --data '{"changes": []}' 2>&1)" \
  && fail "a changes answer with no change was accepted"
case "$refused" in *"/changes: expected at least 1 items, got 0"*) ;; *) fail "no schema error: $refused";; esac
spore signal "$run" checkpoint-a --outcome changes --data '{"changes": [""]}' > /dev/null 2>&1 \
  && fail "an empty change was accepted"
wait_for waiting checkpoint_a 1
echo "ok: spore refused answers that do not fit checkpoint A's schema, and recorded nothing"

changes='{"changes": ["Errors should name the settings key", "skills: the menu needs a hint line"]}'
# The checkpoint's text changes before it is asked again (an upgrade does this).
echo "Edited before the second ask, pass {pass}." >> .spore/workflows/build-0-2/steps/checkpoint-a.md
# Asked by a run-wave from before rounds were saved: the wave's merges are only in the issue.
python3 -c 'import json, sys; f = sys.argv[1]; s = json.load(open(f)); del s["a"]["rounds"]; json.dump(s, open(f, "w"))' "$E/state/checkpoints.json"
spore signal "$run" checkpoint-a --outcome changes --data "$changes" --id try-a-changes-1 > /dev/null
spore signal "$run" checkpoint-a --outcome changes --data "$changes" --id try-a-changes-1 > /dev/null  # a repeat
wait_for waiting checkpoint_a 2
check "'Edited before the second ask, pass 2.' in $(checkpoint A)['body'] and '<!-- build-0-2 checkpoint a -->' in $(checkpoint A)['body']"
check "'## Merged in this wave' in $(checkpoint A)['body'] and '## Merged from your changes, pass 2' in $(checkpoint A)['body']"
check "sum(1 for i in s['issues'].values() if i['title'].startswith('Checkpoint A feedback') and i['state'] == 'CLOSED') == 2"
check "$(checkpoint A)['state'] == 'OPEN'"
sleep 2
wait_for waiting checkpoint_a 2
echo "ok: changes built two feedback tickets, checkpoint A asked again with its current text, pass number and every round's merges; the repeat with the same --id was the same message"

spore signal "$run" checkpoint-a --outcome go > /dev/null
wait_for waiting checkpoint_b 1
check "$(checkpoint A)['state'] == 'CLOSED' and $(checkpoint A)['comments'][-1] == 'Answered \`go\`.'"
check "len($(checkpoint A)['comments']) == 2"
echo "ok: go led to wave 3, which closed checkpoint A's issue, and to checkpoint B"

# Upgrade: stop the worker, install a newer spore, start the worker again.
kill "$worker"; wait "$worker" 2>/dev/null || true; worker=""
export PATH="$E/spore-new:$path"
test "$(command -v spore)" = "$E/spore-new/spore" || fail "the newer spore is not the one on PATH"
again="$(spore --json start build-0-2 --key build-0-2 | py 'print(d["runId"])')"
test "$again" = "$run" || fail "the newer spore started a second run"
spore worker > "$E/worker2.log" 2>&1 & worker=$!
sleep 2
wait_for waiting checkpoint_b 1
dbs="$(cd "$orch/.spore" && ls -- *.db)"
test "$dbs" = "runspore.db" || fail "databases in .spore: $dbs"
python3 "$orch/work/build-0-2/track.py" --page "$E/page.html"
grep -q 'Checkpoint B: waiting for your answer' "$E/page.html" || fail "page"
spore signal "$run" checkpoint-b --outcome go > /dev/null
wait_gh "$(checkpoint B)['state'] == 'CLOSED'"
check "$(checkpoint B)['comments'] == ['Answered \`go\`.']"
spore --json status "$run" | py 'assert d["nodes"]["checkpoint_b"]["outcome"] == "go", d
assert d["position"]["nodeId"] != "checkpoint_b", d' || fail "the run did not move on from checkpoint B"
check "s['labels'] == {'Web-tree/runspore': ['checkpoint'], 'Max-Levitskiy/skills': ['ready-for-agent']}"
check "sum(1 for c in s['log'] if c[:2] == ['label', 'create']) > 2"
echo "ok: the newer spore went on with the same database and run; go closed checkpoint B and wave 4 started; labels made again are unchanged; page renders"
echo "PASS ($E)"
