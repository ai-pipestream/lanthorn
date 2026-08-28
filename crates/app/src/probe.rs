//! Fork the live game into a silent, disposable copy and ask it a question
//! (SQ-1121, and the seam SQ-0785 and SQ-1043 were scheduled to share).
//!
//! There was a first attempt at this — `return_probe.rs` on the abandoned commit
//! `5270882c` (2026-07-30), which probed the way back after a move to close
//! one-way gaps in the automap. It was never on a branch and never merged. Two
//! things from it are kept here because they were right: booting a shadow from
//! the STORY BYTES rather than cloning the engine, and restoring between every
//! candidate so probes stay independent. The rest of it was automapping wired
//! into a probe rather than a probe with a question in it, so this is the
//! generalisation SQ-0785's later note asked for rather than a re-landing.
//!
//! A **shadow** is a second [`Engine`] running the same story, driven from a
//! snapshot of the live one. Commands typed into it never reach the screen, the
//! filesystem, the sound card or the archive; when the answer has been read off
//! it, the shadow is restored back over and reused for the next question. The
//! live session is never touched — not saved, not restored, not stepped. That
//! separation is the point: restoring under a running game is the hazard
//! SQ-0587/0588 documented, because the game never learns it happened.
//!
//! # Why a shadow can answer anything at all
//!
//! [`Engine::save_state`] / [`Engine::restore_state`] are engine-neutral and
//! already in the trait — the host Save State family, not the game's own
//! `@save`. So "what would happen if I typed this?" is answerable by typing it
//! somewhere the answer costs nothing.
//!
//! # How this story says no, discovered rather than assumed
//!
//! The interesting question is almost never *did the parser understand this* —
//! that is a static fact about the dictionary, and [`crate::vocab`] answers it
//! without running anything. It is *did anything happen*, and every family of
//! game phrases its refusals differently (`[I don't know the word "x".]`, `You
//! can't see any such thing.`, `You use word(s) I don't know!`, `You don't have
//! that!`). A detector built on those strings is broken by the next game and
//! unusable outside English.
//!
//! So [`Refusals`] is **learned from the story**, in the shadow, by running
//! deliberate nonsense beside the real question and reading what comes back.
//! Two shapes of control, and each is only believed under a condition:
//!
//! * [`ProbeRun::refusal_from`] — a command the parser cannot have understood at
//!   all (a word this story's dictionary does not hold). Every sentence of the
//!   reply is a refusal.
//! * [`ProbeRun::refusal_from_pair`] — the same command twice with two different
//!   nouns in it. Believed **only when the two replies are the same sentence**
//!   once their own nouns are struck out, which is what tells a generic refusal
//!   from two coincidentally similar successes.
//!
//! Both additionally require the control to have left the world unchanged
//! ([`WorldPrint`]): a control that moved an object *did* something, so whatever
//! it printed is not this story's way of saying no.
//!
//! # The controls belong to the ROOM, not to the session
//!
//! This is the thing that is easy to get wrong, and it was got wrong once here.
//! A refusal fingerprint learned at the start of a session and reused all game
//! is measuring the wrong room: Zork I answers `light rug` with `You don't have
//! that!` in the field and `You don't have the carpet.` in the living room, and
//! `light lamp` with `You don't have that!` outside the house and `(Taken) The
//! brass lantern is now on.` inside it. Same story, same command, different
//! answer — because scope is where the player is standing.
//!
//! So a caller runs its controls **in the same `run` as its questions**, from
//! the same snapshot, and reads the signature off that run. Nothing is cached
//! between turns.
//!
//! # What it still cannot tell you
//!
//! A refusal that no control provokes reads as a success. And a game that
//! consumes randomness may answer the shadow and the live session differently:
//! a probe is evidence, never a guarantee.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use crate::engine::Engine;

/// How long one `run` may spend in the shadow before it gives up and reports
/// nothing. The offer it feeds appears between the player's command and the
/// game's reply, so a probe that overruns must go quiet rather than stall the
/// turn; the caller falls back to whatever it would have said unvetted.
pub const BUDGET: Duration = Duration::from_millis(400);

/// How long the shadow's own BOOT may take before this story is written off as
/// too slow to probe.
///
/// Separate from [`BUDGET`] because it is a different cost with a different
/// shape: it is paid once a session rather than once an offer, and no cap can
/// stop it being paid — a boot cannot be measured until it has happened. So the
/// cap's job is only to decide whether to pay it a SECOND time, and it is
/// generous: Counterfeit Monkey's initialisation is millions of opcodes even
/// accelerated and takes over two seconds here, which is affordable once and
/// nowhere near affordable per turn.
pub const BOOT_BUDGET: Duration = Duration::from_millis(1500);

/// The most commands one `run` will type into the shadow, whatever the caller
/// asks for. A belt to the budget's braces: a story that answers instantly can
/// still not be walked through a hundred candidates. Sized for a question plus
/// its controls — [`crate::vocab`] asks three or four things and runs two
/// controls for each.
pub const MAX_PROBES: usize = 16;

// ── The recipe a shadow is built from ───────────────────────────────────────

/// Everything a silent copy of this story needs in order to exist: the story's
/// own bytes and the handful of boot facts that change how it runs.
///
/// One value rather than six parameters, deliberately (CLAUDE.md's refactoring
/// policy): a caller who supplied a subset would get a shadow that boots and
/// answers *plausibly* on a different machine than the live game, and nothing
/// downstream could tell.
///
/// **What it deliberately does NOT carry, and why.** A v6 launch resolves a
/// whole [`crate::machine_boot::MachineBoot`] — the screen the story is told it
/// has, the art scale, the character cell, §8.3.3's colour pair — and none of it
/// is here. Those facts change how a story is DRAWN; a shadow is only ever read
/// as text, and every comparison made against it (a candidate's reply against a
/// control's) is between two replies from the SAME shadow, so a shadow that
/// wraps differently from the live screen still answers the question asked of
/// it. If a caller ever needs a shadow's GEOMETRY, this is the value that has to
/// grow a `MachineBoot` — do that rather than adding the one field you happen to
/// want.
#[derive(Clone, Debug)]
pub struct ShadowRecipe {
    /// The story file exactly as it was loaded, before any container was
    /// unwrapped — `hints::extract_story` does that again for the shadow.
    pub story_bytes: std::sync::Arc<Vec<u8>>,
    /// Z-machine: whether the game may pick its own colours. Irrelevant to what
    /// a probe reads, but a boot fact, and a shadow that differs from the live
    /// game in any boot fact is a different game.
    pub honor_game_colours: bool,
    /// Z-machine header byte $1E.
    pub interpreter_number: Option<u8>,
    /// The seed the story's randomness starts from, so the shadow rolls the
    /// same dice the live session did.
    pub random_seed: Option<u32>,
    /// Glulx: whether the accelerated Glk functions are installed. Off would
    /// make the shadow's boot minutes long on Counterfeit Monkey.
    pub acceleration: bool,
    /// Glulx: the virtual screen the shadow lays its windows out on.
    pub screen: (u32, u32),
}

// ── What a probe hands back ─────────────────────────────────────────────────

/// A fingerprint of as much of the game world as an engine will show us:
/// where the player is, what is in the room, and what they are carrying.
///
/// Deliberately a hash and not a description — nothing reads it except to ask
/// whether it is the same as another one. `None` for an engine with no
/// introspection, which is an honest "cannot tell", not "nothing changed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldPrint(Option<u64>);

impl WorldPrint {
    /// Read the world as `engine` currently has it.
    pub fn of(engine: &dyn Engine) -> WorldPrint {
        let Some(intro) = engine.introspect() else { return WorldPrint(None) };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let here = engine.current_location().map(|l| l.number);
        here.hash(&mut h);
        let player = intro.player_object();
        player.hash(&mut h);
        if let Some(room) = here {
            let mut v = intro.room_objects_excluding(room, player);
            v.sort();
            v.hash(&mut h);
        }
        if let Some(p) = player {
            let mut v = intro.contents(p);
            v.sort();
            v.hash(&mut h);
        }
        WorldPrint(Some(h.finish()))
    }

    /// True when both prints are readable and they differ — a changed world.
    /// Two unreadable prints are not "the same"; they are not an answer.
    pub fn differs_from(self, other: WorldPrint) -> bool {
        match (self.0, other.0) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }
}

/// What one command did in the shadow.
#[derive(Clone, Debug)]
pub struct ProbeStep {
    /// The command as it was typed into the shadow.
    pub command: String,
    /// Everything the story printed in reply, and nothing else.
    pub reply: String,
    /// The room the shadow ended the command in, when the engine can say.
    pub location: Option<u16>,
    /// The world after the command.
    pub world: WorldPrint,
    /// The story ended.
    pub quit: bool,
    /// The command tried to reach outside the shadow — the game's own
    /// `@save`/`@restore`, or a Glk file prompt. It was refused and the step is
    /// worthless, but nothing escaped.
    pub escaped: bool,
}

/// One `run`: the world the questions were asked from, and what each answered.
#[derive(Clone, Debug)]
pub struct ProbeRun {
    /// The world at the snapshot every step started from.
    pub baseline: WorldPrint,
    /// One entry per command, in the order they were given.
    pub steps: Vec<ProbeStep>,
}

// ── This story's own signature of failure ───────────────────────────────────

/// The sentences this story prints when it has understood nothing and done
/// nothing — discovered, never assumed. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct Refusals {
    sigs: BTreeSet<String>,
}

impl Refusals {
    /// True when `reply` to `command` is one of the refusals this story taught
    /// us. The command's own words are struck out of both sides, so the noun
    /// the sentence names does not have to match.
    ///
    /// Only the **first** sentence is compared, and that is load-bearing rather
    /// than an optimisation. Some engines put furniture in every reply — a Scott
    /// Adams turn ends with `Tell me what to do ?`, which is therefore inside
    /// the refusal a control taught us AND inside every success — so a rule of
    /// "any sentence matches" classifies every reply as a refusal and the offer
    /// falls silent on a whole engine. A refusal is what the story says FIRST;
    /// what follows it is a prompt, a daemon, or the lamp getting dimmer.
    pub fn says_no(&self, reply: &str, command: &str) -> bool {
        signature(reply, command).first().is_some_and(|s| self.sigs.contains(s))
    }

    /// Fold another reading of the same run in.
    pub fn merge(&mut self, other: Refusals) {
        self.sigs.extend(other.sigs);
    }

    /// True when nothing was learned — the controls taught nothing believable.
    /// A caller must not read silence here as "everything succeeded"; it means
    /// the run cannot answer.
    pub fn is_empty(&self) -> bool {
        self.sigs.is_empty()
    }

    /// The normalised sentences, for tests and diagnostics.
    pub fn sentences(&self) -> impl Iterator<Item = &str> {
        self.sigs.iter().map(String::as_str)
    }
}

impl ProbeRun {
    /// The step at `i` was a command the parser cannot have understood, so
    /// **everything** it printed is this story saying no.
    ///
    /// Empty when that step did something after all — moved an object, ended the
    /// story, reached for a file — because then its words describe an action,
    /// not a refusal.
    pub fn refusal_from(&self, i: usize) -> Refusals {
        let Some(step) = self.steps.get(i).filter(|s| self.inert(s)) else {
            return Refusals::default();
        };
        Refusals { sigs: signature(&step.reply, &step.command).into_iter().collect() }
    }

    /// Steps `a` and `b` are the same command carrying two different nouns.
    /// Their reply is a refusal **only if it is the same sentence** once each
    /// one's own noun is struck out — otherwise the two are describing two
    /// different things that happened, and neither is a refusal.
    pub fn refusal_from_pair(&self, a: usize, b: usize) -> Refusals {
        let (Some(x), Some(y)) = (self.steps.get(a), self.steps.get(b)) else {
            return Refusals::default();
        };
        if !self.inert(x) || !self.inert(y) {
            return Refusals::default();
        }
        let sx = signature(&x.reply, &x.command);
        if sx.is_empty() || sx != signature(&y.reply, &y.command) {
            return Refusals::default();
        }
        Refusals { sigs: sx.into_iter().collect() }
    }

    /// Did the step at `i` do anything, as far as this run can tell?
    ///
    /// A changed world settles it whatever was printed. An unchanged one settles
    /// nothing — `examine` and `look` legitimately change nothing — so the words
    /// decide, against a signature this same run discovered.
    pub fn did_something(&self, i: usize, refusals: &Refusals) -> bool {
        let Some(step) = self.steps.get(i) else { return false };
        // Ending the story is unambiguously something happening — a mistyped
        // `quit` is still a `quit`, and a player who typed `quti` meant it. Note
        // this is the OPPOSITE reading from [`Self::inert`]: a control that quit
        // teaches nothing, because the words it printed are a farewell rather
        // than a refusal.
        if step.quit {
            return true;
        }
        if step.escaped || step.reply.trim().is_empty() {
            return false;
        }
        step.world.differs_from(self.baseline) || !refusals.says_no(&step.reply, &step.command)
    }

    /// A step that reached nothing and changed nothing — the only kind whose
    /// words are safe to read as a refusal.
    fn inert(&self, step: &ProbeStep) -> bool {
        !step.escaped && !step.quit && !step.world.differs_from(self.baseline)
    }
}

/// One reply, reduced to the sentences that carry its *shape*: lowercased,
/// punctuation and digits dropped, and every word of the command that produced
/// it struck out — which is what makes `You can't see any lamp here!` and `You
/// can't see any sword here!` the same sentence, and what removes the quoted
/// word from `[I don't know the word "lanturn".]`.
fn signature(reply: &str, command: &str) -> Vec<String> {
    let typed: BTreeSet<String> = command
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    for sentence in reply.split(['.', '!', '?', '\n']) {
        let words: Vec<String> = sentence
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .filter(|w| !typed.contains(w))
            .collect();
        if !words.is_empty() {
            out.push(words.join(" "));
        }
    }
    out
}

// ── The seam ────────────────────────────────────────────────────────────────

/// A silent copy of the live game, kept between questions.
///
/// Lives on [`crate::state::AppState`] because it is per-session state with a
/// lazy, expensive body: the shadow is booted the first time anything asks a
/// question and reused for every later one, so a story whose initialisation
/// costs millions of opcodes pays that once. A boot that fails disables the
/// seam for the session rather than being retried every turn.
#[derive(Default)]
pub struct ShadowProbe {
    recipe: Option<ShadowRecipe>,
    shadow: Option<Box<dyn Engine>>,
    /// The shadow could not be built; stop trying.
    broken: bool,
    /// This story answers too slowly to be probed between a command and its
    /// reply, so the seam switches itself off for the session (SQ-1121).
    ///
    /// Measured rather than guessed, and latched rather than retried: a heavy
    /// Glulx story costs seconds per shadow turn, and a budget that merely cuts
    /// each offer short would spend that on EVERY failed word for the whole
    /// session and show nothing extra for it. Paying it once and then declining
    /// is the only shape that bounds the cost.
    too_slow: bool,
    /// Commands typed into a shadow this session, and the time they took —
    /// the numbers `/info` would want, and the ones that say whether this is
    /// affordable on a given story.
    pub probes: u32,
    /// Total time spent inside `run`, boot included.
    pub spent: Duration,
}

impl std::fmt::Debug for ShadowProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShadowProbe")
            .field("armed", &self.recipe.is_some())
            .field("booted", &self.shadow.is_some())
            .field("broken", &self.broken)
            .field("too_slow", &self.too_slow)
            .field("probes", &self.probes)
            .field("spent", &self.spent)
            .finish()
    }
}

impl ShadowProbe {
    /// Give the seam what it needs to build a shadow. Until this is called
    /// there is no probing — every test-built `AppState` is in that state, and
    /// so is a session whose story bytes were never kept.
    pub fn arm(&mut self, recipe: ShadowRecipe) {
        self.recipe = Some(recipe);
        self.shadow = None;
        self.broken = false;
        self.too_slow = false;
    }

    /// True when a question could be asked — armed, not already given up on, and
    /// not on a story that has proved too slow to ask.
    pub fn is_armed(&self) -> bool {
        self.recipe.is_some() && !self.broken && !self.too_slow
    }

    /// True when this story answered, but too slowly to keep asking. Distinct
    /// from never having been armed: something WAS measured.
    pub fn is_too_slow(&self) -> bool {
        self.too_slow
    }

    /// Type each of `commands` into a silent copy of `live`, every one of them
    /// from the same snapshot, and report what each did.
    ///
    /// `None` when there is no shadow to ask — unarmed, un-bootable, the live
    /// engine mid-`@save`, or the budget spent. That is "no answer", never "no".
    pub fn run(&mut self, live: &dyn Engine, commands: &[String]) -> Option<ProbeRun> {
        if !self.is_armed() || commands.is_empty() || commands.len() > MAX_PROBES {
            return None;
        }
        // Snapshotting a suspended VM would capture it mid-file-operation, and
        // the shadow would resume into an I/O request nobody can answer.
        if live.is_saveload_pending() {
            return None;
        }
        let started = Instant::now();
        let save = live.save_state();
        let baseline = WorldPrint::of(live);

        if self.shadow.is_none() {
            let recipe = self.recipe.clone()?;
            let booting = Instant::now();
            match boot_shadow(&recipe) {
                Ok(e) => self.shadow = Some(e),
                Err(_) => {
                    self.broken = true;
                    self.spent += started.elapsed();
                    return None;
                }
            }
            // The boot is over; the only question left is whether to ever pay it
            // again. Answer it here rather than letting the per-offer budget
            // discover the same thing on every failed word for the rest of the
            // session.
            if booting.elapsed() > BOOT_BUDGET {
                self.too_slow = true;
                self.shadow = None;
                self.spent += started.elapsed();
                return None;
            }
        }
        let shadow = self.shadow.as_mut()?;

        let mut steps = Vec::with_capacity(commands.len());
        let mut overran = false;
        for command in commands {
            if started.elapsed() > BUDGET {
                overran = true;
                break;
            }
            if shadow.restore_state(&save).is_err() {
                // A shadow that will not take the live state is no shadow.
                self.shadow = None;
                self.broken = true;
                self.spent += started.elapsed();
                return None;
            }
            let _ = shadow.take_transcript();
            let _ = shadow.take_transcript_elems();
            let result = shadow.submit(command);
            self.probes += 1;
            // ISOLATION. Nothing typed in here may reach a file. A game that
            // suspends for its own `@save`/`@restore`, or asks Glk for a
            // filename, is answered "that failed" so the VM unwinds inside the
            // shadow, and the step is thrown away.
            let escaped = result.pending_io.is_some() || shadow.pending_filename().is_some();
            if escaped {
                unwind_io(shadow.as_mut(), result.pending_io);
            }
            steps.push(ProbeStep {
                command: command.clone(),
                reply: result.transcript.clone(),
                location: result.location.as_ref().map(|l| l.number),
                world: WorldPrint::of(&**shadow),
                quit: result.quit,
                escaped,
            });
        }

        // A shadow the probe QUIT is dead, and restoring memory under it does
        // not bring it back — the next `submit` would return nothing and the
        // run after this one would silently read every reply as empty. Throw it
        // away and let the next question boot a fresh one. (Found by `quti` on
        // a Scott story: the shadow quit, and the very next offer went unvetted
        // with no sign anything was wrong.)
        if steps.iter().any(|s| s.quit) {
            self.shadow = None;
        } else {
            // Otherwise leave the shadow on the snapshot rather than on the last
            // probe's aftermath, so a shadow that is never asked again is
            // holding a state the live game actually reached.
            let _ = shadow.restore_state(&save);
            let _ = shadow.take_transcript();
        }

        // A run that ran out of budget is a run whose caller cannot use the
        // answer, and the next one will overrun in the same place. Latch it.
        if overran {
            self.too_slow = true;
        }

        self.spent += started.elapsed();
        (!steps.is_empty()).then_some(ProbeRun { baseline, steps })
    }
}

/// Answer whatever host I/O the shadow suspended on with a failure, so the VM
/// resumes and unwinds *inside* the shadow instead of sitting suspended.
///
/// This is the one place a probe could have reached the filesystem, so it is
/// answered rather than merely detected: an in-game `@save` is told the write
/// failed and an in-game `@restore` that the player cancelled, and a Glk
/// `create_by_prompt` gets no filename. The game then prints its own "Failed."
/// and carries on, in a copy that is about to be overwritten anyway.
fn unwind_io(shadow: &mut dyn Engine, io: Option<crate::session::PendingIo>) {
    if shadow.pending_filename().is_some() {
        let _ = shadow.resume_filename(None);
    }
    match io {
        Some(crate::session::PendingIo::Save) => {
            let _ = shadow.resume_save(false);
        }
        Some(crate::session::PendingIo::Restore) => {
            let _ = shadow.resume_restore(None);
        }
        None => {}
    }
}

/// Boot a silent, disposable engine for the same story.
///
/// Everything that could reach outside the process is off: no sound, no
/// graphics, no Blorb, no persistent store (an empty `game_dir`, so the game's
/// own fixed-name Glk saves auto-fail) and an empty file VFS, so nothing the
/// live session cached is visible and nothing the shadow writes survives it.
fn boot_shadow(recipe: &ShadowRecipe) -> Result<Box<dyn Engine>, String> {
    match crate::hints::extract_story(recipe.story_bytes.as_ref().clone())
        .map_err(|e| e.to_string())?
    {
        crate::hints::LoadedStory::ZCode(bytes) => {
            let s = crate::session::GameSession::new_with_trace(
                bytes,
                recipe.honor_game_colours,
                false, // sound unavailable: the story is told there is no sound card
                recipe.interpreter_number,
                false,
                Vec::new(),
                None,
                None,
                None,
            )
            .map_err(|e| format!("{e:?}"))?;
            Ok(Box::new(s))
        }
        crate::hints::LoadedStory::Glulx(bytes) => {
            let s = crate::glulx_session::GlulxSession::new(
                bytes,
                recipe.screen.0,
                recipe.screen.1,
                recipe.acceleration,
                false, // graphics
                false, // sound
                (8, 16),
                None,  // no picture Blorb
                &[],   // empty VFS: no sidecar, live or otherwise
            )
            .map_err(|e| format!("{e:?}"))?;
            Ok(Box::new(s))
        }
        crate::hints::LoadedStory::Scott(bytes) => {
            let s = crate::scott_session::ScottSession::new_with_trace(
                bytes,
                None,
                false,
                recipe.random_seed,
            )?;
            Ok(Box::new(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_strikes_out_the_words_the_command_supplied() {
        assert_eq!(
            signature("You can't see any lamp here!", "light lamp"),
            vec!["you can t see any here"]
        );
        assert_eq!(
            signature("You can't see any sword here!", "light sword"),
            vec!["you can t see any here"],
            "two nouns, one sentence — which is what makes the fingerprint portable"
        );
    }

    #[test]
    fn a_quoted_word_leaves_with_the_command_that_carried_it() {
        assert_eq!(
            signature("[I don't know the word \"lanturn\".]", "take lanturn"),
            vec!["i don t know the word"]
        );
    }

    #[test]
    fn refusals_recognise_the_shape_and_not_the_noun() {
        let mut r = Refusals {
            sigs: signature("You can't see any zzqx here!", "take zzqx").into_iter().collect(),
        };
        assert!(r.says_no("You can't see any lamp here!", "light lamp"));
        assert!(!r.says_no("The brass lantern is now on.", "light lamp"));
        assert!(!r.is_empty());
        r.merge(Refusals {
            sigs: signature("You don't have that!", "light lamp").into_iter().collect(),
        });
        assert!(r.says_no("You don't have that!", "light sword"), "merged readings both count");
    }

    /// A Scott Adams turn ends with `Tell me what to do ?`, so that sentence is
    /// inside every reply the engine ever gives — the refusals a control teaches
    /// AND the successes. Matching on any sentence would silence the offer on
    /// the whole engine; matching on the first does not.
    #[test]
    fn a_prompt_that_rides_every_reply_does_not_make_every_reply_a_refusal() {
        let refused = "You use word(s) I don't know!\n\nTell me what to do ?";
        let worked = "OK.\n\nTell me what to do ?";
        let r = Refusals { sigs: signature(refused, "zqxwvj").into_iter().collect() };
        assert!(
            r.sentences().any(|s| s == "tell me what to do"),
            "the prompt IS in the signature — that is the situation being handled"
        );
        assert!(r.says_no(refused, "zqxwvj"));
        assert!(!r.says_no(worked, "take lamp"), "a success wearing the same prompt");
    }

    #[test]
    fn an_unlearned_signature_says_nothing_rather_than_no() {
        let r = Refusals::default();
        assert!(r.is_empty());
        assert!(!r.says_no("You can't see any lamp here!", "light lamp"));
    }

    #[test]
    fn an_unreadable_world_is_not_an_unchanged_one() {
        let blind = WorldPrint(None);
        assert!(!blind.differs_from(blind));
        assert!(!blind.differs_from(WorldPrint(Some(1))));
        assert!(WorldPrint(Some(1)).differs_from(WorldPrint(Some(2))));
        assert!(!WorldPrint(Some(1)).differs_from(WorldPrint(Some(1))));
    }

    #[test]
    fn an_unarmed_probe_asks_nothing() {
        let p = ShadowProbe::default();
        assert!(!p.is_armed());
        assert_eq!(p.probes, 0, "an unarmed probe has typed nothing");
    }
}
