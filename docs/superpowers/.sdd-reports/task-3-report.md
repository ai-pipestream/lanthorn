# Task 3 Report: Overlay-open cursor guard

## STATUS: COMPLETE

## Commit SHA
(see below — committed after this file is written)

## Cargo test result
`cargo test --workspace`: 480 + 13 + 3 + 159 + 1 + 153 + 2 passed; 0 failed across all crates.

## Zero-new-warnings confirmation
`cargo build --workspace` produced no warnings.

## Files changed
- `crates/app/src/state.rs` — added `pub fn any_overlay_open(&self) -> bool` + test `any_overlay_open_reflects_state`
- `crates/app/src/render/transcript.rs` — changed cursor guard from `state.focus == Focus::Game` to `state.focus == Focus::Game && !state.any_overlay_open()` + test `render_transcript_no_cursor_when_overlay_open`

## AppState fields OR'd in any_overlay_open

1. `gallery: Option<GalleryState>` — symbol gallery modal
2. `saves: Option<SavesState>` — saves manager modal
3. `file_browser: Option<FileBrowserState>` — file browser modal
4. `config_screen: Option<ConfigScreenState>` — config screen modal
5. `verb_menu: Option<VerbMenuState>` — verb/item token-palette modal
6. `hotkey_dialog: bool` — hotkey reference dialog overlay
7. `room_panel: Option<RoomPanel>` — room info/diagnostics corner panel
8. `tidy_anim: Option<TidyAnim>` — tidy animation playback (replaces map content)
9. `prompt: Option<Prompt>` — text-entry sub-mode overlaid on map focus

## Concerns
None. All fields are clearly documented in `AppState` with comments identifying modal/overlay semantics. The `prompt` field is included because while it is not a visual dialog it is an input-capture overlay mode that changes input routing — suppressing the story cursor while a prompt is active is correct (the prompt's own input widget handles text entry). The `tidy_anim` field is included because it is an active overlay mode per the plan's "inspector/tidy" mention.
