//! Music playback and sync.
//!
//! The tune is a tracker module, which gives us two things a plain audio file
//! can't: an exact sample counter to use as the demo's master clock, and the
//! pattern/row cursor, so visuals can land on musical structure rather than on
//! guessed beats.
//!
//! On top of that we run an FFT over the mix for a 16-band spectrum, and detect
//! bass onsets directly. The onset detector is what drives the beat pulse: a
//! metronome locked to the tune's BPM has the right period but no phase, so it
//! ticks happily between the kicks.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use xmrs::prelude::Module;
use xmrsplayer::audio_observer::{MixContext, MixObserver};
use xmrsplayer::observer::{PlayerObserver, RowContext};
use xmrsplayer::xmrsplayer::XmrsPlayer;

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
    pub row: u32,
    pub pattern: u32,
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
    row: AtomicU32,
    pattern: AtomicU32,
    /// Bumped on every new row, so the render loop can detect row changes
    /// without polling faster than the music.
    row_serial: AtomicU64,
    /// Song tempo, republished each row so a mid-song change is picked up.
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

/// Feeds the ring buffer from the mix.
struct Tap {
    state: Arc<SharedState>,
}

impl MixObserver for Tap {
    fn on_mix(&mut self, ctx: &MixContext) {
        let x = (ctx.left as f32 + ctx.right as f32) * 0.5 / 32768.0;
        self.state.ring.push(x);
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

/// Publishes the pattern/row cursor as the song advances.
struct RowTracker {
    state: Arc<SharedState>,
}

impl PlayerObserver for RowTracker {
    fn on_row(&mut self, ctx: &RowContext<'_>) {
        self.state.row.store(ctx.row as u32, Ordering::Relaxed);
        self.state
            .pattern
            .store(ctx.pattern as u32, Ordering::Relaxed);
        self.state.row_serial.fetch_add(1, Ordering::Relaxed);
        self.state.bpm.store(ctx.bpm as u32, Ordering::Relaxed);
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
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        // The player borrows the module for its whole life, and that life is
        // the process's — leaking is simpler than a self-referential struct.
        let module: &'static Module =
            Box::leak(Box::new(Module::load(&bytes).map_err(|e| {
                anyhow!("{path:?} is not a module we can read: {e:?}")
            })?));

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no audio output device")?;
        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;

        let state = Arc::new(SharedState::default());
        state.gain.store(1.0f32.to_bits(), Ordering::Relaxed);
        // Seed the tempo so the beat clock is right before the first row lands.
        state
            .bpm
            .store(module.default_bpm as u32, Ordering::Relaxed);

        let mut player = XmrsPlayer::new(module, sample_rate, 0);
        player.set_max_loop_count(0); // loop forever
        player.add_audio_mix_observer(Box::new(Tap {
            state: state.clone(),
        }));
        player.add_observer(Box::new(RowTracker {
            state: state.clone(),
        }));

        if skip > 0.0 {
            // XM runs at bpm * 2/5 ticks a second.
            let tick = (skip * module.default_bpm as f32 * 0.4) as u32;
            player.goto_tick(tick);
            log::info!("starting {skip:.1}s in");
        }

        let epoch = std::time::Instant::now();
        let player = Arc::new(Mutex::new(player));
        let stream = Self::build_stream(&device, &config, player, state.clone(), channels)?;
        stream.play()?;

        log::info!(
            "playing {} ({} Hz, {} ch)",
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
            beat_phase: skip * module.default_bpm as f32 / 60.0,
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
        player: Arc<Mutex<XmrsPlayer<'static>>>,
        state: Arc<SharedState>,
        channels: usize,
    ) -> anyhow::Result<cpal::Stream> {
        let err = |e| log::error!("audio stream error: {e}");

        // The player yields interleaved stereo i16; fan that out to however
        // many channels the device wants.
        let fill = move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
            // Samples handed over now become audible later. Without correcting
            // for this the visuals consistently lead the music.
            let ts = info.timestamp();
            let latency = ts.playback.duration_since(ts.callback).as_secs_f32();
            state.latency.store(latency.to_bits(), Ordering::Relaxed);

            let mut player = player.lock().unwrap();
            let gain = f32::from_bits(state.gain.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let mut frames = 0u64;
            for frame in out.chunks_mut(channels) {
                let l = player.next().unwrap_or(0) as f32 / 32768.0 * gain;
                let r = player.next().unwrap_or(0) as f32 / 32768.0 * gain;
                for (i, sample) in frame.iter_mut().enumerate() {
                    *sample = if i % 2 == 0 { l } else { r };
                }
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

        // One beat is 60/BPM seconds — the tracker's speed cancels out of
        // (24/speed rows per beat) x (speed*2.5/bpm seconds per row).
        let bpm = self.state.bpm.load(Ordering::Relaxed).max(1) as f32;
        let advanced = (time - previous).max(0.0) * bpm / 60.0;

        self.beat_phase += advanced;

        self.spectrum.update(&self.state.ring, dt);
        let bands = self.spectrum.bands;

        // Aggregates for the shaders that just want "is the kick hitting".
        let peak =
            |range: std::ops::Range<usize>| bands[range].iter().copied().fold(0.0f32, f32::max);
        let low = peak(0..4);

        // The trigger envelope decays; `beat` chases it. Two stages, so the
        // rise is quick but still an ease rather than a jump.
        self.pulse *= (-dt / PULSE_DECAY).exp();
        if self.onset.update(peak(ONSET_BANDS), dt) {
            self.pulse = 1.0;

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

        // Whole-spectrum peak: the accents a cut should land on.
        let full = bands.iter().copied().fold(0.0f32, f32::max);
        let hard_hit =
            self.accent
                .update_with(full, dt, ACCENT_SENSITIVITY, ACCENT_FLOOR, ACCENT_COOLDOWN);

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
            row: self.state.row.load(Ordering::Relaxed),
            output_latency: f32::from_bits(self.state.latency.load(Ordering::Relaxed)),
            dt,
            pattern: self.state.pattern.load(Ordering::Relaxed),
        }
    }
}
