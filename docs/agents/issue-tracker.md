# Issue tracker: GitHub

Issues and specs for this repo live in GitHub Issues at `Xeift/CatDesk`. Use the `gh` CLI with `--repo Xeift/CatDesk` for all operations because `origin` is a fork.

## Conventions

- **Create an issue**: `gh issue create --repo Xeift/CatDesk --title "..." --body "..."`.
- **Read an issue**: `gh issue view <number> --repo Xeift/CatDesk --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --repo Xeift/CatDesk --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --repo Xeift/CatDesk --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --repo Xeift/CatDesk --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --repo Xeift/CatDesk --comment "..."`

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --repo Xeift/CatDesk --comments` and `gh pr diff <number> --repo Xeift/CatDesk`.
- **List external PRs for triage**: `gh pr list --repo Xeift/CatDesk --state open --json number,title,body,labels,author,authorAssociation,comments`, then keep only `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE`.
- **Comment / label / close**: use `gh pr comment`, `gh pr edit`, or `gh pr close` with `--repo Xeift/CatDesk`.

GitHub shares one number space across issues and PRs, so resolve a bare `#42` with `gh pr view 42 --repo Xeift/CatDesk`, then fall back to `gh issue view 42 --repo Xeift/CatDesk`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue in `Xeift/CatDesk`.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --repo Xeift/CatDesk --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue. Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body.
- **Blocking**: GitHub's native issue dependencies. Add an edge with `gh api --method POST repos/Xeift/CatDesk/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`.
- **Frontier query**: list the map's open children, drop any with an open blocker or assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --repo Xeift/CatDesk --add-assignee @me`.
- **Resolve**: comment on and close the issue, then append a context pointer to the map's Decisions-so-far.
