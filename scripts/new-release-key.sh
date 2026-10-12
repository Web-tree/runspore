#!/bin/sh
# Makes a new release key pair (spec/release.md section 4). It stores the private half
# as the repository secret RELEASE_SIGNING_KEY and pins the public half in install.sh,
# README.md and spec/release.md, replacing the key pinned there now. Run it from a
# clone with gh signed in as a repository admin, then commit the three files. Running
# it again rotates the key.

set -eu
cd "$(dirname "$0")/.."

old=$(sed -n "s/^RELEASE_KEY='\(.*\)'\$/\1/p" install.sh)
if [ -z "$old" ]; then
	echo "new-release-key.sh: install.sh has no RELEASE_KEY line" >&2
	exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
ssh-keygen -q -t ed25519 -N '' -C runspore-release -f "$tmp/key"
gh secret set RELEASE_SIGNING_KEY --repo Web-tree/runspore <"$tmp/key"
new=$(cat "$tmp/key.pub")

for file in install.sh README.md spec/release.md; do
	awk -v old="$old" -v new="$new" '
		(i = index($0, old)) > 0 { $0 = substr($0, 1, i - 1) new substr($0, i + length(old)) }
		{ print }
	' "$file" >"$tmp/out"
	cat "$tmp/out" >"$file"
done
echo "pinned $new"
