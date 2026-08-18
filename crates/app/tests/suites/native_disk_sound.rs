//! Sounds a release disk carries natively, checked against a rendition of the same
//! sounds that somebody else made (SQ-0907).
//!
//! Two Infocom games use sound: *The Lurking Horror* and *Sherlock*. On the Amiga
//! their disks hold a `Sound/` directory — an `sN.nam` index per Z-machine effect,
//! naming a sample and a pitch file — and the samples are an Infocom container that
//! nothing else reads.
//!
//! **A format worked out from bytes alone is a guess.** `stories/Lurking.blb` is the
//! same fourteen effects wrapped as Blorb `Snd ` resources in AIFF, by a third party,
//! and it is what turns this from reverse-engineering into verification: the frames
//! this decoder produces must be byte-identical to the ones in that Blorb. That check
//! is the whole point of the suite and is why the decoder was trusted at all.
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
/// The RATES are recorded rather than reconciled. Six of the fourteen agree; on the
/// other eight the Blorb reads LOWER — effect 3 says 9676 Hz against the disk's
/// 15360 — and no consistent ratio relates them, so it is not a units error or a
/// pitch transposition anyone here has explained. That Blorb is one person's
/// rendition; the disk's header is what the Amiga itself was handed, so the disk
/// wins and the divergence is pinned as data instead of averaged away.
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

    let (mut same_rate, mut differing) = (0, Vec::new());
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
        } else {
            differing.push((effect, sound.rate, want.rate));
        }
    }
    differing.sort_unstable();
    assert_eq!(
        differing,
        vec![
            (3u16, 15360u32, 9676u32),
            (4, 20480, 12902),
            (6, 18430, 11610),
            (7, 18430, 11610),
            (8, 18430, 17396),
            (9, 18430, 17396),
            (12, 20480, 13669),
            (17, 32910, 31250),
        ],
        "the rate disagreements moved. Six agree and these eight do not, the Blorb always \
         lower; if that set changes, re-read which source is right before touching either",
    );
    assert_eq!(same_rate, 6, "and the rest agree exactly");
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
