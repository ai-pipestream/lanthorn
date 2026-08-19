//! Sounds a release disk carries natively, checked against a rendition of the same
//! sounds that somebody else made (SQ-0907).
//!
//! Two Infocom games use sound: *The Lurking Horror* and *Sherlock*. On the Amiga
//! their disks hold a `Sound/` directory — an `sN.nam` index per Z-machine effect,
//! naming a sample and a pitch file — and the samples are an Infocom container that
//! nothing else reads. The Macintosh compilation lays the same sounds out as
//! `/MAC/SOUND`, with the pitch inlined into the index.
//!
//! **A format worked out from bytes alone is a guess.** `stories/Lurking.blb` and
//! `stories/Sherlock.blb` are these same effects wrapped as Blorb `Snd ` resources in
//! AIFF, by a third party, and they are what turn this from reverse-engineering into
//! verification: the frames must be byte-identical, and — since SQ-0923 read the pitch
//! model out of the Amiga interpreter rather than guessing it from the corpus — the
//! rates must agree too, which they do on 27 of the 29 effects the two Blorbs carry.
//! That pair of checks is the whole point of the suite and is why the decoder is
//! trusted at all.
//!
//! Fixtures are gitignored, so every case skips vacuously when one is absent.

use std::path::{Path, PathBuf};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn treasures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../treasures")
}

/// The Amiga Lurking Horror, on the disk of the *Lost Treasures* set that carries it.
fn lurking() -> Option<PathBuf> {
    let p = treasures_dir().join("Lost Treasures of Infocom, The_Disk4.adf");
    p.is_file().then_some(p)
}

/// One rendered sound out of the reference Blorb.
struct Rendered {
    rate: u32,
    frames: u32,
    samples: Vec<u8>,
}

/// Every AIFF `Snd ` in a Blorb, by resource number.
fn blorb_sounds(path: &Path) -> Option<std::collections::HashMap<u32, Rendered>> {
    let raw = std::fs::read(path).ok()?;
    let b = blorb::Blorb::parse(raw).ok()?;
    let mut out = std::collections::HashMap::new();
    for r in b.resources().iter().filter(|r| &r.usage == b"Snd ") {
        let Some((bytes, _)) = b.sound(r.number) else { continue };
        let (mut p, mut comm, mut ssnd) = (12usize, None, None);
        while p + 8 <= bytes.len() {
            let id = &bytes[p..p + 4];
            let len = u32::from_be_bytes([bytes[p + 4], bytes[p + 5], bytes[p + 6], bytes[p + 7]]) as usize;
            let body = bytes.get(p + 8..p + 8 + len)?;
            if id == b"COMM" {
                let frames = u32::from_be_bytes([body[2], body[3], body[4], body[5]]);
                // AIFF states its rate as an 80-bit extended float.
                let exp = u16::from_be_bytes([body[8], body[9]]) as i32;
                let man = u64::from_be_bytes(body[10..18].try_into().ok()?);
                let rate = ((man as f64) * 2f64.powi(exp - 16383 - 63)).round() as u32;
                comm = Some((rate, frames));
            } else if id == b"SSND" {
                ssnd = Some(body[8..].to_vec());
            }
            p += 8 + len + (len & 1);
        }
        if let (Some((rate, frames)), Some(samples)) = (comm, ssnd) {
            out.insert(r.number, Rendered { rate, frames, samples });
        }
    }
    Some(out)
}

/// **The decoder's samples are the Blorb's samples, byte for byte.**
///
/// This is the case that makes the container understood rather than guessed. For
/// every effect the two sources share, the frame count and every sample byte must
/// agree — and they do, on all fourteen: 3, 4, 6–13, 15–18.
///
/// **And the rates are the disk's, shifted by whole semitones** — which is the
/// finding that overturned this case's first reading (SQ-0912).
///
/// Eight of the fourteen rates disagree with the disk's header, always lower, and
/// that was first recorded here as one person's rendition diverging from the machine.
/// It is not: every disagreement is an exact number of equal-tempered semitones, and
/// disassembling the Amiga interpreter Infocom shipped shows `ioa_Period` computed
/// from `1.05946309` (2^(1/12)), `1000.0` and `3579.49` (the NTSC Amiga clock in kHz).
/// So the Blorb's author baked a pitch in, which is the only way to express a pitched
/// sample as plain AIFF.
///
/// **Whose pitch is a separate question, and this suite does not follow the Blorb.**
/// The shifts here fit `note − 74` — a fixed reference the disks do not state
/// anywhere — while the interpreter takes the difference of the pitch file's OWN two
/// notes, which is a unison on every Amiga file and therefore no shift at all. The
/// two models disagree, and this case asserts only the part they share: that the gap
/// is a whole number of semitones. See `blorb::infocom_sound::Pitch`.
///
/// Asserted as the relation, over all fourteen: `12·log(disk ÷ blorb) / log(2)` is a
/// whole number. Thirteen come out at 8.000, 1.000, 7.000 or 0.000 to three decimals.
/// Effect 17 is the exception at 0.896 and is pinned as such rather than smoothed —
/// its disk rate of 32910 Hz is past what Paula can clock a channel at, so something
/// clamped, and knowing WHICH is worth more than an assertion that tolerates it.
#[test]
fn the_decoded_samples_are_the_blorbs_samples() {
    let (Some(disk), blb) = (lurking(), stories_dir().join("Lurking.blb")) else {
        eprintln!("SKIP: gitignored Lost Treasures disk 4 absent");
        return;
    };
    if !blb.is_file() {
        eprintln!("SKIP: gitignored stories/Lurking.blb absent");
        return;
    }
    let Some(reference) = blorb_sounds(&blb) else { panic!("Lurking.blb should parse") };
    let ours = app::native_sound::from_medium(&disk);
    assert_eq!(ours.len(), 14, "the disk states fourteen effects");
    assert_eq!(reference.len(), 14, "and the Blorb renders the same fourteen");

    let mut same_rate = 0;
    for (&effect, sound) in &ours {
        let want = reference
            .get(&u32::from(effect))
            .unwrap_or_else(|| panic!("effect {effect} is on the disk but not in the Blorb"));
        assert_eq!(
            sound.frames as u32, want.frames,
            "effect {effect}: frame count disagrees with the Blorb",
        );
        let ssnd_at = sound.aiff.len() - sound.frames - usize::from(sound.frames % 2 == 1);
        assert_eq!(
            &sound.aiff[ssnd_at..ssnd_at + sound.frames],
            &want.samples[..],
            "effect {effect}: the samples are NOT the Blorb's — the container is not understood",
        );
        if sound.rate == want.rate {
            same_rate += 1;
        }
    }
    // The rates themselves are `the_pitch_model_reproduces_both_blorbs`' business, and
    // it asserts them exactly rather than as a whole-semitone relation. What is left
    // here is the container: the frames and the bytes.
    assert_eq!(same_rate, 13, "thirteen of the fourteen agree outright; effect 17 is capped");
}

/// The index is what carries the effect numbers, and a naming convention is not a
/// substitute for it.
///
/// *Sherlock* is the fixture that proves it. Its samples are called `armor`,
/// `growl`, `splash`, `violin.bin` — nothing resembling `sN.dat` — and effects 11,
/// 12 and 13 all name the SAME `heart` sample against three different pitch files. A
/// reader keyed on filenames finds nothing on this disk at all, and one that ignored
/// the index would collapse three effects into one.
#[test]
fn sherlocks_effects_come_from_the_index_not_from_filenames() {
    let p = stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf");
    if !p.is_file() {
        eprintln!("SKIP: gitignored Sherlock floppy absent");
        return;
    }
    let sounds = app::native_sound::from_medium(&p);
    let mut effects: Vec<u16> = sounds.keys().copied().collect();
    effects.sort_unstable();
    assert_eq!(
        effects,
        (3..=17).collect::<Vec<u16>>(),
        "Sherlock states effects 3 through 17 with no gaps",
    );
    assert!(
        sounds.values().all(|s| !s.name.starts_with('s') || !s.name.ends_with(".dat")),
        "not one of these is named after its effect number: {:?}",
        sounds.values().map(|s| s.name.as_str()).collect::<Vec<_>>(),
    );
    for e in [11u16, 12, 13] {
        assert_eq!(sounds[&e].name, "heart", "effect {e} shares one sample at its own pitch");
    }
    assert_eq!(
        sounds[&11].frames, sounds[&13].frames,
        "the shared sample really is the same bytes, not two files that happen to be named alike",
    );
}

/// Every sound the medium offers is a complete, well-formed AIFF the host can
/// decode — the shape `play_turn_sounds` hands to the mixer.
#[test]
fn every_decoded_sound_is_a_playable_aiff() {
    let mut ran = 0;
    for p in [
        lurking(),
        stories_dir()
            .join("Sherlock - The Riddle of the Crown Jewels.adf")
            .is_file()
            .then(|| stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf")),
    ]
    .into_iter()
    .flatten()
    {
        for s in app::native_sound::from_medium(&p).values() {
            let a = &s.aiff;
            assert_eq!(&a[0..4], b"FORM", "{} #{}", s.name, s.effect);
            assert_eq!(&a[8..12], b"AIFF", "{} #{}", s.name, s.effect);
            let form = u32::from_be_bytes([a[4], a[5], a[6], a[7]]) as usize;
            assert_eq!(form, a.len() - 8, "{} #{}: FORM length covers the file", s.name, s.effect);
            assert!(s.rate >= 4000 && s.rate <= 48000, "{} #{}: rate {} Hz", s.name, s.effect, s.rate);
            assert!(s.frames > 0, "{} #{}: no samples", s.name, s.effect);
            ran += 1;
        }
    }
    if lurking().is_some() {
        assert!(ran > 0, "fixtures are present but nothing was decoded");
    }
}

/// **The pitch model reproduces both Blorbs, effect for effect** — which is what
/// overturned SQ-0912's reading that the Amiga never bends a sound (SQ-0923).
///
/// The rate a sound plays at is `rate · 2^(((note − transposition) − base)/12)`, where
/// `note` and its channel's transposition come from the `.mid` and `base` comes from
/// the SAMPLE's header — the low byte of the flags word, which this suite recorded as
/// not understood for as long as the unison reading stood. See
/// `blorb::infocom_sound::Pitch` for the disassembly it was read out of.
///
/// **This case is the corpus-wide check, and it is why the model is trusted.**
/// `stories/Lurking.blb` and `stories/Sherlock.blb` are a third party's renderings of
/// these same effects, and the decoded rate matches them EXACTLY on 27 of the 29
/// effects they carry: all eighteen the model leaves unbent, and all nine it bends.
/// The old reading agreed on the eighteen and disagreed on all nine — the shape of a
/// model that is right only when the answer happens to be zero.
///
/// Two effects are excluded, both known-anomalous before this quest:
///
/// * *Lurking Horror* 17 states 32910 Hz, which needs a Paula period of 109 — past the
///   hardware's minimum of 124 before any bend is applied. It is capped, and the Blorb
///   carries its own guess at what that sounded like.
/// * *Sherlock* 13's Blorb entry is a differently-trimmed take — 13,989 frames against
///   its siblings' 13,999 — and its rate matches note 72 where the disk says 79. Two
///   independent signs of one botched entry, so it is not evidence either way.
#[test]
fn the_pitch_model_reproduces_both_blorbs() {
    let sherlock = stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf");
    let cases: Vec<(PathBuf, PathBuf, u16)> = [
        lurking().map(|d| (d, stories_dir().join("Lurking.blb"), 17u16)),
        sherlock.is_file().then(|| (sherlock.clone(), stories_dir().join("Sherlock.blb"), 13u16)),
    ]
    .into_iter()
    .flatten()
    .filter(|(_, b, _)| b.is_file())
    .collect();
    if cases.is_empty() {
        eprintln!("SKIP: gitignored Amiga sound floppies or their Blorbs absent");
        return;
    }
    let (mut agreed, mut bent) = (0, 0);
    for (disk, blb, anomaly) in &cases {
        let sounds = app::native_sound::from_medium(disk);
        let rendered = blorb_sounds(blb).expect("the Blorb parses");
        for (effect, s) in &sounds {
            let Some(r) = rendered.get(&u32::from(*effect)) else { continue };
            if effect == anomaly {
                continue;
            }
            assert_eq!(
                s.rate, r.rate,
                "{}: effect {effect} decodes to {} Hz against the Blorb's {}",
                disk.file_name().unwrap().to_string_lossy(),
                s.rate,
                r.rate,
            );
            agreed += 1;
        }
    }
    assert_eq!(agreed, if cases.len() == 2 { 27 } else { 13 }, "every shared effect but the two anomalies");

    // Non-vacuity: the model must actually BEND things, or it is the old one wearing
    // a new name. Nine of the twenty-nine, and the three that share one recording.
    if let Some((disk, _, _)) = cases.iter().find(|(d, _, _)| d.to_string_lossy().contains("Sherlock")) {
        let s = app::native_sound::from_medium(disk);
        assert_eq!(
            (s[&11].rate, s[&12].rate, s[&13].rate),
            (13032, 18430, 24601),
            "Sherlock's three heartbeats are one recording at three pitches, not one sound thrice",
        );
        assert_eq!(s[&11].name, s[&13].name, "and it really is the one recording");
        bent += 2;
    }
    assert!(bent > 0 || cases.len() == 1);
}

/// The payload under a `/MAC/SOUND` header is **offset binary**, and the Amiga's is
/// signed — which the header does not record either way (SQ-0921).
///
/// Read one as the other and every sample is reflected about silence: a quiet tail
/// becomes full-scale noise, the RMS lands two to five times too high, and the result
/// reaches the player as "very distorted and crunchy", running long because the part
/// that should have faded out never does.
///
/// **The sign is settled by a cross-medium identity, not by inspection**, and the
/// chain is asserted here end to end. *Sherlock*'s effect 8 is the same master on both
/// discs, and once the Macintosh payload is converted it is byte-identical to the
/// Amiga's over all 25,820 frames. Those Amiga bytes are in turn byte-identical to
/// `stories/Sherlock.blb`'s `SSND` payload — and an 8-bit AIFF is signed by
/// definition. So the chain runs Macintosh → Amiga → a third party's rendition, with
/// no link resting on a judgement about how it sounds.
///
/// The other effects cannot be asserted byte for byte and are not: 7, 10, 11–13 and 14
/// are a half-rate decimation on this disc (a different master), and 3, 4, 5, 6, 9,
/// 15, 16 agree only to within 0.6% of their bytes. What holds across all fifteen is
/// the statistic the encoding governs — with the conversion, the mean sample sits
/// within a couple of counts of silence, exactly as the Amiga's does.
#[test]
fn the_macintosh_payload_is_offset_binary() {
    let iso = treasures_dir().join("LostTreasures2.iso");
    let adf = stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf");
    if !iso.is_file() || !adf.is_file() {
        eprintln!("SKIP: gitignored Lost Treasures disc 2 or Sherlock floppy absent");
        return;
    }
    let mac = app::native_sound::from_medium(&iso);
    let amiga = app::native_sound::from_medium(&adf);
    assert_eq!(mac.len(), 15, "Sherlock's fifteen effects, off /MAC/SOUND");

    /// The sample bytes, out of the AIFF the sound is carried as.
    fn pcm(s: &app::native_sound::DiskSound) -> &[u8] {
        let at = s.aiff.len() - s.frames - usize::from(s.frames % 2 == 1);
        &s.aiff[at..at + s.frames]
    }

    // The identity that fixes the sign. Effect 8 is the one master both discs share
    // at full rate; if the conversion were dropped this is 25,820 differing bytes.
    assert_eq!(mac[&8].frames, amiga[&8].frames, "effect 8 is the same master on both discs");
    // Counted rather than compared whole: 25,820 bytes in an assertion message is
    // not a diagnosis, and the count IS the finding — it is zero or it is all of them.
    let differing = pcm(&mac[&8]).iter().zip(pcm(&amiga[&8])).filter(|(m, a)| m != a).count();
    assert_eq!(differing, 0, "effect 8 differs in {differing} of {} frames", mac[&8].frames);

    // The link that makes the Amiga side of that identity the SIGNED one: an 8-bit
    // AIFF is signed by definition, and a third party's AIFF holds the same bytes.
    // Skipped rather than asserted when the Blorb is absent — it is a separate
    // gitignored fixture from the two discs.
    if let Some(blb) = blorb_sounds(&stories_dir().join("Sherlock.blb")) {
        let r = &blb[&8];
        assert_eq!(r.frames as usize, amiga[&8].frames - 1, "the Blorb trims a frame, as it does throughout");
        let differing = r.samples.iter().zip(pcm(&amiga[&8])).filter(|(b, a)| b != a).count();
        assert_eq!(differing, 0, "the Amiga payload is the Blorb's signed AIFF payload");
    } else {
        eprintln!("NOTE: gitignored stories/Sherlock.blb absent, so the third link is unchecked");
    }

    // **No pop.** A Macintosh sample opens and closes at the machine's rest level
    // rather than at its DC, and a flat 0x80 turns both ramps into an excursion to
    // full negative — a click at each end (SQ-0922). Every effect must start and end
    // at silence; with the ramp left in, thirteen of the fifteen read -128 here.
    for e in 3..=17u16 {
        let s = pcm(&mac[&e]);
        // Widened to i32 before abs: the value this is guarding against is exactly
        // -128, and `i8::abs` overflows on it.
        let (first, last) = (i32::from(s[0] as i8), i32::from(s[s.len() - 1] as i8));
        assert!(
            first.abs() < 32 && last.abs() < 32,
            "effect {e} steps to {first}/{last} against silence at its ends",
        );
    }

    // **And nothing but the ramp differs between the masters**, which is what says the
    // trapezoid is the whole story rather than a shape that merely fits. Outside a
    // ramp of `rate / 150` samples, a same-length effect is byte-identical to the
    // Amiga's — so the bytes that disagreed under a flat 0x80 were the ramp and only
    // the ramp: 204 of them for effect 3 against N = 102, 128 for effect 9 against 64.
    // Effect 8 is left out because it is already asserted identical in FULL above,
    // which is stronger; and because it is the one effect here with an `M` file, so
    // its reported rate is the bent one and would not measure the ramp in the file.
    for e in [3u16, 4, 5, 6, 9, 15, 16] {
        let (m, a) = (pcm(&mac[&e]), pcm(&amiga[&e]));
        assert_eq!(m.len(), a.len(), "effect {e} is the same master at the same rate");
        // Rounded up, because the ramp reaches full drive on the sample that first
        // divides out to 128 — 103 at 15360 Hz, not 102.
        let n = (mac[&e].rate as usize).div_ceil(150);
        let body = |i: usize| i >= n && m.len() - 1 - i >= n;
        let differing = (0..m.len()).filter(|&i| body(i) && m[i] != a[i]).count();
        assert_eq!(differing, 0, "effect {e} differs from the Amiga in {differing} frames outside the ramp");
    }

    // And the statistic that holds where the masters differ: silence is at zero.
    for e in 3..=17u16 {
        let mean = |s: &app::native_sound::DiskSound| {
            pcm(s).iter().map(|&b| f64::from(b as i8)).sum::<f64>() / s.frames as f64
        };
        assert!(
            mean(&mac[&e]).abs() < 3.0,
            "effect {e}: /MAC/SOUND mean {:.2} is not centred on silence — read as signed it is {:.2}",
            mean(&mac[&e]),
            pcm(&mac[&e]).iter().map(|&b| f64::from((b ^ 0x80) as i8)).sum::<f64>() / mac[&e].frames as f64,
        );
    }
}

/// **The Macintosh is where the pitch pair comes apart** — and the layout is two
/// files, not three (SQ-0912).
///
/// `/MAC/SOUND` on *Lost Treasures* disc 2 carries bare `S<n>` samples in the same
/// container, and an `M<n>` only for the four effects whose pitch is not the sample's
/// own: 8, 11, 13 and 14 — exactly the four whose Amiga `.mid` does not read 74.
/// `M<n>` is the eleven-byte pitch blob with the index appended, so one file does the
/// work of the Amiga's `.mid` and `.nam` together.
///
/// Effect 12 has no `M12`, so `S12` is effect 12 in its own right; `M11` and `M13`
/// both redirect to that same `S12`. Nothing but the index says so — which is the
/// same lesson `sherlocks_effects_come_from_the_index_not_from_filenames` teaches on
/// the Amiga, in a layout where it is even easier to get wrong.
///
/// The rates are asserted against the disc's own headers bent by the disc's own
/// notes, never against `stories/Sherlock.blb`, which follows a different model — see
/// `the_decoded_samples_are_the_blorbs_samples`.
#[test]
fn the_macintosh_release_bends_the_shared_heartbeat() {
    let iso = treasures_dir().join("LostTreasures2.iso");
    if !iso.is_file() {
        eprintln!("SKIP: gitignored Lost Treasures disc 2 absent");
        return;
    }
    let s = app::native_sound::from_medium(&iso);
    let mut effects: Vec<u16> = s.keys().copied().collect();
    effects.sort_unstable();
    assert_eq!(effects, (3..=17).collect::<Vec<u16>>(), "Sherlock's fifteen effects, off /MAC/SOUND");

    // One sample under three effect numbers, which only the M-files reveal.
    for e in [11u16, 12, 13] {
        assert_eq!(s[&e].name, "S12", "effect {e} plays the one heartbeat sample");
    }
    assert_eq!(s[&11].frames, s[&13].frames);
    assert_eq!(s[&3].name, "S3", "an effect with no M-file is its own sample, name and case intact");

    // S12 states 9215 Hz and base note 50, and its three effects ask for notes 68, 74
    // and 79 — the same three the Amiga floppy asks for, so the same three pitches.
    // The Macintosh master is half-rate throughout, so every figure here is half the
    // floppy's and every PITCH is identical (SQ-0923).
    assert_eq!(s[&11].rate, 6516, "M11 asks for note 68: 68 - 24 - 50 = -6 semitones");
    assert_eq!(s[&12].rate, 9215, "no M12 at all, so the sample plays as itself");
    assert_eq!(s[&13].rate, 12301, "M13 asks for note 79: +5");
    assert_ne!(s[&13].rate, s[&11].rate, "one recording, three pitches");
    // The other two M-files, both note 72 — two semitones down, like the floppy's.
    assert_eq!(s[&8].rate, 9123, "S8 states 10240 Hz, M8 asks for note 72");
    assert_eq!(s[&14].rate, 8211, "S14 states 9216 Hz — half the floppy's, give or take — and M14 the same note");

    // The whole point of the medium comparison: the Macintosh and the floppy agree on
    // PITCH even though they disagree on rate, because the master was decimated.
    let adf = stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf");
    if adf.is_file() {
        let a = app::native_sound::from_medium(&adf);
        // Effect 8's Macintosh master is full-rate, so its figure matches outright;
        // 11-14 are the half-rate decimations, so theirs is half over half the frames,
        // which is the same sound. Within a few Hz rather than exactly, because S14's
        // header states 9216 where the floppy states 18430 — one Hz off a true halving,
        // which the bend then multiplies up.
        assert_eq!(a[&8].rate, s[&8].rate, "effect 8 is not decimated on either disc");
        for e in [11u16, 12, 13, 14] {
            let drift = a[&e].rate.abs_diff(s[&e].rate * 2);
            assert!(
                drift <= 4,
                "effect {e}: the floppy's {} Hz should be twice the Macintosh's {} Hz over twice \
                 the frames — the same sound — but drifts by {drift}",
                a[&e].rate,
                s[&e].rate,
            );
            // Twice the frames give or take a dozen — the decimation drops a few at the
            // tail rather than halving exactly.
            assert!(
                a[&e].frames.abs_diff(s[&e].frames * 2) <= 16,
                "effect {e}: {} floppy frames against {} Macintosh ones",
                a[&e].frames,
                s[&e].frames,
            );
        }
    }
}

/// **The medium outranks a Blorb, on the shipped configuration** (SQ-0914).
///
/// This is not hypothetical: `stories/Sherlock.blb` sits beside the Sherlock floppy
/// in the same directory, `blorb::resolve_resource_blorb` finds it, and until this
/// quest it won — so playing that ADF played the Blorb and confirmed nothing about
/// the disk. The two sources are distinguishable by rate, which is what makes the
/// pick observable here rather than merely asserted: the floppy states 18430 Hz for
/// effect 11 and the Blorb states 13032, because the Blorb's author baked in a pitch
/// model that is not the interpreter's (see `Pitch` and the case above).
///
/// Falsified by swapping the arms of `app::state::resolve_sound`, which returns the
/// Blorb's bytes and fails the byte-equality assertion.
#[test]
fn the_medium_outranks_a_blorb_filed_beside_the_story() {
    let adf = stories_dir().join("Sherlock - The Riddle of the Crown Jewels.adf");
    if !adf.is_file() {
        eprintln!("SKIP: gitignored Sherlock floppy absent");
        return;
    }
    let Some((blb, blb_path)) = blorb::resolve_resource_blorb(&adf) else {
        eprintln!("SKIP: gitignored stories/Sherlock.blb absent, so there is no contest to settle");
        return;
    };
    // The premise: both sources really do offer this effect, or the case is vacuous.
    assert!(blb.sound(11).is_some(), "{} should carry effect 11", blb_path.display());
    let disk = app::native_sound::from_medium(&adf);
    assert_eq!(disk[&11].rate, 13032, "the floppy's 18430 Hz header bent six semitones down");

    let (bytes, kind, from_medium) =
        app::state::resolve_sound(&disk, Some(&blb), 11).expect("effect 11 resolves");
    assert_eq!(from_medium, Some("heart"), "the DISK answered, and says which sample");
    assert_eq!(kind, blorb::SoundKind::Aiff);
    assert_eq!(bytes, &disk[&11].aiff[..], "byte for byte the disk's, not the Blorb's");
    assert_ne!(
        bytes,
        blb.sound(11).expect("asserted above").0,
        "the two renditions differ, which is why the precedence is observable at all",
    );
}

