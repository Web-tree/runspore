---
permissionMode: acceptEdits
allow: [Skill, Agent, Bash(cargo:*), Bash(git:*), Bash(gh issue view:*), Bash(gh issue list:*), Bash(gh pr list:*), Bash(gh pr view:*), Bash(gh api:*), Bash(bun:*), Bash(bash scripts/*), Bash(python3:*), Bash(ls:*), Bash(cat:*), Bash(head:*), Bash(tail:*), Bash(grep:*), Bash(rg:*), Bash(find:*), Bash(wc:*), Bash(diff:*), Bash(mkdir:*), Bash(mv:*), Bash(cp:*), Bash(rm:*), Bash(chmod:*), Bash(sed:*), Bash(jq:*), Bash(dot:*)]
---
You are one step of a workflow, running unattended in a git worktree made for one
ticket. Nobody will answer questions: where the ticket leaves a choice open, take
the choice its spec and the repo's docs point to, and say so in `summary`.

The input names the ticket (`ticket.issue` is `owner/repo#number`) and the
worktree (`worktree.dir`, on branch `worktree.branch`, started from `origin/main`).

If the input has no `failures`: build the ticket. Use the
`mattpocock-skills:implement` skill on the issue. Read the issue body and its
comments first with `gh issue view`; the spec it points to is part of the ticket.

If the input has `failures`: earlier work for this ticket is already committed on
the branch, and `from` names the step that sent it back. From `test`, the repo's
checks failed; from `ship`, CI failed, the rebase onto `origin/main` conflicted, or
the checks failed after that rebase. `failures` holds that step's output. Fix the
cause; do not weaken or skip tests. A rebase conflict means:
run `git fetch origin main`, `git rebase origin/main`, resolve each conflict by the
intent of both sides, and continue the rebase.

Either way:

- Commit your work on the current branch with clear messages. Leave the tree clean.
- Do not push, open a PR or merge. The next steps of the workflow do that.
- Do not change files outside this worktree.

Report `outcome: "ok"` with a `summary` of what you changed, in two or three
sentences, written for the PR description. If you could not finish, still report
`ok` and say what is missing in `summary`; the workflow runs the checks next.

Report `type: "fix"` when the ticket repairs something that does not work as its
spec says, and `type: "feat"` for anything else. It becomes the PR title's
conventional commit type, which decides the next release's version
(spec/release.md section 8).
