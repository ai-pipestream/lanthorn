# Interop golden saves — provenance (SQ-0158)

These are reference-interpreter-produced save files, checked in so the READ-direction
interop tests (`crates/zvm/tests/save_interop.rs`) can run in CI without any external
binary. Regenerate with `scripts/gen-interop-goldens.sh`.

## `minizork-at-P.qzl`

- **Story:** `crates/zvm/tests/fixtures/minizork.z3` (Mini-Zork I, Infocom 1988).
- **Reference interpreter:** `dfrotz` (FROTZ V2.55, Dumb interface; homebrew `frotz`).
- **Point P — prefix commands (verbatim):** `open mailbox` → `take leaflet` → `north`.
  Resulting state: room = *North of House*, the leaflet is in the player's inventory.
- **Save command:** the game's `save` verb, written to this path.
- **Format:** Quetzal `FORM … IFZS` (`IFhd` + `CMem` + `Stks`), ~366 bytes.
- **Probe (used by the test):** `look` (reveals room) + `inventory` (reveals the leaflet).
  A broken restore would place the player elsewhere or drop the leaflet — so the
  cross-load equivalence assertion cannot pass vacuously.
