# Task 1 Report: BorderStyle + Standard Border Renderer

**STATUS:** DONE

## Commit

Pending (see below).

## Files Changed

- Created: crates/app/src/render/paneframe.rs
- Modified: crates/app/src/render/mod.rs (added `pub mod paneframe;`)

## Test Result

cargo test --workspace: 789 passed; 0 failed across all crates (app: 459, babelmap: 12, headless: 3, plus other crates)

## Zero New Warnings

Confirmed: no warnings emitted by cargo test --workspace.

## Implementation Notes

- BorderStyle enum: None, Single, Double, Thick, PictureFrame (all five variants)
- parse_border_style: exact matches for "none"/"single"/"double"/"thick"/"picture-frame"; unknown input returns Single
- PaneFrame struct: area, content, top_inset all pub Rect fields
- draw_pane_frame: renders None (no glyphs, content == area), Single (U+250C/2500/2510/2502/2514/2518), Double (U+2554/2550/2557/2551/255A/255D), Thick (U+250F/2501/2513/2503/2517/251B)
- PictureFrame falls through to Single for now, per plan spec
- top_inset for bordered styles = row y, cols x+1..(x+w-1); for None = top row of full area
- All three verbatim test functions from the plan pass

## Concerns

None.
