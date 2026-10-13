#!/usr/bin/env bash
# End-to-end test of the build-0-2 harness with a real spore and fake gh and claude.
#
#   work/build-0-2/test/e2e.sh [scratch-dir]   # SPORE=<binary> to test that spore;
#                                              # else this checkout's spore is built
#
# The build runs from a scratch clone of this checkout (with its uncommitted
# changes), so `build --upgrade` can check it out; each upgrade's "install" is a
# link to $SPORE. Scratch git repos stand in for both GitHub repos (fake
# `gh pr merge` squash-merges into their bare origins). Checks: the first
# `build --upgrade` runs the fast lane and asks checkpoint 0; `go` runs waves 1-2
# (R9 goes red once and is fixed; R10's implement step can't recover, is stuck,
# and `spore resolve --retry` gets it merged; R12's checks stay red past the fix
# loop's limit, its run waits at the stuck step, and `spore signal ... keep-going`
# gets it merged) and checkpoint A asks; spore refuses answers that do not fit its
# schema; a `changes` answer builds two feedback tickets and asks again; `go` moves
# on to checkpoint B;
# upgrading to a newer commit starts a new database and replays to B without
# asking again.
set -euo pipefail
# Run from inside a step (a ticket's test, say), the caller's RUNSPORE_DB names the
# real build's database, and buildlib would prefer it to the scratch one.
for v in $(compgen -e); do case "$v" in RUNSPORE_*) unset "$v" ;; esac; done
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
E="${1:-$(mktemp -d)}"
if [ -z "${SPORE:-}" ]; then
  cargo build --quiet --locked --manifest-path "$root/Cargo.toml" -p runspore-cli
  SPORE="${CARGO_TARGET_DIR:-$root/target}/debug/spore"
fi
rm -rf "$E" && mkdir -p "$E"
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
commit() { git -C "$orch" add -A && git -C "$orch" -c user.name=e2e -c user.email=e2e@x commit -q --allow-empty -m "$1"
  git -C "$orch" rev-parse HEAD; }
engine() { mkdir -p "$E/engines/${1:0:12}/bin" && ln -sf "$SPORE" "$E/engines/${1:0:12}/bin/spore"; }
first="$(commit "e2e: the working tree")"
engine "$first"
export PATH="$here/fakebin:$PATH" BUILD_ENGINES="$E/engines"
export BUILD_STATE="$E/state" BUILD_HOME="$E/wt" BUILD_POLL_S=0.3 BUILD_NO_LABELS=1
export BUILD_REPOS="{\"Web-tree/runspore\": \"$E/runspore\", \"Max-Levitskiy/skills\": \"$E/skills\"}"
export BUILD_CHECKS='{"Web-tree/runspore": [["bash", "-c", "test ! -f BROKEN"]], "Max-Levitskiy/skills": [["true"]]}'
export FAKE_GH_STATE="$E/gh.json"
export FAKE_CLAUDE_STUCK=R10 FAKE_CLAUDE_UNSTICK="$E/r10-unstick"
export FAKE_CLAUDE_RED=R12 FAKE_CLAUDE_UNRED="$E/r12-unred"
export FAKE_GH_BARES="{\"Web-tree/runspore\": \"$E/bare-runspore.git\", \"Max-Levitskiy/skills\": \"$E/bare-skills.git\"}"
bin="$orch/work/build-0-2/bin"
worker=""
stop() { [ -n "$worker" ] && kill "$worker" 2>/dev/null; wait 2>/dev/null || true; }
trap stop EXIT

where() { python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import buildlib as b
d = b.spore("status", b.build_run_id()); p = d["position"]
print(d["status"], p["nodeId"] if p else "-", p["visit"] if p else "-")' "$bin" 2>/dev/null || echo "starting"; }
wait_for() {  # status node visit
  for _ in $(seq 1 300); do
    now="$(where)"
    [ "$now" = "$*" ] && return 0
    case "$now" in failed*|completed*) echo "FAIL: run is $now, wanted $*"; exit 1;; esac
    sleep 1
  done
  echo "FAIL: still $now, wanted $*"; exit 1
}
on_engine() {  # wait until `build --upgrade` has recorded commit $1
  for _ in $(seq 1 100); do grep -qs "$1" "$BUILD_STATE/engine.json" && return 0; sleep 0.2; done
  echo "FAIL: build --upgrade did not record $1"; exit 1
}
check() { python3 -c "import json,sys; s=json.load(open('$FAKE_GH_STATE')); sys.exit(0 if ($1) else 1)" || { echo "FAIL: $1"; exit 1; }; }

"$bin/build" --upgrade "$first" > "$E/worker.log" 2>&1 & worker=$!
on_engine "$first"
wait_for waiting checkpoint_0 1
check "sum(1 for p in s['prs'].values() if p['state'] == 'MERGED') == 2"
"$bin/answer" 0 go > /dev/null
echo "ok: upgrade started the run, the fast lane merged R1 and R3, checkpoint 0 asked"

r10="Web-tree/runspore#60"
for _ in $(seq 1 300); do
  python3 -c "import json,sys; s=json.load(open('$FAKE_GH_STATE'))
sys.exit(0 if any('is stuck' in c for c in s['issues'].get('$r10', {}).get('comments', [])) else 1)" 2>/dev/null && break
  sleep 1
done
check "any('resolve' in c and '--retry' in c for c in s['issues']['$r10']['comments'])"
stuck_run="$(python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import buildlib as b
r = [r for r in b.runs() if r["startKey"] == "ticket:" + sys.argv[2] + ":1"][0]
d = b.spore("status", r["runId"]); i = d["invocation"]
assert d["status"] == "needs-intervention" and i["state"] == "stuck" and i["attempt"] == i["maxAttempts"] == 3, d
assert i["error"]["code"] == "command.exit", d
assert not [r for r in b.runs() if r["startKey"] == "ticket:" + sys.argv[2] + ":2"], "a second try started"
print(r["runId"])' "$bin" "$r10")" || { echo "FAIL: R10's ticket run is not stuck"; exit 1; }
touch "$FAKE_CLAUDE_UNSTICK"
python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import buildlib as b
assert b.spore("resolve", sys.argv[2], "--retry")["disposition"] == "applied"' "$bin" "$stuck_run"
echo "ok: R10's implement step could not recover, so its ticket run is stuck and the wave asked; resolve --retry"

r12="Web-tree/runspore#62"
for _ in $(seq 1 300); do
  python3 -c "import json,sys; s=json.load(open('$FAKE_GH_STATE'))
sys.exit(0 if any('loop limit' in c for c in s['issues'].get('$r12', {}).get('comments', [])) else 1)" 2>/dev/null && break
  sleep 1
done
check "any('signal' in c and 'keep-going' in c for c in s['issues']['$r12']['comments'])"
red_run="$(python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import buildlib as b
r = [r for r in b.runs() if r["startKey"] == "ticket:" + sys.argv[2] + ":1"][0]
d = b.spore("status", r["runId"]); p = d["position"]
assert d["status"] == "waiting" and p["nodeId"] == "stuck", d
assert p["from"] == {"nodeId": "test", "outcome": "exhausted"}, d
assert d["nodes"]["test"]["outcome"] == "red" and d["nodes"]["test"]["visit"] == 4, d
assert d["streaks"] == {}, d
assert not [r for r in b.runs() if r["startKey"] == "ticket:" + sys.argv[2] + ":2"], "a second try started"
print(r["runId"])' "$bin" "$r12")" || { echo "FAIL: R12's ticket run did not take its exhausted arrow"; exit 1; }
touch "$FAKE_CLAUDE_UNRED"
python3 -c 'import sys; sys.path.insert(0, sys.argv[1]); import buildlib as b
b.spore("signal", sys.argv[2], "stuck", "--outcome", "keep-going")' "$bin" "$red_run"
echo "ok: R12's checks stayed red, so its fix loop took the exhausted arrow to the stuck step and the wave asked; keep-going"

wait_for waiting checkpoint_a 1
check "sum(1 for p in s['prs'].values() if p['state'] == 'MERGED') == 11"
check "all(__import__('re').match(r'(feat|fix)\\([A-Z0-9-]+\\): [a-z]', p['title']) for p in s['prs'].values())"
check "any(p['title'].startswith('fix(R9): ') for p in s['prs'].values())"
check "s['issues']['Web-tree/runspore#59']['state'] == 'CLOSED'"
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^fix(R9): ')" = 1 || { echo "FAIL: R9 not merged once"; exit 1; }
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^feat(R10): ')" = 1 || { echo "FAIL: R10 not merged once"; exit 1; }
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^feat(R12): ')" = 1 || { echo "FAIL: R12 not merged once"; exit 1; }
echo "ok: waves 1-2 merged, checkpoint A asked"

refused="$("$bin/answer" a changes 2>&1)" && { echo "FAIL: a changes answer with no change was accepted"; exit 1; }
case "$refused" in *"/changes: expected at least 1 items, got 0"*) ;; *) echo "FAIL: no schema error: $refused"; exit 1;; esac
"$bin/answer" a changes "" > /dev/null 2>&1 && { echo "FAIL: an empty change was accepted"; exit 1; }
wait_for waiting checkpoint_a 1
echo "ok: spore refused answers that do not fit checkpoint A's schema, and recorded nothing"

"$bin/answer" a changes "Errors should name the settings key" "skills: the menu needs a hint line" > /dev/null
"$bin/answer" a changes "Errors should name the settings key" "skills: the menu needs a hint line" > /dev/null  # a repeat
wait_for waiting checkpoint_a 2
check "sum(1 for i in s['issues'].values() if i['title'].startswith('Checkpoint A feedback') and i['state'] == 'CLOSED') == 2"
sleep 2
wait_for waiting checkpoint_a 2
echo "ok: changes built two feedback tickets, checkpoint A asked again; the repeated answer was the same message"

"$bin/answer" a go > /dev/null
wait_for waiting checkpoint_b 1
echo "ok: go led to wave 3 and checkpoint B"

kill "$worker"; wait "$worker" 2>/dev/null || true
second="$(commit "e2e: a newer spore")"
engine "$second"
git -C "$orch" checkout -q --detach "$first"
"$bin/build" --upgrade "$second" > "$E/worker2.log" 2>&1 & worker=$!
on_engine "$second"
wait_for waiting checkpoint_b 1
test -f "$orch/.spore/runspore-${second:0:12}.db" || { echo "FAIL: no database for the new spore"; exit 1; }
grep -q "build --upgrade ${first:0:12}" "$E/worker2.log" || { echo "FAIL: no way back printed"; exit 1; }
check "len([i for i in s['issues'].values() if i['title'].startswith('Checkpoint A:')][0]['comments']) == 2"
check "len([i for i in s['issues'].values() if i['title'].startswith('Checkpoint 0:')][0]['comments']) == 1"
python3 "$orch/work/build-0-2/track.py" --page "$E/page.html"
grep -q 'Checkpoint B: waiting for your answer' "$E/page.html" || { echo "FAIL: page"; exit 1; }
echo "ok: upgrade to a newer commit replayed to checkpoint B in a new database; page renders"
echo "PASS ($E)"
