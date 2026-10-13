#!/bin/sh
# Checks that a PR title is a conventional commit title (spec/release.md section 8): a
# squash merge makes it the commit title, and release-please reads that. Prints why a
# title is wrong. Exits 1 only when PR_TITLE_STRICT is true; otherwise it warns.
#
#   check-pr-title.sh <title>

set -eu
title=${1?usage: check-pr-title.sh <title>}
types='feat|fix|perf|refactor|docs|test|build|ci|chore|revert'

if printf '%s\n' "$title" | grep -Eq "^($types)(\([^()]+\))?!?: [^ ]"; then
	echo "PR title ok: $title"
	exit 0
fi

why="PR title is not a conventional commit: expected '<type>: <subject>' or '<type>(<scope>): <subject>', <type> one of $(echo "$types" | sed 's/|/, /g'), '!' before ':' for a breaking change; got '$title'"
if [ "${PR_TITLE_STRICT:-}" = true ]; then
	echo "::error::$why"
	exit 1
fi
echo "::warning::$why (a warning until the repository variable PR_TITLE_STRICT is true)"
