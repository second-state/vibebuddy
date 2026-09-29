# Issue tracker: GitHub

Issues and specs in this repo all live as GitHub issues. Use the `gh` CLI for every operation.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments with `jq` and fetching labels along the way.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`, adding `--label` and `--state` as needed.
- **Comment**: `gh issue comment <number> --body "..."`
- **Add / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

The repo is inferred from `git remote -v`; `gh` picks it up automatically when run inside the checkout.

## PRs as a triage entry point

**PRs as a request entry point: no.** _(If this repo treats external PRs as feature requests, change this to "yes"; `/triage` reads this switch.)_

When set to "yes", PRs go through the same labels and states as issues, using the matching `gh pr` commands:

- **Read a PR**: `gh pr view <number> --comments`; for the diff, `gh pr diff <number>`.
- **List external PRs awaiting triage**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments`, keeping only those whose `authorAssociation` is `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR` or `NONE` (dropping `OWNER`, `MEMBER`, `COLLABORATOR`).
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub issues and PRs share one number space, so a bare `#42` could be either: try `gh pr view 42` first, then `gh issue view 42` if that fails.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is one issue; its **children** are sub-issues that act as tickets.

- **Map**: an issue labelled `wayfinder:map`, with Notes / Decisions-so-far / Fog in its body. `gh issue create --label wayfinder:map`.
- **Child ticket**: attached under the map as a GitHub sub-issue (via the sub-issues endpoint of `gh api`). If sub-issues are unavailable, add the child to a task list in the map's body and put `Part of #<map>` at the top of the child's body. Label: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, assign the ticket to the developer who owns it.
- **Blocking**: use GitHub's **native issue dependencies**, the canonical representation visible in the UI. To add an edge: `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports still-open blockers through `issue_dependencies_summary.blocked_by`; that is the live gate. If dependencies are unavailable, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child's body. A ticket is unblocked only when all its blockers are closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, limited to the map's sub-issues / task list), drop any that still have an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or already have an assignee, and take the first one in map order.
- **Claim**: `gh issue edit <n> --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist plus link) to the map's Decisions-so-far.
