# wave17-style-file Cleanup Report

## STATUS: COMPLETE

All three fixes applied. Build is clean, all tests pass, zero warnings.

## Cargo Test Result

```
test result: ok. 456 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.58s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 159 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 153 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.02s
```

## Zero-Warnings Confirmation

`cargo build --workspace` emits zero warnings (confirmed via grep for "^warning|^error").

## Fix 1 - Orphan Removals

### Removed

- `config::ColorsConfig` struct (crates/app/src/config.rs): confirmed zero live (non-test) callers
  in src/. Only callers were colors.rs tests and headless.rs integration test, both of which
  exclusively tested the removed item.

- `config::write_symbols` function (crates/app/src/config.rs): confirmed zero live callers.
  Only caller was the `write_symbols_round_trips_preserving_other_keys` test, which was also removed.

- `colors::ColorScheme::resolve` method (crates/app/src/colors.rs): confirmed zero live callers.
  All callers were in `colors.rs` tests and `headless.rs`. Tests were removed; headless.rs test
  was updated to use the live path (`style::resolve` with `StyleDoc`).

- `colors::apply_terminal_overrides` function (crates/app/src/colors.rs): only called from
  `ColorScheme::resolve` (which was removed). Removed together.

- `colors::dummy_ansi_scheme` function (crates/app/src/colors.rs): only called from
  `apply_terminal_overrides` (which was removed). Removed together.

- Colors.rs tests that exclusively exercised removed items:
  - `resolve_no_scheme_gives_terminal_default`
  - `resolve_builtin_tomorrow_night`
  - `resolve_builtin_mono`
  - `resolve_bad_path_warns_and_falls_back`
  - `resolve_file_path_loads_scheme`
  - `resolve_no_scheme_with_element_override`
  - `resolve_bad_parse_warns_and_falls_back`

- Config.rs test that exclusively exercised removed item:
  - `write_symbols_round_trips_preserving_other_keys`

- headless.rs: removed `use app::config::ColorsConfig` and `use app::colors::ColorScheme` imports
  (now unused). Updated `colors_scheme_swap_changes_connector_color` test to use
  `style::resolve` with a `StyleDoc` instead.

### Kept (still referenced)

- `colors::resolve_base`: live caller in `style::resolve` (crates/app/src/style.rs line 367).
  Not touched.

- `colors::ColorScheme::from_ghostty`: called from `resolve_base` (colors.rs line 400).
  The `overrides` parameter is retained because the function signature is part of the public
  API and the live path passes an empty map. The parameter itself is used internally (the
  `resolve_element` closures use it). No elements-stripping was done; the function is intact.

- All `from_ghostty` tests (`from_ghostty_connector_maps_to_palette6`, etc.) and tests for
  `parse_color_value`, `GhosttyScheme::parse`, `terminal_default_*`, `element_override_*`: kept.

## Fix 2 - DEFAULT_STYLE_TOML Symbols Section

Replaced the four hardcoded preset lines in `DEFAULT_STYLE_TOML` with an empty `[symbols]`
section. The existing `load_style_default_name_parses_builtin` test passes (it only checks
that the doc parses without error). The `resolve_empty_doc_equals_terminal_default` test
continues to confirm that an empty StyleDoc resolves to terminal defaults.

## Fix 3 - style_to_decl Invariant Comment

Added one-line doc comment to `style_to_decl` (~line 497) noting the invariant:
relies on `Style::patch` only ADDING modifiers (never removing), which holds because
every ColorScheme constructor carries REVERSED/BOLD modifiers on the relevant fields.
No code change.

## Concerns

None. All removals were confirmed dead before removal. The headless.rs test
`colors_scheme_swap_changes_connector_color` was updated rather than deleted because it
tests valid behavior (scheme-swap changes connector color) that belongs in the integration
test suite -- it just needed to use the current live API instead of the now-removed one.
