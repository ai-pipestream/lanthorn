//! Cross-platform host-side audio backend for babelmap. Plays synthesised tones
//! (Z-machine bleeps) and decoded samples (Blorb `Snd ` resources) via `rodio`.
//! With the `playback` feature off, the backend is a compile-time no-op.

/// Identifies a playing sampled sound so the host can stop it or detect its end.
pub type SoundId = u32;

/// Sampled-sound container format, chosen by the host from the Blorb chunk type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundFormat {
    Aiff,
    Ogg,
    Mod,
}

const SAMPLE_RATE: u32 = 44100;

/// Master+Z-scale gain in 0.0..=1.0. Master is 0..=100; z_volume is the Z-machine
/// 1..=8 scale, with 0/255 meaning "loudest" (full).
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn gain(master: u8, z_volume: u8) -> f32 {
    (master.min(100) as f32 / 100.0)
        * match z_volume {
            0 | 255 => 1.0,
            v => (v.min(8) as f32) / 8.0,
        }
}

/// A short decaying sine at `freq_hz` for `ms` at 44100 Hz (unit amplitude,
/// linear decay envelope). Volume is applied by the caller via the sink.
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn synth_tone(freq_hz: f32, ms: u32) -> Vec<f32> {
    let n = ((ms as f32 / 1000.0) * SAMPLE_RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (1.0 - (i as f32 / n.max(1) as f32)).max(0.0);
        let s = (2.0 * std::f32::consts::PI * freq_hz * t).sin();
        out.push(s * env);
    }
    out
}

/// Decode a 10-byte 80-bit IEEE-754 extended float (AIFF sample rate) to u32 Hz.
/// Layout: 1 sign bit, 15 exponent bits (bias 16383), 64 mantissa bits with an
/// explicit integer bit. value = mantissa * 2^(exponent - 16383 - 63).
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn extended80_to_u32(b: &[u8; 10]) -> u32 {
    let exponent = (((b[0] & 0x7F) as u32) << 8) | b[1] as u32;
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return 0;
    }
    let e = exponent as i32 - 16383 - 63;
    let mut val = mantissa as f64;
    if e >= 0 {
        val *= 2f64.powi(e);
    } else {
        val /= 2f64.powi(-e);
    }
    val as u32
}

/// Parse an IFF `FORM`/`AIFF` container into (channels, sample_rate, interleaved
/// big-endian 16-bit PCM). Returns None on a malformed or non-16-bit AIFF.
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn decode_aiff(bytes: &[u8]) -> Option<(u16, u32, Vec<i16>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" || &bytes[8..12] != b"AIFF" {
        return None;
    }
    let mut pos = 12;
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut sample_size: u16 = 0;
    let mut pcm: Vec<i16> = Vec::new();
    while pos + 8 <= bytes.len() {
        let id = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let len = u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let data_start = pos + 8;
        if data_start + len > bytes.len() {
            break;
        }
        match &id {
            b"COMM" if len >= 18 => {
                channels = u16::from_be_bytes([bytes[data_start], bytes[data_start + 1]]);
                sample_size = u16::from_be_bytes([bytes[data_start + 6], bytes[data_start + 7]]);
                let mut ext = [0u8; 10];
                ext.copy_from_slice(&bytes[data_start + 8..data_start + 18]);
                sample_rate = extended80_to_u32(&ext);
            }
            b"SSND" if len >= 8 => {
                // Skip offset (u32) + blockSize (u32); the rest is big-endian PCM.
                let pcm_start = data_start + 8;
                let pcm_end = data_start + len;
                if sample_size <= 16 {
                    let mut i = pcm_start;
                    while i + 1 < pcm_end {
                        pcm.push(i16::from_be_bytes([bytes[i], bytes[i + 1]]));
                        i += 2;
                    }
                }
            }
            _ => {}
        }
        pos = data_start + len + (len & 1);
    }
    if channels == 0 || sample_rate == 0 || pcm.is_empty() {
        return None;
    }
    Some((channels, sample_rate, pcm))
}

/// RAII guard that removes its temp file on drop — covers every exit path
/// (normal return, early `?`/`None`, and panic-unwind).
#[cfg(feature = "mod-music")]
struct TempFileGuard(std::path::PathBuf);

#[cfg(feature = "mod-music")]
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `mod_player` 0.1's only public loader is `read_mod_file(path: &str) -> Song`,
/// which reads from disk (there is no in-memory/slice loader). We stage `bytes`
/// to a uniquely-named temp file, parse it, then remove the temp file (via
/// `TempFileGuard`) — the returned `Song` owns all sample/pattern data, so
/// nothing further touches it.
///
/// Known accepted issue: `mod_player` 0.1.4's `get_format` emits a stray
/// `println!` on every parse (an upstream debug leftover) which can briefly
/// garble the TUI until the next redraw. Not removable without vendoring the
/// crate; tracked as a follow-up — do not attempt to fix/suppress it here.
#[cfg(feature = "mod-music")]
fn load_mod_song(bytes: &[u8]) -> Option<mod_player::Song> {
    // ProTracker header (title + 31 sample records + songlen/restart + order
    // table + 4-byte tag) is 1084 bytes minimum; `mod_player::get_format`
    // slices `file_data[1080..1084]` unchecked, so shorter input panics.
    if bytes.len() < 1084 {
        return None;
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("babelmap-mod-{}-{n}.mod", std::process::id()));
    std::fs::write(&path, bytes).ok()?;
    let _guard = TempFileGuard(path.clone());
    let path_str = path.to_str()?;

    // `mod_player` panics (unwrap/panic!) on various malformed inputs beyond
    // the length check above. Catch that here so a corrupt MOD from an
    // untrusted Blorb resource can't crash the interpreter. Silencing the
    // panic hook for the duration is safe because this sound-loading path
    // runs synchronously on a single thread — no concurrent panics can race
    // on the global hook.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mod_player::read_mod_file(path_str)));
    std::panic::set_hook(prev_hook);

    result.ok()
}

/// A `rodio::Source` that streams a ProTracker module via `mod_player`, yielding
/// interleaved stereo f32 (left then right of each frame on alternate `next`).
///
/// `next` always returns `Some`, and `total_duration()`/`current_frame_len()`
/// are both `None`, so the owning `Sink` never becomes empty. A MOD therefore
/// never reports via `finished()` and never fires a finish-routine — intentional
/// for looping tracker music; it stays tracked until an explicit `stop`/`stop_all`.
#[cfg(feature = "mod-music")]
struct ModSource {
    song: mod_player::Song,
    state: mod_player::PlayerState,
    rate: u32,
    pending_right: Option<f32>,
}

#[cfg(feature = "mod-music")]
impl Iterator for ModSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.pending_right.take() {
            return Some(r);
        }
        let (l, r) = mod_player::next_sample(&self.song, &mut self.state);
        self.pending_right = Some(r);
        Some(l)
    }
}

#[cfg(feature = "mod-music")]
impl rodio::Source for ModSource {
    fn current_frame_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { 2 }
    fn sample_rate(&self) -> u32 { self.rate }
    fn total_duration(&self) -> Option<std::time::Duration> { None }
}

/// Map the Z-machine repeat count to (loop_forever, finite_play_count).
/// 255 = forever; 0 (or omitted, which the engine records as 0) = play once;
/// 1..=254 = that many plays. (Matches de-facto interpreter behavior.)
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn repeat_plan(repeats: u8) -> (bool, u8) {
    match repeats {
        255 => (true, 1),
        0 => (false, 1),
        n => (false, n),
    }
}

// ── Real backend (playback feature on) ────────────────────────────────────────

#[cfg(feature = "playback")]
pub struct AudioBackend {
    stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    samples: std::collections::HashMap<SoundId, rodio::Sink>,
    tones: Vec<rodio::Sink>,
    next_id: SoundId,
    master: u8,
}

#[cfg(feature = "playback")]
impl std::fmt::Debug for AudioBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioBackend").finish_non_exhaustive()
    }
}

#[cfg(feature = "playback")]
impl AudioBackend {
    pub fn new(volume: u8) -> AudioBackend {
        let stream = match rodio::OutputStream::try_default() {
            Ok((s, h)) => Some((s, h)),
            Err(e) => {
                eprintln!("audio: no output device ({e}); sound disabled");
                None
            }
        };
        AudioBackend {
            stream,
            samples: std::collections::HashMap::new(),
            tones: Vec::new(),
            next_id: 1,
            master: volume.min(100),
        }
    }

    pub fn play_tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8) {
        let Some((_, handle)) = &self.stream else { return };
        let Ok(sink) = rodio::Sink::try_new(handle) else { return };
        sink.set_volume(gain(self.master, z_volume));
        sink.append(rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, synth_tone(freq_hz, ms)));
        self.tones.push(sink);
    }

    /// Decode `bytes` per `format`, play on a fresh sink at gain(master, z_volume),
    /// looping per `repeats` (see `repeat_plan`: 255 = forever, 0/omitted = once).
    /// Returns a SoundId to `stop`/track.
    /// Returns None if there is no device, the format is unsupported, or decode fails.
    pub fn play_sample(&mut self, bytes: &[u8], format: SoundFormat, z_volume: u8, repeats: u8) -> Option<SoundId> {
        use rodio::Source;
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        sink.set_volume(gain(self.master, z_volume));
        let (forever, count) = repeat_plan(repeats);
        match format {
            SoundFormat::Aiff => {
                let (channels, rate, pcm) = decode_aiff(bytes)?;
                if forever {
                    sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()).repeat_infinite());
                } else {
                    for _ in 0..count {
                        sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()));
                    }
                }
            }
            SoundFormat::Ogg => {
                if forever {
                    let dec = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())).ok()?;
                    sink.append(dec.repeat_infinite());
                } else {
                    for _ in 0..count {
                        if let Ok(dec) = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())) {
                            sink.append(dec);
                        }
                    }
                }
            }
            SoundFormat::Mod => {
                #[cfg(feature = "mod-music")]
                { return self.play_mod(bytes, z_volume); }
                #[cfg(not(feature = "mod-music"))]
                {
                    eprintln!("audio: unsupported sound format (MOD; mod-music feature off)");
                    return None;
                }
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, sink);
        Some(id)
    }

    /// Decode and play a ProTracker MOD via `mod_player`, streaming through
    /// `ModSource`. `repeats`/looping are not modelled for MOD (tracker songs
    /// loop internally via pattern jumps); this plays the song once.
    #[cfg(feature = "mod-music")]
    fn play_mod(&mut self, bytes: &[u8], z_volume: u8) -> Option<SoundId> {
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        sink.set_volume(gain(self.master, z_volume));
        let song = load_mod_song(bytes)?;
        let state = mod_player::PlayerState::new(song.format.num_channels, SAMPLE_RATE);
        let source = ModSource { song, state, rate: SAMPLE_RATE, pending_right: None };
        sink.append(source);
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, sink);
        Some(id)
    }

    pub fn stop(&mut self, id: SoundId) {
        if let Some(sink) = self.samples.remove(&id) {
            sink.stop();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, s) in self.samples.drain() {
            s.stop();
        }
        for s in self.tones.drain(..) {
            s.stop();
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.master = volume.min(100);
        let v = self.master as f32 / 100.0;
        for s in self.samples.values() {
            s.set_volume(v);
        }
        for s in &self.tones {
            s.set_volume(v);
        }
    }

    /// Drain completed sample ids (whose sink is empty) and prune finished tones.
    pub fn finished(&mut self) -> Vec<SoundId> {
        self.tones.retain(|s| !s.empty());
        let done: Vec<SoundId> = self
            .samples
            .iter()
            .filter(|(_, s)| s.empty())
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            self.samples.remove(id);
        }
        done
    }
}

// ── No-op backend (playback feature off) ──────────────────────────────────────

#[cfg(not(feature = "playback"))]
#[derive(Debug)]
pub struct AudioBackend;

#[cfg(not(feature = "playback"))]
impl AudioBackend {
    pub fn new(_volume: u8) -> AudioBackend { AudioBackend }
    pub fn play_tone(&mut self, _freq_hz: f32, _ms: u32, _z_volume: u8) {}
    pub fn play_sample(&mut self, _bytes: &[u8], _format: SoundFormat, _z_volume: u8, _repeats: u8) -> Option<SoundId> { None }
    pub fn stop(&mut self, _id: SoundId) {}
    pub fn stop_all(&mut self) {}
    pub fn set_volume(&mut self, _volume: u8) {}
    pub fn finished(&mut self) -> Vec<SoundId> { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_combines_master_and_z_volume() {
        assert_eq!(gain(100, 8), 1.0);        // full master, loudest z-scale
        assert_eq!(gain(50, 8), 0.5);         // half master
        assert_eq!(gain(100, 4), 0.5);        // z 4/8
        assert_eq!(gain(100, 0), 1.0);        // 0 -> treated as full
        assert_eq!(gain(100, 255), 1.0);      // 255 -> loudest
        assert_eq!(gain(0, 8), 0.0);          // muted master
    }

    #[test]
    fn repeat_plan_maps_counts() {
        assert_eq!(repeat_plan(0), (false, 1));
        assert_eq!(repeat_plan(255), (true, 1));
        assert_eq!(repeat_plan(5), (false, 5));
        assert_eq!(repeat_plan(1), (false, 1));
    }

    #[test]
    fn synth_tone_has_expected_length_and_energy() {
        let s = synth_tone(440.0, 100); // 100ms @ 44100 = 4410 samples
        assert_eq!(s.len(), 4410, "length = ms/1000 * 44100");
        assert!(s.iter().any(|v| v.abs() > 0.1), "tone must not be silent");
        // Decaying envelope: the first quarter is louder than the last quarter.
        let peak_early = s[..1000].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        let peak_late = s[3410..].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        assert!(peak_early > peak_late, "amplitude decays over time");
    }

    #[test]
    fn backend_no_device_paths_never_panic() {
        // Constructing a backend must succeed even with no output device (CI).
        let mut b = AudioBackend::new(100);
        b.play_tone(800.0, 50, 8);
        b.set_volume(50);
        b.stop(1);
        b.stop_all();
        let _ = b.finished();
    }

    /// Build a tiny 1-channel, 16-bit, 44100 Hz AIFF with two PCM frames.
    fn tiny_aiff() -> Vec<u8> {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        // COMM: channels=1, numFrames=2, sampleSize=16, rate = 44100 as 80-bit ext.
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());       // channels
        comm.extend_from_slice(&2u32.to_be_bytes());       // numSampleFrames
        comm.extend_from_slice(&16u16.to_be_bytes());      // sampleSize
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=0, blockSize=0, then two BE i16 samples: 256, -256.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&256i16.to_be_bytes());
        ssnd.extend_from_slice(&(-256i16).to_be_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"AIFF");
        body.extend_from_slice(&be_chunk(b"COMM", &comm));
        body.extend_from_slice(&be_chunk(b"SSND", &ssnd));

        let mut form = Vec::new();
        form.extend_from_slice(b"FORM");
        form.extend_from_slice(&(body.len() as u32).to_be_bytes());
        form.extend_from_slice(&body);
        form
    }

    #[test]
    fn extended80_decodes_44100() {
        assert_eq!(extended80_to_u32(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]), 44100);
        assert_eq!(extended80_to_u32(&[0; 10]), 0);
    }

    #[test]
    fn decode_aiff_parses_comm_and_ssnd() {
        let (channels, rate, pcm) = decode_aiff(&tiny_aiff()).expect("valid AIFF");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![256i16, -256i16]);
    }

    #[cfg(not(feature = "playback"))]
    #[test]
    fn play_sample_returns_none_without_playback() {
        let mut b = AudioBackend::new(100);
        assert!(b.play_sample(&tiny_aiff(), SoundFormat::Aiff, 8, 1).is_none());
    }

    /// Smallest structurally-valid 4-channel ("M.K.") ProTracker MOD: 1084-byte
    /// header (20-byte title, 31 * 30-byte sample records, songlen, restart,
    /// 128-byte order table, "M.K."), then one 1024-byte (64-row * 4-ch * 4-byte)
    /// zero pattern, and no sample data (all sample lengths are 0). It is silent,
    /// so the test asserts structure (channels == 2) + no-panic frame pulls, per
    /// the plan's documented fallback (a byte-exact non-silent tiny MOD is not
    /// practical to hand-build here).
    #[cfg(feature = "mod-music")]
    fn minimal_mod() -> Vec<u8> {
        let mut v = vec![0u8; 20];          // title
        v.extend(std::iter::repeat(0u8).take(31 * 30)); // 31 sample records
        v.push(1);                          // song length = 1 pattern in the order
        v.push(127);                        // restart position
        v.extend(std::iter::repeat(0u8).take(128)); // order table (pattern 0)
        v.extend_from_slice(b"M.K.");       // 4-channel tag
        v.extend(std::iter::repeat(0u8).take(64 * 4 * 4)); // one zero pattern
        v
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn load_mod_song_rejects_malformed_without_panic() {
        assert!(load_mod_song(&[0u8; 100]).is_none(), "short input is rejected, not panicked on");
        assert!(load_mod_song(&[]).is_none(), "empty input is rejected, not panicked on");
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn mod_source_reports_stereo_and_pulls_frames() {
        use rodio::Source;
        let song = load_mod_song(&minimal_mod()).expect("write+parse temp mod file");
        let state = mod_player::PlayerState::new(song.format.num_channels, SAMPLE_RATE);
        let mut src = ModSource {
            song,
            state,
            rate: SAMPLE_RATE,
            pending_right: None,
        };
        assert_eq!(src.channels(), 2, "MOD is decoded as stereo");
        assert_eq!(src.sample_rate(), SAMPLE_RATE);
        for _ in 0..16 {
            assert!(src.next().is_some(), "frames pull without panic");
        }
    }
}
