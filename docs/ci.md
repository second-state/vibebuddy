# CI Status Feedback

## Why it's worth doing

By the time CI produces a result, you've usually long since switched to something else. The tab on your laptop screen only helps if you go look at it, while the box sits at the edge of your vision the whole time. This is one of the cases where the device is genuinely more useful than a screen.

CI and agents share the same task cards, the same buddy and the same set of announcements; only the source differs: agents push through hooks, while `vibebuddyd` actively polls GitHub Actions for CI.

## Mapping

| GitHub Actions run | VibeBuddy state |
| --- | --- |
| `queued` / `in_progress` | Working, titled `CI:<repo name>` |
| `completed` + `success` | Done; plays "All done!" once |
| `completed` + `failure` / `timed_out` / other | Failed; plays "Uh-oh, something went wrong." once |
| `completed` + `cancelled` / `skipped` / `neutral` | Quietly dismisses the card, no announcement |

Each repo takes at most one card, showing that repo's most recent run. The polling interval is 30 seconds; CI is measured in minutes, so 30 seconds is plenty without spending API quota on it.

**Only report runs actually seen running.** If a run has already finished and `vibebuddyd` never saw it in progress, nothing happens. Without this rule, every daemon restart would re-announce each repo's most recent historical result, including yesterday's failure. The cost: a run that starts and finishes between two polls won't be announced.

## Which repos to watch

There's no config file, and you don't need to make a list. What `vibebuddyd` watches is **the GitHub repos an agent has worked in within the last hour**: every hook carries `cwd`, which the adapter already has to resolve to a git project root to build the task card title; CI reuses the same fact and then reads `owner/repo` from the `origin` remote in `.git/config`.

This derivation reads only local files and calls neither git nor the network. Subdirectories and worktrees all resolve to the main repo. Projects without a GitHub remote, and projects with no agent activity in the last hour, aren't polled; when no project is being tracked, the whole feature makes no requests at all.

The one-hour window corresponds to "I'm working in this repo, so I care about its CI". CI usually produces results within a few minutes of a push, and there's always agent activity before a push.

`VIBEBUDDY_CI_REPOS` (comma-separated) overrides the automatic derivation, for watching repos that aren't checked out on this machine.

## Where `gh` lives

`vibebuddyd` reads status via `gh run list`, reusing your existing GitHub login instead of storing a token of its own.

Processes started by launchd get only a very short `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which usually doesn't include `gh`, so `vibebuddyd` tries `~/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and `/usr/bin` in turn, and falls back to `PATH` only if none of them has it. If it's installed elsewhere, set its absolute path with `VIBEBUDDY_GH`. No plist changes needed.

## Boundaries

Only a run's `databaseId`, `status` and `conclusion` are requested. No logs, no diffs, no commit messages are read; only the repo name and status ever appear on the device.

When reads for a repo keep failing, only the first failure is logged, and recovery is logged once. One warning every 30 seconds adds up to thousands of lines a day, burying the logs that actually matter.
