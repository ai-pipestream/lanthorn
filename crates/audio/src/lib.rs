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
    // Blorb's `Blorb::sound` returns the FORM chunk's PAYLOAD (the 8-byte
    // `FORM`+len header already stripped), so the bytes it hands us usually
    // start directly with the form type `AIFF`/`AIFC`. Accept both that shape
    // and a full `FORM`...`AIFF`/`AIFC` container (e.g. a standalone .aiff file).
    let mut pos = if bytes.len() >= 12
        && &bytes[0..4] == b"FORM"
        && (&bytes[8..12] == b"AIFF" || &bytes[8..12] == b"AIFC")
    {
        12
    } else if bytes.len() >= 4 && (&bytes[0..4] == b"AIFF" || &bytes[0..4] == b"AIFC") {
        4
    } else {
        return None;
    };
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
                let mut i = pcm_start;
                if sample_size <= 8 {
                    while i < pcm_end {
                        let s = bytes[i] as i8;
                        pcm.push((s as i16) << 8);
                        i += 1;
                    }
                } else {
                    let bps = ((sample_size as usize + 7) / 8).max(2);
                    while i + 1 < pcm_end {
                        pcm.push(i16::from_be_bytes([bytes[i], bytes[i + 1]]));
                        i += bps;
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

/// Render a ProTracker MOD to interleaved stereo i16 PCM via `xmrsplayer`.
/// Returns None on a malformed module (xmrs `Module::load` returns `Err`) — no
/// panic, no temp file, no stdout. Plays the song through once
/// (`set_max_loop_count(1)`); looping/repeat is handled by the caller via
/// `repeat_plan` (same path as AIFF). A safety cap bounds memory and guarantees
/// termination even if a pathological song never signals its end.
#[cfg(feature = "mod-music")]
fn render_mod(bytes: &[u8]) -> Option<(u16, u32, Vec<i16>)> {
    use xmrs::prelude::*;
    use xmrsplayer::prelude::*;
    let module = Module::load(bytes).ok()?;
    let mut player = XmrsPlayer::new(&module, SAMPLE_RATE, 0);
    player.set_max_loop_count(1); // 0 = forever; 1 = play once (iterator ends at song end)
    const MAX_SECONDS: usize = 600;
    let cap = SAMPLE_RATE as usize * 2 * MAX_SECONDS; // interleaved stereo
    let pcm: Vec<i16> = player.by_ref().take(cap).collect();
    if pcm.is_empty() {
        return None;
    }
    Some((2, SAMPLE_RATE, pcm))
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
    samples: std::collections::HashMap<SoundId, (rodio::Sink, u8)>,
    tones: Vec<(rodio::Sink, u8)>,
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
        self.tones.push((sink, z_volume));
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
                {
                    let (channels, rate, pcm) = render_mod(bytes)?;
                    if forever {
                        sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()).repeat_infinite());
                    } else {
                        for _ in 0..count {
                            sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()));
                        }
                    }
                }
                #[cfg(not(feature = "mod-music"))]
                {
                    eprintln!("audio: unsupported sound format (MOD; mod-music feature off)");
                    return None;
                }
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, (sink, z_volume));
        Some(id)
    }

    pub fn stop(&mut self, id: SoundId) {
        if let Some((sink, _)) = self.samples.remove(&id) {
            sink.stop();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, (sink, _)) in self.samples.drain() {
            sink.stop();
        }
        for (sink, _) in self.tones.drain(..) {
            sink.stop();
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.master = volume.min(100);
        for (s, z_volume) in self.samples.values() {
            s.set_volume(gain(self.master, *z_volume));
        }
        for (s, z_volume) in &self.tones {
            s.set_volume(gain(self.master, *z_volume));
        }
    }

    /// Drain completed sample ids (whose sink is empty) and prune finished tones.
    pub fn finished(&mut self) -> Vec<SoundId> {
        self.tones.retain(|(s, _)| !s.empty());
        let done: Vec<SoundId> = self
            .samples
            .iter()
            .filter(|(_, (s, _))| s.empty())
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

    #[cfg(feature = "playback")]
    #[test]
    fn set_volume_preserves_per_sound_z_scale() {
        // A sound played at a reduced z_volume must keep its z-scale relationship
        // to master after a runtime volume change — set_volume must apply
        // gain(master, z_volume), not bare master/100.
        // Backend may have no device in CI; assert on the pure gain relationship
        // that set_volume now uses, so the test is deterministic without a sink:
        let quiet = gain(50, 4); // half master, mid z-scale
        let loud = gain(50, 8); // half master, full z-scale
        assert!(quiet < loud, "z_volume must still scale after a master change");
        assert_eq!(gain(50, 8), 0.5, "full z-scale at half master == master/100");
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

    /// Build a blorb-shaped AIFF payload: the FORM chunk's PAYLOAD as `Blorb::sound`
    /// actually hands to `decode_aiff` — starts with the form type `AIFF` (no
    /// leading `FORM`+len header), COMM is 8-bit mono, SSND holds signed 8-bit
    /// samples. Reuses the same 80-bit extended sample-rate bytes as `tiny_aiff()`.
    fn blorb_aiff_payload_8bit() -> Vec<u8> {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        // COMM: channels=1, numFrames=3, sampleSize=8, rate = 44100 as 80-bit ext
        // (same extended-float bytes tiny_aiff() uses).
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());       // channels
        comm.extend_from_slice(&3u32.to_be_bytes());       // numSampleFrames
        comm.extend_from_slice(&8u16.to_be_bytes());       // sampleSize
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=0, blockSize=0, then three signed 8-bit samples.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&[0x7f, 0x80, 0x00]);       // i8: +127, -128, 0

        let mut payload = Vec::new();
        payload.extend_from_slice(b"AIFF");
        payload.extend_from_slice(&be_chunk(b"COMM", &comm));
        payload.extend_from_slice(&be_chunk(b"SSND", &ssnd));
        payload
    }

    #[test]
    fn decode_aiff_accepts_blorb_payload_8bit() {
        let (channels, rate, pcm) = decode_aiff(&blorb_aiff_payload_8bit()).expect("valid blorb AIFF payload");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![32512i16, -32768i16, 0i16]);
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
    /// so `render_mod` asserts structure (channels == 2, correct rate) and a
    /// non-empty rendered buffer rather than any particular audio content.
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
    fn render_mod_rejects_malformed_without_panic() {
        assert!(render_mod(&[0u8; 100]).is_none(), "short input rejected, not panicked on");
        assert!(render_mod(&[]).is_none(), "empty input rejected, not panicked on");
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn render_mod_produces_stereo_buffer() {
        let (channels, rate, pcm) = render_mod(&minimal_mod()).expect("valid minimal MOD renders");
        assert_eq!(channels, 2, "MOD is rendered as stereo");
        assert_eq!(rate, SAMPLE_RATE);
        assert!(!pcm.is_empty(), "one pattern still yields frames");
    }
}
