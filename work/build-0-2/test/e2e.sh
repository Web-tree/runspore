#!/usr/bin/env bash
# End-to-end test of the build-0-2 harness with real spore 0.1 and fake gh and claude.
#
#   work/build-0-2/test/e2e.sh [scratch-dir]
#
# Scratch git repos stand in for both GitHub repos (fake `gh pr merge` squash-merges
# into their bare origins). Checks: waves 1-2 merge (R9 goes red once and is fixed),
# checkpoint A asks; a `changes` answer builds two feedback tickets and asks again;
# `go` moves on to checkpoint B; `build --restart` replays to B without asking A.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
E="${1:-$(mktemp -d)}"
SPORE="${SPORE:-$HOME/.local/share/runspore-0.1/bin/spore}"
rm -rf "$E" && mkdir -p "$E"
for r in runspore skills; do
  git init -q --bare -b main "$E/bare-$r.git"
  git clone -q "$E/bare-$r.git" "$E/$r" 2>/dev/null
  (cd "$E/$r" && echo seed > README && git add README \
    && git -c user.name=seed -c user.email=seed@x commit -qm seed && git push -q origin main)
done
export SPORE PATH="$here/fakebin:$PATH"
export RUNSPORE_DB="$E/runspore.db" BUILD_STATE="$E/state" BUILD_HOME="$E/wt" BUILD_POLL_S=0.3 BUILD_NO_LABELS=1
export BUILD_REPOS="{\"Web-tree/runspore\": \"$E/runspore\", \"Max-Levitskiy/skills\": \"$E/skills\"}"
export BUILD_CHECKS='{"Web-tree/runspore": [["bash", "-c", "test ! -f BROKEN"]], "Max-Levitskiy/skills": [["true"]]}'
export FAKE_GH_STATE="$E/gh.json"
export FAKE_GH_BARES="{\"Web-tree/runspore\": \"$E/bare-runspore.git\", \"Max-Levitskiy/skills\": \"$E/bare-skills.git\"}"
bin="$root/work/build-0-2/bin"
worker=""
stop() { [ -n "$worker" ] && kill "$worker" 2>/dev/null; wait 2>/dev/null || true; }
trap stop EXIT

where() { "$SPORE" --db "$RUNSPORE_DB" --json status "$("$bin/start-run")" \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);p=d["position"];print(d["status"], p["nodeId"] if p else "-", p["visit"] if p else "-")'; }
wait_for() {  # status node visit
  for _ in $(seq 1 300); do
    now="$(where)"
    [ "$now" = "$*" ] && return 0
    case "$now" in failed*|completed*) echo "FAIL: run is $now, wanted $*"; exit 1;; esac
    sleep 1
  done
  echo "FAIL: still $now, wanted $*"; exit 1
}
check() { python3 -c "import json,sys; s=json.load(open('$FAKE_GH_STATE')); sys.exit(0 if ($1) else 1)" || { echo "FAIL: $1"; exit 1; }; }

"$bin/build" > "$E/worker.log" 2>&1 & worker=$!
wait_for waiting checkpoint_a 1
check "sum(1 for p in s['prs'].values() if p['state'] == 'MERGED') == 11"
check "s['issues']['Web-tree/runspore#59']['state'] == 'CLOSED'"
test "$(git -C "$E/bare-runspore.git" log --format=%s main | grep -c '^R9: ')" = 1 || { echo "FAIL: R9 not merged once"; exit 1; }
echo "ok: waves 1-2 merged, checkpoint A asked"

"$bin/answer" a changes "Errors should name the settings key" "skills: the menu needs a hint line" > /dev/null
wait_for waiting checkpoint_a 2
check "sum(1 for i in s['issues'].values() if i['title'].startswith('Checkpoint A feedback') and i['state'] == 'CLOSED') == 2"
echo "ok: changes built two feedback tickets, checkpoint A asked again"

"$bin/answer" a go > /dev/null
wait_for waiting checkpoint_b 1
echo "ok: go led to wave 3 and checkpoint B"

kill "$worker"; wait "$worker" 2>/dev/null || true
"$bin/build" --restart > "$E/worker2.log" 2>&1 & worker=$!
wait_for waiting checkpoint_b 1
check "len([i for i in s['issues'].values() if i['title'].startswith('Checkpoint A:')][0]['comments']) == 2"
python3 "$root/work/build-0-2/track.py" --page "$E/page.html"
grep -q 'Checkpoint B: waiting for your answer' "$E/page.html" || { echo "FAIL: page"; exit 1; }
echo "ok: restart replayed to checkpoint B without asking A; page renders"
echo "PASS ($E)"
