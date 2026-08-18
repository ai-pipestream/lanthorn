//! Infocom's own sampled-sound files, as they sit on a release disk (SQ-0907).
//!
//! Two Infocom games use sound: *The Lurking Horror* and *Sherlock*. On the Amiga
//! their release disks carry a `Sound/` directory holding one THREE-file set per
//! Z-machine effect number — `sN.dat` (the sample), `sN.mid` (eleven bytes of
//! MIDI-shaped Note-On/Note-Off giving the pitch it is played at) and `sN.nam` (a
//! manifest naming the other two). The numbering is the standard's own: ZMSD §9
//! reserves effects 1 and 2 for the interpreter's bleeps, so a game's sounds start
//! at 3.
//!
//! # The `.dat` container
//!
//! A ten-byte big-endian header, then signed 8-bit mono PCM:
//!
//! | offset | field |
//! |---|---|
//! | 0 | byte count of everything after this field — the file's length less 2 |
//! | 2 | flags; observed `0x003C`, `0x013C`, `0x0032`, `0x0132` |
//! | 4 | sample rate in Hz |
//! | 6 | zero on every file measured |
//! | 8 | frame count |
//! | 10 | `frames` bytes of samples |
//!
//! **Verified against a reference rendition rather than reverse-engineered alone.**
//! `stories/Lurking.blb` is the same fourteen effects wrapped as Blorb `Snd `
//! resources in AIFF, and on all fourteen — 3, 4, 6–13 and 15–18 — the frame count
//! here equals the AIFF's `COMM` frame count and the sample bytes are
//! **byte-identical** to its `SSND` payload. The length field equals the file size
//! less two on every one, which is what makes it a usable signature.
//!
//! The rates agree on seven of the fourteen and differ on the rest, always with the
//! Blorb reading LOWER. That Blorb is a third-party rendition; this header is what
//! the Amiga itself was handed, so [`InfocomSound::rate`] reports the disk's own
//! figure and the divergence is recorded here rather than reconciled away.
//!
//! # Why AIFF comes back out
//!
//! [`InfocomSound::to_aiff`] wraps the samples in the container the host already
//! decodes, so a disk-native sound reaches the mixer through exactly the path a
//! Blorb one does — same volume scaling, same repeats, same finish routine. Nothing
//! downstream learns that this format exists.

/// What a `sN.nam` says: which sample file and which pitch file are effect N.
///
/// **This index is not optional, and a naming convention is not a substitute.** On
/// *The Lurking Horror* every entry happens to name `sN.dat` and `sN.mid`, so the
/// numbering looks like it lives in the filenames. On *Sherlock* it does not: its
/// samples are called `armor`, `growl`, `splash`, `violin.bin`, and three separate
/// effects — 11, 12 and 13 — all name the SAME sample, `heart`, against three
/// different pitch files. A reader that matched `sN.dat` would find nothing on that
/// disk at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundIndex {
    /// The sample file, relative to the `Sound` directory.
    pub sample: String,
    /// The pitch file, relative to the same directory. See [`InfocomSound`] on why
    /// nothing is currently done with it.
    pub midi: String,
}

impl SoundIndex {
    /// Parse a `sN.nam`: a two-byte prefix, then the sample's NUL-terminated name,
    /// then the pitch file's, then zero padding to the block.
    pub fn parse(raw: &[u8]) -> Option<SoundIndex> {
        let body = raw.get(2..)?;
        let mut names = body.split(|&b| b == 0).filter(|s| !s.is_empty());
        let sample = std::str::from_utf8(names.next()?).ok()?.to_string();
        let midi = names.next().and_then(|m| std::str::from_utf8(m).ok()).unwrap_or("").to_string();
        (!sample.is_empty()).then_some(SoundIndex { sample, midi })
    }
}

/// One decoded Infocom sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfocomSound {
    /// Playback rate in Hz, as the disk states it.
    pub rate: u32,
    /// The header's flags word, kept because it is not yet understood and a caller
    /// measuring these disks should be able to see it.
    pub flags: u16,
    /// Signed 8-bit mono PCM, `frames` bytes.
    pub samples: Vec<u8>,
}

/// The fixed header, before the samples.
const HEADER: usize = 10;

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

impl InfocomSound {
    /// Parse a `Sound/sN.dat`, or `None` when the bytes are not one.
    ///
    /// The signature is the length field agreeing with the file's own size, which
    /// no other file on these disks satisfies by accident, plus a frame count that
    /// fits in what is left.
    pub fn parse(raw: &[u8]) -> Option<InfocomSound> {
        if raw.len() < HEADER || raw.len() > u16::MAX as usize + 2 {
            return None;
        }
        if be16(raw, 0) as usize != raw.len() - 2 {
            return None;
        }
        let frames = be16(raw, 8) as usize;
        if frames == 0 || HEADER + frames > raw.len() {
            return None;
        }
        Some(InfocomSound {
            rate: u32::from(be16(raw, 4)),
            flags: be16(raw, 2),
            samples: raw[HEADER..HEADER + frames].to_vec(),
        })
    }

    /// The same sound as an AIFF `FORM`, for the host's existing decoder.
    ///
    /// Mono, 8-bit, at [`InfocomSound::rate`] — which is what the samples are, so
    /// nothing is resampled or requantised here and the payload survives unchanged.
    pub fn to_aiff(&self) -> Vec<u8> {
        let n = self.samples.len() as u32;
        let mut out = Vec::with_capacity(self.samples.len() + 64);
        // COMM (18) and SSND (8 + samples), each with an 8-byte chunk header, after
        // the four bytes of form type.
        let form_len = 4 + (8 + 18) + (8 + 8 + n);
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&form_len.to_be_bytes());
        out.extend_from_slice(b"AIFF");

        out.extend_from_slice(b"COMM");
        out.extend_from_slice(&18u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes()); // channels
        out.extend_from_slice(&n.to_be_bytes()); // frames
        out.extend_from_slice(&8u16.to_be_bytes()); // bits per sample
        out.extend_from_slice(&extended80(self.rate));

        out.extend_from_slice(b"SSND");
        out.extend_from_slice(&(8 + n).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // offset
        out.extend_from_slice(&0u32.to_be_bytes()); // block size
        out.extend_from_slice(&self.samples);
        // A FORM's chunks are word-aligned; SSND's payload may be odd.
        if n % 2 == 1 {
            out.push(0);
        }
        out
    }
}

/// An integer sample rate as AIFF's 80-bit IEEE 754 extended float.
///
/// The format is a 15-bit biased exponent, a sign bit, and an **explicit** leading
/// mantissa bit — unlike IEEE single/double, where it is implied. So a normalised
/// value has the mantissa's top bit set, and the bias is 16383.
fn extended80(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let top = 31 - rate.leading_zeros(); // floor(log2(rate))
    let exp = 16383 + top;
    let mantissa = u64::from(rate) << (63 - top);
    out[0..2].copy_from_slice(&(exp as u16).to_be_bytes());
    out[2..10].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Every sound a mounted volume's `Sound/` directory offers, by Z-machine effect
/// number, as `(sample name, decoded sample)`.
///
/// `files` is whatever the medium reported — [`crate::medium::MountedDisk::contents`]
/// or any equivalent — as `(path, bytes)`. Lives here rather than in a host so the
/// TUI and `zvm-cli` cannot drift: both mount their own medium and hand it over.
///
/// **The effect number comes from the `sN.nam` INDEX, never from a filename.** On
/// *The Lurking Horror* every index happens to name `sN.dat`, so the numbering looks
/// like a convention; on *Sherlock* the samples are `armor`, `growl`, `splash`,
/// `violin.bin`, and effects 11, 12 and 13 all name the same `heart` sample at three
/// different pitches. Keying on filenames finds nothing on that disk.
///
/// Effects below 3 are dropped: ZMSD §9 reserves 1 and 2 for the interpreter's own
/// bleeps, which are synthesised and never sampled.
pub fn from_volume<'a, I>(files: I) -> std::collections::BTreeMap<u16, (String, InfocomSound)>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    let by_path: std::collections::BTreeMap<String, &[u8]> =
        files.into_iter().map(|(p, b)| (p.to_ascii_lowercase(), b)).collect();
    let mut out = std::collections::BTreeMap::new();
    for (path, raw) in &by_path {
        let Some(effect) = index_effect(path) else { continue };
        let Some(idx) = SoundIndex::parse(raw) else { continue };
        // The sample sits beside its index, so resolve against that directory.
        let dir = path.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
        let Some(sample) = by_path.get(&format!("{dir}{}", idx.sample.to_ascii_lowercase())) else {
            continue;
        };
        let Some(snd) = InfocomSound::parse(sample) else { continue };
        out.insert(effect, (idx.sample.clone(), snd));
    }
    out
}

/// The effect number in a `Sound/sN.nam` path, or `None`.
///
/// Matched on the PATH, because the directory is what makes it a sound index rather
/// than any other `s3.nam` on the volume — and because until SQ-0908 an AmigaDOS
/// mount reported no directory at all, so this could not have been written.
fn index_effect(lower_path: &str) -> Option<u16> {
    let stem = lower_path.strip_suffix(".nam")?;
    if !lower_path.starts_with("sound/") && !lower_path.contains("/sound/") {
        return None;
    }
    let n: u16 = stem.rsplit('/').next()?.strip_prefix('s')?.parse().ok()?;
    (n >= 3).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.dat` of `frames` samples, built the way the disk builds one.
    fn dat(rate: u16, flags: u16, samples: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((HEADER + samples.len() - 2) as u16).to_be_bytes());
        v.extend_from_slice(&flags.to_be_bytes());
        v.extend_from_slice(&rate.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&(samples.len() as u16).to_be_bytes());
        v.extend_from_slice(samples);
        v
    }

    /// The index, in both the shapes the two sound games use.
    #[test]
    fn an_index_path_yields_its_effect_number() {
        assert_eq!(index_effect("sound/s3.nam"), Some(3));
        assert_eq!(index_effect("sound/s18.nam"), Some(18));
        assert_eq!(index_effect("lurking/sound/s7.nam"), Some(7), "nested under the game");
    }

    /// Everything that is NOT one, which is the half that keeps this off the rest of
    /// a disk.
    #[test]
    fn anything_else_is_not_an_index() {
        assert_eq!(index_effect("sound/s10.dat"), None, "the sample itself");
        assert_eq!(index_effect("sound/s10.mid"), None, "its pitch file");
        assert_eq!(index_effect("s3.nam"), None, "outside the Sound directory");
        assert_eq!(index_effect("sound/story.nam"), None, "not numbered");
        assert_eq!(index_effect("sound/s1.nam"), None, "1 and 2 are the interpreter's bleeps");
        assert_eq!(index_effect("sound/s2.nam"), None);
    }

    /// A whole volume, in the shape Sherlock's disk has: descriptive sample names,
    /// and one sample shared by more than one effect.
    #[test]
    fn a_volume_is_indexed_by_effect_number() {
        let heart = dat(18430, 0, &[1, 2, 3, 4]);
        let armor = dat(15360, 0, &[9, 9]);
        let files: Vec<(&str, &[u8])> = vec![
            ("Sound/s3.nam", b"\x01\x00armor\x00\x00fan.mid\x00"),
            ("Sound/armor", &armor),
            ("Sound/s11.nam", b"\x01\x00heart\x00\x00heart1.mid\x00"),
            ("Sound/s13.nam", b"\x01\x00heart\x00\x00heart3.mid\x00"),
            ("Sound/heart", &heart),
            ("Sound/s2.nam", b"\x01\x00armor\x00\x00fan.mid\x00"),
            ("Story.Data", b"not a sound"),
        ];
        let got = from_volume(files);
        assert_eq!(got.keys().copied().collect::<Vec<_>>(), vec![3, 11, 13], "2 is a bleep, not a sample");
        assert_eq!(got[&3].0, "armor");
        assert_eq!(got[&11].0, "heart");
        assert_eq!(got[&13].0, "heart", "two effects, one sample, different pitches");
        assert_eq!(got[&11].1.samples, got[&13].1.samples);
        assert_eq!(got[&3].1.rate, 15360);
    }

    #[test]
    fn a_nam_names_its_sample_and_its_pitch_file() {
        // The Lurking Horror, where the names echo the effect number.
        let lh = SoundIndex::parse(b"\x01\x00s10.dat\x00\x00s10.mid\x00\x00\x00\x00\x00\x00").expect("parses");
        assert_eq!(lh, SoundIndex { sample: "s10.dat".into(), midi: "s10.mid".into() });
        // Sherlock, where they do not — and where three effects share one sample.
        let s11 = SoundIndex::parse(b"\x01\x00heart\x00\x00heart1.mid\x00\x00").expect("parses");
        let s13 = SoundIndex::parse(b"\x01\x00heart\x00\x00heart3.mid\x00\x00").expect("parses");
        assert_eq!(s11.sample, "heart");
        assert_eq!(s13.sample, "heart", "the same sample, at a different pitch");
        assert_ne!(s11.midi, s13.midi);
        assert_eq!(
            SoundIndex::parse(b"\x01\x00violin.bin\x00\x00clk.mid\x00\x00\x00").map(|i| i.sample),
            Some("violin.bin".to_string()),
            "a sample name may carry its own extension",
        );
        assert_eq!(SoundIndex::parse(b""), None);
        assert_eq!(SoundIndex::parse(b"\x01\x00"), None, "no name at all");
    }

    #[test]
    fn the_header_is_read_and_the_samples_come_out_whole() {
        let s = InfocomSound::parse(&dat(15360, 0x003C, &[0x0F, 0x06, 0xFA, 0x0D])).expect("parses");
        assert_eq!(s.rate, 15360);
        assert_eq!(s.flags, 0x003C);
        assert_eq!(s.samples, vec![0x0F, 0x06, 0xFA, 0x0D], "the payload is not touched");
    }

    /// The length field is the signature, so bytes that do not satisfy it are not
    /// one of these — which is what keeps the sniff off every other file on a disk.
    #[test]
    fn bytes_that_are_not_one_are_refused() {
        assert_eq!(InfocomSound::parse(b""), None, "empty");
        assert_eq!(InfocomSound::parse(b"FORM\0\0\0\x08AIFF"), None, "an AIFF is not one");
        let mut wrong = dat(15360, 0, &[1, 2, 3, 4]);
        wrong[1] ^= 0xFF;
        assert_eq!(InfocomSound::parse(&wrong), None, "a length that disagrees with the file");
        let mut over = dat(15360, 0, &[1, 2, 3, 4]);
        over[8..10].copy_from_slice(&9999u16.to_be_bytes());
        assert_eq!(InfocomSound::parse(&over), None, "a frame count past the end");
    }

    /// AIFF's 80-bit extended float, against the value a real Blorb carries.
    ///
    /// Every expectation here is COPIED OUT of `stories/Lurking.blb`, not computed
    /// by hand — the first draft of this case invented the 18430 bytes and was wrong,
    /// which is the whole reason a constant gets read from a source (CLAUDE.md).
    ///
    /// | effect | rate | the Blorb's ten bytes |
    /// |---|---|---|
    /// | 10 | 15360 | `40 0C F0 00 …` |
    /// | 11 | 18430 | `40 0D 8F FC …` |
    /// | 15 | 11100 | `40 0C AD 70 …` |
    /// | 17 | 31250 | `40 0D F4 24 …` |
    ///
    /// The leading mantissa bit is EXPLICIT in this format, unlike IEEE single and
    /// double, which is the part that is easy to get wrong.
    #[test]
    fn a_rate_encodes_as_the_blorbs_own_bytes() {
        assert_eq!(extended80(15360), [0x40, 0x0C, 0xF0, 0x00, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(18430), [0x40, 0x0D, 0x8F, 0xFC, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(11100), [0x40, 0x0C, 0xAD, 0x70, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(31250), [0x40, 0x0D, 0xF4, 0x24, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(0), [0; 10], "no rate at all encodes as zero, not as a panic");
    }

    /// The wrapper is an AIFF the host's decoder reads, and the samples inside it
    /// are the ones that went in.
    #[test]
    fn the_aiff_wrapper_declares_mono_eight_bit_at_the_disks_rate() {
        let s = InfocomSound::parse(&dat(15360, 0, &[0x0F, 0x06, 0xFA, 0x0D])).expect("parses");
        let a = s.to_aiff();
        assert_eq!(&a[0..4], b"FORM");
        assert_eq!(&a[8..12], b"AIFF");
        assert_eq!(u32::from_be_bytes([a[4], a[5], a[6], a[7]]) as usize, a.len() - 8, "FORM length");
        assert_eq!(&a[12..16], b"COMM");
        assert_eq!(be16(&a, 20), 1, "channels");
        assert_eq!(u32::from_be_bytes([a[22], a[23], a[24], a[25]]), 4, "frames");
        assert_eq!(be16(&a, 26), 8, "bits per sample");
        assert_eq!(&a[38..42], b"SSND");
        // id(4) + length(4) + offset(4) + blockSize(4) precede the frames.
        assert_eq!(&a[54..58], &[0x0F, 0x06, 0xFA, 0x0D], "the samples, unchanged");
    }

    /// An odd frame count still leaves a well-formed FORM: AIFF chunks are
    /// word-aligned, so the pad byte is outside `SSND`'s declared length.
    #[test]
    fn an_odd_sample_count_is_padded_without_being_counted() {
        let s = InfocomSound::parse(&dat(11100, 0, &[1, 2, 3])).expect("parses");
        let a = s.to_aiff();
        assert_eq!(a.len() % 2, 0, "the form is word-aligned");
        let ssnd_len = u32::from_be_bytes([a[42], a[43], a[44], a[45]]);
        assert_eq!(ssnd_len, 8 + 3, "SSND counts its samples, not the pad");
    }
}
