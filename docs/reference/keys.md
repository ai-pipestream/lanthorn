<!-- generated from crates/app/src/keymap.rs (keymap::KeyMap::default) by docs_reference; do not edit by hand -->
# Key reference

The built-in key bindings, one row per binding — several keys may reach the same command. Rebind any of these under `[keymap.*]` in `config.toml`; see [config.md](config.md).

| Context | Key | Command | Description |
|---|---|---|---|
| Global | `Tab` | `toggle-focus` | switch focus between panes |
| Global | `Ctrl+S` | `save-state` | save an emulator Save State, optionally to a named slot |
| Global | `Ctrl+R` | `restore-state` | restore an emulator Save State — bare opens the saves dialog to pick one; a name restores that slot directly |
| Global | `Alt+1` | `journal-tab map` | show a tab of the Journal (the right-hand panel), revealing it if hidden; remembered per game |
| Global | `Alt+2` | `journal-tab room` | show a tab of the Journal (the right-hand panel), revealing it if hidden; remembered per game |
| Global | `Alt+3` | `journal-tab inventory` | show a tab of the Journal (the right-hand panel), revealing it if hidden; remembered per game |
| Global | `Alt+4` | `journal-tab hints` | show a tab of the Journal (the right-hand panel), revealing it if hidden; remembered per game |
| Global | `Alt+5` | `journal-tab documents` | show a tab of the Journal (the right-hand panel), revealing it if hidden; remembered per game |
| Anim | `H` | `pan-map -1 0` | pan the map by dx columns and dy rows |
| Anim | `L` | `pan-map 1 0` | pan the map by dx columns and dy rows |
| Anim | `K` | `pan-map 0 -1` | pan the map by dx columns and dy rows |
| Anim | `J` | `pan-map 0 1` | pan the map by dx columns and dy rows |
| Anim | `Shift+←` | `pan-map -1 0` | pan the map by dx columns and dy rows |
| Anim | `Shift+→` | `pan-map 1 0` | pan the map by dx columns and dy rows |
| Anim | `Shift+↑` | `pan-map 0 -1` | pan the map by dx columns and dy rows |
| Anim | `Shift+↓` | `pan-map 0 1` | pan the map by dx columns and dy rows |
| Anim | `+` | `zoom-map in` | zoom the map in/out, reset, or step by signed n |
| Anim | `=` | `zoom-map in` | zoom the map in/out, reset, or step by signed n |
| Anim | `-` | `zoom-map out` | zoom the map in/out, reset, or step by signed n |
| Anim | `←` | `anim-step back` | step the animation one frame |
| Anim | `→` | `anim-step forward` | step the animation one frame |
| Anim | `Space` | `anim-play` | toggle animation play/pause |
| Anim | `Esc` | `anim-exit` | exit the animation view |
| Anim | `Enter` | `anim-exit` | exit the animation view |
| Browser | `↑` | `move-selection 0 -1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `K` | `move-selection 0 -1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `↓` | `move-selection 0 1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `J` | `move-selection 0 1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `←` | `move-selection -1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `H` | `move-selection -1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `→` | `move-selection 1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `L` | `move-selection 1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `PgUp` | `page-selection -1` | move the browser's selection by n pages |
| Browser | `PgDn` | `page-selection 1` | move the browser's selection by n pages |
| Browser | `Ctrl+U` | `half-page-selection -1` | move the browser's selection by half a page (vim Ctrl-U/Ctrl-D) |
| Browser | `Ctrl+D` | `half-page-selection 1` | move the browser's selection by half a page (vim Ctrl-U/Ctrl-D) |
| Browser | `Home` | `select-edge first` | jump the browser's selection to the first or last game |
| Browser | `End` | `select-edge last` | jump the browser's selection to the first or last game |
| Browser | `Enter` | `play-story` | launch the selected game |
| Browser | `O` | `open-launch-options` | open the launch-options dialog for the selected game |
| Browser | `Shift+Enter` | `open-launch-options` | open the launch-options dialog for the selected game |
| Browser | `Space` | `open-story-menu` | open the per-game menu beside the selected game |
| Browser | `?` | `show-browser-keys` | show the game browser's key reference |
| Browser | `Tab` | `toggle-info-panel` | open or close the browser's game info panel |
| Browser | `I` | `toggle-info-panel` | open or close the browser's game info panel |
| Browser | `G` | `toggle-gallery` | switch the browser between the game list and the cover gallery |
| Browser | `F` | `fetch-story` | re-fetch the selected game's IFDB metadata, ignoring the cache |
| Browser | `R` | `refresh-library` | fetch IFDB metadata for every game that is missing or stale |
| Browser | `U` | `set-ifdb-url` | point the selected game at an IFDB page by hand |
| Browser | `/` | `search-ifdb` | search IFDB by title or author and download a game into this directory |
| Browser | `M` | `create-documents-folder` | create this game's documents folder (manuals, maps) beside its IFDB id |
| Browser | `Shift+U` | `open-url` | download a game from a URL into this library and open it |
| Browser | `Shift+H` | `download-hints` | download a matching InvisiClues hint file for this game (the selected one in the browser) |
| Browser | `Shift+D` | `download-documents` | choose manuals, feelies and maps listed on IFDB for this game and save them to its documents folder |
| Browser | `S` | `sort-library` | cycle the browser's sort column, keeping the direction |
| Browser | `D` | `reverse-sort` | reverse the browser's sort direction, keeping the column |
| Browser | `Ctrl+F` | `find-story` | type to filter the whole library by title, author, filename or folder |
| Browser | `Backspace` | `parent-folder` | leave the current library folder for the one above it |
| Browser | `Q` | `quit-browser` | leave the game browser |
| Browser | `Ctrl+Q` | `quit-browser` | leave the game browser |
| Browser | `Esc` | `cancel-browser` | cancel a running fetch, or leave the browser when nothing is in flight |
