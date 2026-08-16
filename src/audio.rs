//! Music playback and sync.
//!
//! The soundtrack is decoded up front and streamed from memory. The number of
//! frames handed to the sound card is the demo's master clock, keeping visuals
//! locked to the rendered song without relying on wall time.
//!
//! On top of that we run an FFT over the mix for a 16-band spectrum, and detect
//! bass onsets directly. The onset detector is what drives the beat pulse: a
//! metronome locked to the tune's BPM has the right period but no phase, so it
//! ticks happily between the kicks.

use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use lewton::inside_ogg::OggStreamReader;

/// Output latency is measured from the backend; this is the extra nudge for
/// everything it can't see (display pipeline, compositor). Positive = visuals
/// run later. Tune by eye — typical values are tens of milliseconds.
const LATENCY_OFFSET_MS: f32 = 0.0;

/// How quickly the clock is steered toward the audio position. Long enough
/// that buffer-sized steps are invisible, short enough to stay locked.
const CLOCK_TIME_CONSTANT: f32 = 0.75;

/// If we're further off than this, ease-in would be slower than a jump.
const RESYNC_THRESHOLD: f32 = 0.25;

/// Which spectrum bands the onset detector watches: the leftmost four bars in
/// the overlay, roughly 40-180Hz — kick and sub. Toggle the overlay with B to
/// re-pick these by eye.
const ONSET_BANDS: std::ops::Range<usize> = 0..4;

/// How hard an on-beat onset drags the beat phase into alignment. The phase
/// runs at the tune's tempo but starts arbitrary, so it needs pulling onto the
/// music; too high and syncopation yanks it around.
const PHASE_LOCK_GAIN: f32 = 0.3;
/// Only onsets this close to a beat boundary are treated as being *on* the
/// beat. Anything further off is syncopation and must not move the phase.
const PHASE_LOCK_WINDOW: f32 = 0.25;

/// How fast the beat pulse swells. Not instant: a step change in a value that
/// drives geometry reads as a pop rather than a hit.
const PULSE_ATTACK: f32 = 0.055;
/// How long it takes to fall away again.
const PULSE_DECAY: f32 = 0.28;

/// How far above the running average a rise must be to count as a hit.
const ONSET_SENSITIVITY: f32 = 1.8;
/// Absolute floor, so silence doesn't trigger on noise.
const ONSET_FLOOR: f32 = 0.012;
/// Minimum gap between hits — at 125 BPM a beat is 480ms, so this only
/// suppresses the ringing of a single kick.
const ONSET_COOLDOWN: f32 = 0.12;

/// The accent detector is deliberately fussier than the beat detector: it
/// watches the whole spectrum and demands a much bigger jump, so it fires on
/// the hits an arrangement actually lands on rather than on every kick.
const ACCENT_SENSITIVITY: f32 = 3.4;
const ACCENT_FLOOR: f32 = 0.05;
const ACCENT_COOLDOWN: f32 = 0.4;

/// Number of log-spaced spectrum bands handed to the shaders.
pub const BAND_COUNT: usize = 16;
const BAND_MIN_HZ: f32 = 40.0;
const BAND_MAX_HZ: f32 = 16_000.0;
const BAND_GAIN: f32 = 2.2;

/// FFT window. At 48kHz this is ~43ms of audio and ~23Hz per bin — enough
/// resolution to separate a kick from a bassline, short enough to stay snappy.
const FFT_SIZE: usize = 2048;
const RING_SIZE: usize = 8192;

/// A snapshot of the music, sampled once per frame.
#[derive(Clone, Copy, Default)]
pub struct Sync {
    /// Seconds elapsed, from the audio clock rather than wall time.
    pub time: f32,
    /// Log-spaced spectrum, BAND_COUNT bands from ~40 Hz to ~16 kHz.
    pub bands: [f32; BAND_COUNT],
    /// Convenience aggregates over `bands`: kick/bass, body, hats/air.
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    /// Decaying pulse fired on every beat — ease off this, don't gate on it.
    pub beat: f32,
    /// Beats since the song started; the fractional part is the beat's phase.
    pub beat_phase: f32,
    /// Position within a four-beat bar, 0..1.
    pub bar_phase: f32,
    /// True on the frame a hard transient lands — a peak across the whole
    /// spectrum, not just the bass. Cuts hang off this.
    pub hard_hit: bool,
    /// Backend-reported output latency, already applied to `time`.
    pub output_latency: f32,
    /// Seconds since the previous frame.
    pub dt: f32,
}

/// Shared between the audio thread and the render loop. All fields are written
/// by audio, read by rendering; f32s ride inside AtomicU32 as raw bits.
#[derive(Default)]
struct SharedState {
    frames: AtomicU64,
    ring: RingBuffer,
    /// Song tempo, loaded from the soundtrack's companion `.bpm` file.
    bpm: AtomicU32,
    /// Output latency in seconds, as reported by the backend: the gap between
    /// a callback running and those samples actually being audible.
    latency: AtomicU32,
    /// Master output gain, published by the visual timeline as raw f32 bits.
    gain: AtomicU32,
}

/// The audio thread's window into the recent past.
///
/// A plain ring buffer of mono samples: the audio callback writes, the render
/// thread reads the newest `FFT_SIZE` of them. Reads are unsynchronised on
/// purpose — a torn sample at the seam is invisible in a spectrum, and the
/// audio thread must never wait on rendering.
struct RingBuffer {
    samples: [AtomicU32; RING_SIZE],
    write: AtomicU64,
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self {
            samples: [const { AtomicU32::new(0) }; RING_SIZE],
            write: AtomicU64::new(0),
        }
    }
}

impl RingBuffer {
    fn push(&self, v: f32) {
        let index = self.write.fetch_add(1, Ordering::Relaxed) as usize;
        self.samples[index % RING_SIZE].store(v.to_bits(), Ordering::Relaxed);
    }

    /// Copy the most recent `out.len()` samples, oldest first.
    fn read_latest(&self, out: &mut [f32]) {
        let end = self.write.load(Ordering::Relaxed) as usize;
        let start = end.saturating_sub(out.len());
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = f32::from_bits(self.samples[(start + i) % RING_SIZE].load(Ordering::Relaxed));
        }
    }
}

/// Windowed FFT into log-spaced bands, run on the render thread once a frame.
struct Spectrum {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<rustfft::num_complex::Complex<f32>>,
    samples: Vec<f32>,
    /// Which FFT bin each band starts at.
    edges: [usize; BAND_COUNT + 1],
    bands: [f32; BAND_COUNT],
    /// Ring position at the last analysis, so we don't re-run the FFT over
    /// samples we've already seen — the audio thread only refills every buffer.
    last_seen: u64,
}

impl Spectrum {
    fn new(sample_rate: f32) -> Self {
        let fft = rustfft::FftPlanner::new().plan_fft_forward(FFT_SIZE);

        // Hann window, so a tone between bins doesn't smear across all of them.
        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();

        // Bands are log-spaced: pitch is logarithmic, so linear bands would put
        // almost everything in the top two.
        let bin_of = |hz: f32| ((hz / sample_rate) * FFT_SIZE as f32) as usize;
        let mut edges = [0usize; BAND_COUNT + 1];
        for (i, edge) in edges.iter_mut().enumerate() {
            let t = i as f32 / BAND_COUNT as f32;
            let hz = BAND_MIN_HZ * (BAND_MAX_HZ / BAND_MIN_HZ).powf(t);
            *edge = bin_of(hz).clamp(1, FFT_SIZE / 2 - 1);
        }

        Self {
            fft,
            window,
            scratch: vec![rustfft::num_complex::Complex::new(0.0, 0.0); FFT_SIZE],
            samples: vec![0.0; FFT_SIZE],
            edges,
            bands: [0.0; BAND_COUNT],
            last_seen: 0,
        }
    }

    /// Analyse the newest window and envelope-follow each band. Returns true
    /// when this call saw audio it hadn't analysed before.
    fn update(&mut self, ring: &RingBuffer, dt: f32) -> bool {
        let position = ring.write.load(Ordering::Relaxed);
        if position == self.last_seen {
            return false;
        }
        self.last_seen = position;

        ring.read_latest(&mut self.samples);

        for (i, slot) in self.scratch.iter_mut().enumerate() {
            *slot = rustfft::num_complex::Complex::new(self.samples[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        // Bands feed geometry, so they need slew limiting in both directions:
        // a raw spectrum is noisy frame to frame, and driving a surface with it
        // makes that surface visibly vibrate rather than pulse.
        let attack = 1.0 - (-dt / 0.045).exp();
        let release = (-dt / 0.16).exp();

        for band in 0..BAND_COUNT {
            let (from, to) = (
                self.edges[band],
                self.edges[band + 1].max(self.edges[band] + 1),
            );
            let mut sum = 0.0;
            for bin in from..to {
                sum += self.scratch[bin].norm();
            }
            // Mean magnitude, then a square root to compress the huge dynamic
            // range into something a shader can multiply by directly.
            let mean = sum / (to - from) as f32 / (FFT_SIZE as f32 * 0.25);
            let level = (mean.sqrt() * BAND_GAIN).min(1.5);
            let current = self.bands[band];
            self.bands[band] = if level > current {
                current + (level - current) * attack
            } else {
                (current * release).max(level)
            };
        }

        true
    }
}

/// Fires when the bass actually hits.
///
/// A metronome locked to the tune's BPM has the right period but no phase — it
/// happily ticks between the kicks forever. Detecting the onset instead means
/// the visuals land on what you can hear, and it needs no tempo at all.
#[derive(Default)]
struct OnsetDetector {
    previous: f32,
    /// Rolling average of recent rises, the threshold to beat.
    average_flux: f32,
    cooldown: f32,
}

impl OnsetDetector {
    /// `level` is the current bass energy. Returns true on an attack.
    fn update(&mut self, level: f32, dt: f32) -> bool {
        self.update_with(level, dt, ONSET_SENSITIVITY, ONSET_FLOOR, ONSET_COOLDOWN)
    }

    fn update_with(
        &mut self,
        level: f32,
        dt: f32,
        sensitivity: f32,
        floor: f32,
        cooldown: f32,
    ) -> bool {
        self.cooldown = (self.cooldown - dt).max(0.0);

        // Spectral flux: rises only. A decaying tail is not a new hit.
        let flux = (level - self.previous).max(0.0);
        self.previous = level;

        let hit = flux > self.average_flux * sensitivity + floor && self.cooldown <= 0.0;

        // Track the average *after* testing, so a hit doesn't raise the bar it
        // just cleared. Slow, so it follows the mix rather than single notes.
        let alpha = 1.0 - (-dt / 0.35).exp();
        self.average_flux += (flux - self.average_flux) * alpha;

        if hit {
            self.cooldown = cooldown;
        }
        hit
    }
}

pub struct Music {
    // Holding the stream alive is what keeps playback running.
    _stream: cpal::Stream,
    state: Arc<SharedState>,
    sample_rate: f32,
    /// Beats elapsed since the start of the song, accumulated from the audio
    /// clock at the current tempo. Counting rows instead would misalign
    /// wherever a pattern's length isn't a whole number of beats.
    beat_phase: f32,
    beat: f32,
    /// Trigger envelope behind `beat`, before attack smoothing.
    pulse: f32,
    epoch: std::time::Instant,
    last_time: f32,
    /// Filtered difference between the audio position and wall time.
    offset: f32,
    /// Seconds the tune was started into.
    skip: f32,
    locked: bool,
    spectrum: Spectrum,
    onset: OnsetDetector,
    accent: OnsetDetector,
    debug: bool,
    last_onset: f32,
}

impl Music {
    /// `skip` starts the tune that many seconds in. The clock is offset to
    /// match, so the timeline lands in the same place — handy for working on a
    /// section without watching the intro every time.
    pub fn start(path: &std::path::Path, skip: f32) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("opening soundtrack {}", path.display()))?;
        let mut decoder = OggStreamReader::new(BufReader::new(file))
            .with_context(|| format!("decoding Ogg soundtrack {}", path.display()))?;
        let source_rate = decoder.ident_hdr.audio_sample_rate;
        let source_channels = decoder.ident_hdr.audio_channels as usize;
        if source_channels == 0 {
            return Err(anyhow!("soundtrack has no audio channels"));
        }
        let mut samples = Vec::<[f32; 2]>::new();
        while let Some(packet) = decoder.read_dec_packet_itl()? {
            for frame in packet.chunks(source_channels) {
                let left = frame[0] as f32 / 32768.0;
                let right = frame.get(1).copied().unwrap_or(frame[0]) as f32 / 32768.0;
                samples.push([left, right]);
            }
        }

        let bpm_path = path.with_extension("bpm");
        let bpm: u32 = std::fs::read_to_string(&bpm_path)
            .with_context(|| format!("reading tempo from {}", bpm_path.display()))?
            .trim()
            .parse()
            .with_context(|| format!("invalid tempo in {}", bpm_path.display()))?;

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no audio output device")?;
        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;

        let state = Arc::new(SharedState::default());
        state.gain.store(1.0f32.to_bits(), Ordering::Relaxed);
        state.bpm.store(bpm, Ordering::Relaxed);

        let epoch = std::time::Instant::now();
        let stream = Self::build_stream(
            &device,
            &config,
            Arc::new(samples),
            source_rate,
            skip,
            state.clone(),
            channels,
        )?;
        stream.play()?;

        log::info!(
            "playing {} at {bpm} BPM ({} Hz, {} ch)",
            path.display(),
            sample_rate,
            channels
        );

        Ok(Self {
            _stream: stream,
            state,
            sample_rate: sample_rate as f32,
            // Started where the skip lands, or the timeline would run from
            // zero while the tune ran from the skip: every scene change is
            // keyed on beats, so they would all fire at the wrong time.
            beat_phase: skip * bpm as f32 / 60.0,
            beat: 0.0,
            pulse: 0.0,
            epoch,
            // Also the skip: the first frame measures elapsed time against
            // this, and starting from zero counted the whole skip a second
            // time.
            last_time: skip,
            offset: 0.0,
            skip,
            locked: false,
            spectrum: Spectrum::new(sample_rate as f32),
            onset: OnsetDetector::default(),
            accent: OnsetDetector::default(),
            debug: std::env::var("KR_DEBUG").is_ok(),
            last_onset: 0.0,
        })
    }

    fn build_stream(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        samples: Arc<Vec<[f32; 2]>>,
        source_rate: u32,
        skip: f32,
        state: Arc<SharedState>,
        channels: usize,
    ) -> anyhow::Result<cpal::Stream> {
        let err = |e| log::error!("audio stream error: {e}");

        let device_rate = config.sample_rate() as f64;
        let step = source_rate as f64 / device_rate;
        let mut position = skip.max(0.0) as f64 * source_rate as f64;
        let mut fill = move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
            // Samples handed over now become audible later. Without correcting
            // for this the visuals consistently lead the music.
            let ts = info.timestamp();
            let latency = ts.playback.duration_since(ts.callback).as_secs_f32();
            state.latency.store(latency.to_bits(), Ordering::Relaxed);

            let gain = f32::from_bits(state.gain.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let mut frames = 0u64;
            for frame in out.chunks_mut(channels) {
                let index = position as usize;
                let fraction = position.fract() as f32;
                let a = samples.get(index).copied().unwrap_or([0.0; 2]);
                let b = samples.get(index + 1).copied().unwrap_or(a);
                let l_raw = a[0] + (b[0] - a[0]) * fraction;
                let r_raw = a[1] + (b[1] - a[1]) * fraction;
                state.ring.push((l_raw + r_raw) * 0.5);
                let l = l_raw * gain;
                let r = r_raw * gain;
                for (i, sample) in frame.iter_mut().enumerate() {
                    *sample = if i % 2 == 0 { l } else { r };
                }
                position += step;
                frames += 1;
            }
            state.frames.fetch_add(frames, Ordering::Relaxed);
        };

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config.config(),
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| fill(data, info),
                err,
                None,
            )?,
            other => return Err(anyhow!("unsupported output sample format: {other:?}")),
        };

        Ok(stream)
    }

    /// Set the demo's master output level without blocking the audio callback.
    pub fn set_gain(&self, gain: f32) {
        self.state
            .gain
            .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// The demo's master clock.
    ///
    /// The sample counter is exact but arrives in buffer-sized steps (~21ms),
    /// so using it directly makes every animation advance in jerks. Wall time
    /// is perfectly smooth but free-runs against the sound card's crystal. So
    /// we run on wall time and steer it toward the audio clock with a slow
    /// filter: smooth frame to frame, exact over the length of a tune.
    fn clock(&mut self, dt: f32) -> f32 {
        let wall = self.epoch.elapsed().as_secs_f64() as f32;
        let audio = self.state.frames.load(Ordering::Relaxed) as f32 / self.sample_rate;
        let error = audio - wall;

        if !self.locked || (error - self.offset).abs() > RESYNC_THRESHOLD {
            // First frame, or something stalled badly enough that easing back
            // would take longer than just jumping.
            self.offset = error;
            self.locked = true;
        } else {
            let alpha = 1.0 - (-dt / CLOCK_TIME_CONSTANT).exp();
            self.offset += (error - self.offset) * alpha;
        }

        let latency = f32::from_bits(self.state.latency.load(Ordering::Relaxed));
        let time = wall + self.offset - latency + LATENCY_OFFSET_MS / 1000.0 + self.skip;
        self.last_time = time.max(self.last_time);
        self.last_time
    }

    /// Sample the music for this frame. `dt` drives the beat pulse's decay.
    pub fn sample(&mut self, dt: f32) -> Sync {
        let previous = self.last_time;
        let time = self.clock(dt);

        // One beat is 60/BPM seconds.
        let bpm = self.state.bpm.load(Ordering::Relaxed).max(1) as f32;
        let advanced = (time - previous).max(0.0) * bpm / 60.0;

        self.beat_phase += advanced;

        self.spectrum.update(&self.state.ring, dt);
        let bands = self.spectrum.bands;

        // Aggregates for the shaders that just want "is the kick hitting".
        let peak =
            |range: std::ops::Range<usize>| bands[range].iter().copied().fold(0.0f32, f32::max);
        let low = peak(0..4);

        // Detect the bass separately because only kicks are reliable anchors
        // for the beat grid. Strong full-spectrum attacks include the DnB
        // snares; those should drive the visible pulse too, but must not drag
        // the phase away from the kick.
        let bass_hit = self.onset.update(peak(ONSET_BANDS), dt);
        let full = bands.iter().copied().fold(0.0f32, f32::max);
        let hard_hit =
            self.accent
                .update_with(full, dt, ACCENT_SENSITIVITY, ACCENT_FLOOR, ACCENT_COOLDOWN);

        // The trigger envelope decays; `beat` chases it. Two stages, so the
        // rise is quick but still an ease rather than a jump.
        self.pulse *= (-dt / PULSE_DECAY).exp();
        if bass_hit || hard_hit {
            self.pulse = 1.0;
        }

        if bass_hit {
            // Phase-lock: ease the beat grid toward this hit when the hit is
            // near a beat line. Over a few bars the phase settles onto the
            // music instead of wherever the program happened to start.
            let nearest = self.beat_phase.round();
            if (self.beat_phase - nearest).abs() < PHASE_LOCK_WINDOW {
                self.beat_phase += (nearest - self.beat_phase) * PHASE_LOCK_GAIN;
            }
            if self.debug {
                log::info!(
                    "onset at {time:.2}s  (+{:.3}s since last, low {low:.2})",
                    time - self.last_onset
                );
            }
            self.last_onset = time;
        }
        self.beat += (self.pulse - self.beat) * (1.0 - (-dt / PULSE_ATTACK).exp());

        Sync {
            time,
            hard_hit,
            beat_phase: self.beat_phase,
            bar_phase: (self.beat_phase * 0.25).fract(),
            bands,
            low,
            mid: peak(4..10),
            high: peak(10..BAND_COUNT),
            beat: self.beat,
            output_latency: f32::from_bits(self.state.latency.load(Ordering::Relaxed)),
            dt,
        }
    }
}
