#!/usr/bin/env bash
# Regenerate the save-format interop golden files and run the live interop suite.
# Developer-run (NOT CI). See docs/superpowers/specs/2026-07-08-save-interop-testing-design.md
#
# Requires: dfrotz  (brew install frotz)
#
# Z-machine only. Glulx interop is deferred to SQ-0229 (homebrew glulxe is curses-only
# and not headless-scriptable; a sound fixture needs Inform 6 + library).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

command -v dfrotz >/dev/null || { echo "dfrotz not found — run: brew install frotz" >&2; exit 1; }

STORY="crates/zvm/tests/fixtures/minizork.z3"
GOLD="crates/zvm/tests/fixtures/interop/minizork-at-P.qzl"

# Point P: open mailbox -> take leaflet -> north  (North of House, leaflet carried).
# Drive the prefix, then the game's `save` verb writing to $GOLD, then quit.
mkdir -p "$(dirname "$GOLD")"
rm -f "$GOLD"
printf 'open mailbox\ntake leaflet\nnorth\nsave\n%s\nquit\ny\n' "$GOLD" | dfrotz "$STORY" >/dev/null
[ -s "$GOLD" ] || { echo "FAILED to write $GOLD" >&2; exit 1; }
echo "wrote $GOLD ($(wc -c < "$GOLD") bytes)"

# Run the live (#[ignore]) interop tests, which need dfrotz at test time.
echo "running live interop tests (cargo test -p zvm --test save_interop -- --ignored) ..."
cargo test -p zvm --test save_interop -- --ignored
