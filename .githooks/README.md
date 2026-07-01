# Git hooks — TODO/COMPLETED bookkeeping

These hooks keep `TODO.md` and `COMPLETED.md` (both tracked) in lockstep with
git history, so finished work is always recorded and every completed item is
traceable to the commit that finished it.

## Activation (once per clone)

`core.hooksPath` is stored in local git config, which is **not** copied by
`git clone`. After cloning, run:

```sh
git config core.hooksPath .githooks
```

Existing worktrees of an already-configured repo share the setting (it lives in
the common config) — you only need this on a fresh clone.

## The id scheme

Each completed item carries a random id `TODO-xxxxxx` (6 hex, ~16.7M space). The
id is the stable link between `COMPLETED.md` and the commit that finished the
work, recorded as a `Completes: TODO-xxxxxx` trailer in the commit message. (A
commit can't contain its own hash, so the id — which we assign — is the join key
instead.)

Random ids need no central counter, so parallel worktree lanes each allocate
independently. The `commit-msg` hook rejects the rare collision; just reroll.

- item → commit: `git log --grep='Completes: TODO-3f9a2c'`
- commit → item: read its `Completes:` trailer, grep `COMPLETED.md`

## Workflow

When a commit finishes a TODO item:

```sh
scripts/todo-done "<text that matches the TODO.md line>"
git add TODO.md COMPLETED.md <code files>
git commit          # trailer auto-added and validated
```

`todo-done` moves the one matching line out of `TODO.md` and into
`COMPLETED.md` with a fresh random 6-hex id. Works the same on main or in a
worktree.

## What the hooks do

- **pre-commit** — warns (non-blocking) if `crates/**` is staged but
  `COMPLETED.md` isn't, in case a finished item wasn't recorded.
- **prepare-commit-msg** — auto-appends a `Completes: TODO-NNNN` trailer for
  every id newly added to `COMPLETED.md`.
- **commit-msg** — blocks the commit unless: no duplicate ids in `COMPLETED.md`,
  every new id is cited by a trailer, and every trailer references a real id.

Bypass any hook with `git commit --no-verify` for genuine exceptions.
