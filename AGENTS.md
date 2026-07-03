# babelmap for agents

babelmap tracks its work with **side-quest**, a git-native issue tracker. Quests
live on a dedicated git ref (`refs/side-quest/quests`), not in the working tree,
and are driven through the `side-quest` MCP server (registered in `.mcp.json`).

> The old `TODO.md` / `COMPLETED.md` + `.githooks/` + `scripts/todo-done`
> bookkeeping has been **retired** and imported into side-quest (SQ-0001–SQ-0157
> completed, SQ-0158–SQ-0194 open/partial/deferred). Do **not** recreate those
> files or that workflow — capture and track everything as quests.

## Capture reflex

When a new, unrelated idea surfaces mid-task, capture it instead of derailing:
use `/sq <idea>`, or call the `quest_new` MCP tool with a concise `title` and a
one-sentence `context` on *why it came up now*. Don't set it current; keep
working.

## Attributing commits (the trailer contract)

Commits link to quests through message trailers, read by side-quest's
`post-commit` hook:

- `Quest: SQ-0001` — link this commit to SQ-0001 (no status change).
- `Completes: SQ-0001` — link it and mark SQ-0001 done.
- `Quest: none` — explicit opt-out for a genuine chore.

Prefer explicit, per-commit trailers over sticky state, so unrelated commits are
never mis-attributed. `require_quest` enforcement is off by default, so a commit
without a trailer is not blocked.

## The current quest

Each worktree can have one "current" quest (`quest_set_current`). With
`auto_trailer` on (the default), the `prepare-commit-msg` hook injects that
quest's `Quest:` trailer automatically. Setting a current quest is optional;
prefer writing the trailer explicitly.

## Triage values

`type` is `bug` or `feature` (default `feature`); `priority` is `high` or `low`
(default `low`); `status` is `open`, `partial`, `done`, `deferred`, or
`discarded` (new quests start `open`). Tags are free-form annotations.

## Browsing

`quest_list` / `quest_show` via MCP, or the CLI: `side-quest list`,
`side-quest show SQ-0001`.
