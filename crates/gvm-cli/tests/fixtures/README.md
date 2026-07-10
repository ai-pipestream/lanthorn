# gvm-cli test fixtures

## glulxercise.ulx

The **glulxercise** Glulx interpreter conformance suite by Andrew Plotkin — the
Glulx analogue of the Z-machine `czech`/`praxix` torture tests. Driven headlessly
by `tests/glulxercise.rs` as the phase-3a Glk I/O capstone.

- **Source:** <https://eblong.com/zarf/glulx/glulxercise.ulx>
- **Version:** Release 13 / Serial 241202 (Inform v6.43), game-file format 3.1.3
- **SHA-256:** `b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348`
- **Downloaded:** 2026-06-27 via `curl -L`

### In scope (asserted PASSING)

Core VM + the Glk I/O subset: `operand`, `arith`, `bitwise`, `shift`, `aload`,
`astore`, `arraybit`, `call`, `callstack`, `jump`, `jumpform`, `compare`, `stack`,
`throw`, `streamnum`, `strings`, `ramstring`, `glk`, `search`, `mzero`, `mcopy`,
`nonrandom`, `undo`, `protect`, `memsize`, `heap`, `verify`, and more — all pass.

The **filter I/O system** (iosys mode 1; SQ-0245) is also implemented and
in scope: `iosys`, `iosys2`, `iosys3`, `filter`, `nullio`, `gestalt` all pass
(the plain `iosys` group's mid-string I/O-system-switch case was fixed under
SQ-0249).

### Out of scope (excluded; not yet implemented)

- `gidispa` — the **gi_dispatch introspection layer** (type-tagged string
  dispatch); not part of the IF Glk subset and not used by real games.
- `acceleration` — accelerated functions ARE intercepted (on by default;
  `--no-accel` disables); this glulxercise group is simply not in the in-scope
  assertion list above.
- `doubleconv`/`doublearith`/… — double-precision opcodes are deferred
  (gestalt Double = 0). Single-precision `floatconv`/`floatarith` ARE implemented
  (gestalt Float = 1) but are likewise not in the in-scope assertion list.
- `restore` — file streams / Glk-stream save are not wired in this phase.
