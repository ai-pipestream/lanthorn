// Test fixtures build structs by defaulting then setting a few fields, which is
// clearer than a full struct literal here. Silence the pedantic lint in tests only.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

/// The ONE lock the integration suites take around `zvm::screen::set_palette`.
///
/// The palette is process-global. Every suite that boots a story on a machine profile
/// sets it, and every suite that asserts a resolved colour depends on it, so they must
/// exclude one another — and for a long time each suite declared its **own**
/// `static PALETTE`, twenty-three of them. Within a suite that reads as the rule its
/// doc claims ("no two cases here may boot at once"); across suites it excludes
/// nothing, because `crates/app/tests/suites/` are MODULES pulled into ~14 group
/// binaries and every suite in a group shares one process.
///
/// Invisible to the local gate and fatal on CI, which is the whole point of putting it
/// here: `cargo nextest run` gives every test its own PROCESS, so no suite can observe
/// another's palette and twenty-three locks are indistinguishable from one. `cargo
/// test`, which CI runs, gives a binary's tests one process and many threads. MEASURED
/// on main: `arthurs_notices_are_the_machines_white_on_the_machines_dark_grey` read a
/// page whose r channel was 90 — `#5A5A5A`, §8.3.1's standard grey — where the Amiga's
/// is 68, because another suite in `v6_render` held the standard palette at the moment
/// it looked (SQ-0904).
///
/// A suite takes it with `static PALETTE: &Mutex<()> = &app::V6_PALETTE_LOCK;` so its
/// own call sites are unchanged. It lives in the library rather than in a test module
/// because one static per PROCESS is what correctness needs, and every group binary
/// links this crate.
pub static V6_PALETTE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take [`V6_PALETTE_LOCK`] **and** state the palette, in one call that cannot do
/// one without the other.
///
/// The lock alone only ever protected the WRITERS. `palette_lock_discipline` fails a
/// suite that calls `zvm::screen::set_palette` without taking the lock, because a
/// write is a call and a call is visible in source. The other half is a suite that
/// merely READS — one that asserts a resolved colour without installing a palette of
/// its own, and so believes whatever the last suite in its group binary happened to
/// leave behind. That is an ABSENCE, invisible both to the source check and to the
/// gate: `cargo nextest run` gives every test its own process, where the inherited
/// palette is always the default and the suite is always right. MEASURED on main
/// (SQ-0958): `v6_shogun_gameplay` asserted §8.3.1 white while its sibling
/// `v6_shogun_title_header` booted THE SAME STORY under `InterpreterProfile::IbmPc`
/// and installed the IBM YZIP table, so two of its cases read `Rgb(173, 173, 173)`
/// under `cargo test` — which is what CI runs, so main was red and the local gate
/// green.
///
/// Hence the pairing here rather than two habits kept in step by hand: a suite that
/// wants the lock must name a palette to get it, and a suite that names a palette
/// gets the lock whether it thought about the race or not. The user's rule is
/// **"no test should be written that makes an assumption about a palette it did not
/// write"**, and `Palette::Standard` is as much an assumption as any other — a suite
/// that means the default still has to say so.
///
/// ```ignore
/// let _g = app::v6_palette(zvm::screen::Palette::Standard); // held for the case
/// ```
///
/// The guard must outlive every boot, render and colour assertion in the case: the
/// palette is process-global, so dropping it early lets a sibling install a machine's
/// table between the render and the assertion about it — and, since SQ-0959, dropping
/// it also puts `Palette::Standard` back, so an early drop moves the table under the
/// case that asked for it.
pub fn v6_palette(p: zvm::screen::Palette) -> V6PaletteGuard {
    let lock = V6_PALETTE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    zvm::screen::set_palette(p);
    V6PaletteGuard { _lock: lock }
}

/// What [`v6_palette`] hands back: the shared lock, plus the undertaking to put the
/// palette back when the case ends.
///
/// # Why the guard restores at all (SQ-0959)
///
/// Taking the lock says "no one else may write the palette while I hold this". It
/// never said anything about afterwards, so the first case in a group binary to boot
/// a machine press left that machine's table installed for the WHOLE PROCESS. Every
/// later case then ran on the last writer's palette rather than on the default —
/// under `cargo test`, which is what CI runs, and only there: `cargo nextest run`
/// gives every test its own process, so the table it inherits is always `Standard`
/// and the dirt is structurally invisible. That is the SQ-0958 shape exactly, one
/// level down: the reader rule tells a suite to state its palette, and this makes the
/// state a suite inherits when it does the same one nextest would have given it.
///
/// # Why `Standard` rather than whatever was there before
///
/// Restoring the PREVIOUS value is the tempting "leave no trace" reading, and it is
/// the right one for `zvm-cli`'s `swatch`, which borrows the palette per table row
/// inside a run whose machine is a real fact. Here there is no such fact: a test
/// process has no machine, and the value a guard would find on entry is only
/// meaningful if every writer restores. Twenty-eight suites still take the raw
/// [`V6_PALETTE_LOCK`] and set the palette themselves, restoring nothing — each from
/// inside its own `boot()`, after resolving a profile the case has not seen yet, which
/// is why they cannot simply name a palette here — and restore-previous would copy
/// whichever machine's table they left through every later guard, for ever. Restoring
/// `Standard` needs no saved state, is idempotent, cleans up after the writers that do
/// not, and leaves each case starting from the same table nextest's fresh process
/// gives it, which is the property the reader rule assumes.
///
/// The palette goes back while the lock is still held: `Drop::drop` runs before the
/// struct's fields, so no other guard can be admitted into the window between.
#[must_use = "the palette is only guaranteed for as long as the guard is held, and is \
              put back the moment it is dropped"]
pub struct V6PaletteGuard {
    /// Released when this guard drops — after [`Drop::drop`] has put the palette back.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for V6PaletteGuard {
    fn drop(&mut self) {
        zvm::screen::set_palette(zvm::screen::Palette::Standard);
    }
}

/// The guard's own behaviour, asserted HERE rather than in an integration suite.
///
/// `tests/suites/` is the wrong place for it: every one of those files is compiled
/// into a group binary beside a dozen others, several of which take the raw lock and
/// set the palette themselves without restoring, so reading the table back after a
/// drop is a race against whichever sibling happens to hold the lock next. This
/// crate's own test binary has no palette writer at all — `interpreter.rs`'s cases
/// only ask a *profile* which palette it names, which touches nothing global. One
/// case rather than two, deliberately: two would race with each other for the same
/// reason.
#[cfg(test)]
mod v6_palette_guard {
    use zvm::screen::{palette, set_palette, Palette};

    /// The guard installs the palette it names, puts `Standard` back when it drops,
    /// and puts back `Standard` rather than the value it displaced.
    ///
    /// Falsified by deleting the `Drop` impl above: the second assertion then reads
    /// `Amiga`, which is exactly the dirt SQ-0959 is about.
    #[test]
    fn the_guard_installs_a_palette_and_puts_the_default_back() {
        assert_eq!(palette(), Palette::Standard, "the process starts on the default");
        {
            let _g = super::v6_palette(Palette::Amiga);
            assert_eq!(palette(), Palette::Amiga, "the guard installs what it was named");
        }
        assert_eq!(palette(), Palette::Standard, "and hands the default back on drop");

        // Now the other half of the choice: leave the table dirty the way a raw-lock
        // suite leaves it, and guard over the top. Restoring the DISPLACED value would
        // carry the IBM table forward for ever; restoring the default ends it here.
        set_palette(Palette::IbmCga);
        {
            let _g = super::v6_palette(Palette::IbmYzip);
        }
        assert_eq!(palette(), Palette::Standard, "the guard cleans up after the writer before it");
    }
}

pub mod anim;
pub mod archive;
pub mod assets;
pub mod cell_dump;
pub mod aux_store;
pub mod browser;
pub mod clipboard;
pub mod export;
pub mod history;
pub mod hints;
pub mod hint_download;
pub mod slash;
pub mod colors;
pub mod complete;
pub mod config;
pub mod config_template;
pub mod cover;
pub mod debug_panel;
/// Which files are volumes of one multi-disk release (SQ-0844).
///
/// **Re-exported, not declared** (SQ-0874): the rule moved to `cli-host` the day
/// `zvm-cli` needed it, because a CLI cannot depend on `app` and a second copy of
/// "which files form a release" is how two front-ends end up disagreeing about
/// what is on a shelf. One implementation, and every `app::disk_set::…` spelling
/// still reaches it.
pub use cli_host::disk_set;
pub mod engine;
pub mod garglk_ini;
pub mod glk_backend;
pub mod glulx_debug;
pub mod glulx_roomlock;
pub mod glulx_session;
pub mod graphics;
pub mod inline_image;
pub mod inventory;
pub mod export_dot;
pub mod export_svg;
pub mod fetch_worker;
pub mod cover_gallery;
pub mod ifdb;
pub mod ifdb_search;
pub mod ifdb_search_modal;
pub mod ifiction;
pub mod ifid;
pub mod input;
pub mod interpreter;
pub mod keymap;
pub mod launch_options;
pub mod native_sound;
pub mod layout;
pub mod list_scroll;
pub mod map_dump;
pub mod notify;
pub mod pager;
pub mod pane_drag;
pub mod pcset_store;
pub mod period;
pub mod pixel_mouse;
pub mod persist_files;
pub mod picker;
pub mod query_sweep;
pub mod reload;
pub mod render;
pub mod roomid;
pub mod scott_debug;
pub mod scott_session;
pub mod session;
pub mod state;
pub mod stderr_redirect;
pub mod storage;
pub mod style;
pub mod story_info;
pub mod styles;
pub mod symbols;
pub mod term_colors;
pub mod text_field;
pub mod textwidth;
pub mod theme;
pub mod tidy;
pub mod trace;
pub mod vfs_store;
pub mod watch;
