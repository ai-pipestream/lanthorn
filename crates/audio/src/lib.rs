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
fn gain(master: u8, z_volume: u8) -> f32 {
    (master.min(100) as f32 / 100.0)
        * match z_volume {
            0 | 255 => 1.0,
            v => (v.min(8) as f32) / 8.0,
        }
}

/// A short decaying sine at `freq_hz` for `ms` at 44100 Hz (unit amplitude,
/// linear decay envelope). Volume is applied by the caller via the sink.
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

    /// Decode + play a sampled sound. Real decoding lands in the next task;
    /// for now this is a stub so both feature configs expose the same surface.
    pub fn play_sample(&mut self, _bytes: &[u8], _format: SoundFormat, _z_volume: u8, _repeats: u8) -> Option<SoundId> {
        None
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
}
