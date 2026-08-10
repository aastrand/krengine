//! Dump a module's timing and per-channel note layout, so sync can be based on
//! what the tune actually does rather than on tracker conventions.
//!
//! Runs the player headlessly (no audio device) and observes the rows it
//! resolves, which is simpler than reconstructing the pattern grid by hand.
//!
//! cargo run --release --example analyze -- path/to/song.xm

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use xmrs::prelude::*;
use xmrsplayer::audio_observer::{MixContext, MixObserver};
use xmrsplayer::observer::{PlayerObserver, RowContext};
use xmrsplayer::xmrsplayer::XmrsPlayer;

#[derive(Default)]
struct Stats {
    rows: usize,
    /// channel -> (note count, hits per row-slot within 16, instrument tally)
    channels: BTreeMap<usize, ChannelStats>,
    speeds: BTreeMap<usize, usize>,
    bpms: BTreeMap<usize, usize>,
    instrument_names: Vec<String>,
}

#[derive(Default)]
struct ChannelStats {
    notes: usize,
    slots: [usize; 16],
    pitches: BTreeMap<String, usize>,
}

/// Records the low-band envelope at 100 Hz so we can measure the actual kick
/// period, rather than trusting tracker tick conventions.
struct LowBandProbe {
    envelope: Arc<Mutex<Vec<f32>>>,
    lp: f32,
    coeff: f32,
    peak: f32,
    counter: u32,
}

impl MixObserver for LowBandProbe {
    fn on_mix(&mut self, ctx: &MixContext) {
        let x = (ctx.left as f32 + ctx.right as f32) * 0.5 / 32768.0;
        self.lp += (x - self.lp) * self.coeff;
        self.peak = self.peak.max(self.lp.abs());

        self.counter += 1;
        if self.counter >= 480 {
            // one bucket per 10ms
            self.counter = 0;
            self.envelope.lock().unwrap().push(self.peak);
            self.peak = 0.0;
        }
    }
}

/// Strongest periodicity in the envelope, by autocorrelation, searched over
/// musically plausible tempos.
fn dominant_period(envelope: &[f32], rate: f32) -> Option<(f32, f32)> {
    // Onset strength: positive change only, so we lock to attacks not sustain.
    let flux: Vec<f32> = envelope
        .windows(2)
        .map(|w| (w[1] - w[0]).max(0.0))
        .collect();
    if flux.len() < 1000 {
        return None;
    }

    let mean = flux.iter().sum::<f32>() / flux.len() as f32;
    let centered: Vec<f32> = flux.iter().map(|v| v - mean).collect();

    let min_lag = (rate * 60.0 / 220.0) as usize; // 220 BPM
    let max_lag = (rate * 60.0 / 60.0) as usize; // 60 BPM

    let mut best = (0usize, f32::MIN);
    for lag in min_lag..max_lag.min(centered.len() / 2) {
        let score: f32 = centered
            .iter()
            .zip(centered[lag..].iter())
            .map(|(a, b)| a * b)
            .sum::<f32>()
            / (centered.len() - lag) as f32;
        if score > best.1 {
            best = (lag, score);
        }
    }

    let seconds = best.0 as f32 / rate;
    Some((seconds, 60.0 / seconds))
}

struct Collector(Arc<Mutex<Stats>>);

impl PlayerObserver for Collector {
    fn on_row(&mut self, ctx: &RowContext<'_>) {
        let mut stats = self.0.lock().unwrap();
        stats.rows += 1;
        *stats.speeds.entry(ctx.tempo).or_default() += 1;
        *stats.bpms.entry(ctx.bpm).or_default() += 1;

        if stats.instrument_names.is_empty() {
            stats.instrument_names = ctx
                .module
                .instrument
                .iter()
                .map(|i| i.name.trim().to_string())
                .collect();
        }

        let slot = ctx.row % 16;
        for (channel, cell) in ctx.cells.iter().enumerate() {
            let Some(pitch) = cell.event.pitch() else {
                continue;
            };
            let entry = stats.channels.entry(channel).or_default();
            entry.notes += 1;
            entry.slots[slot] += 1;
            *entry.pitches.entry(format!("{pitch:?}")).or_default() += 1;
        }
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: analyze <module>");
    let bytes = std::fs::read(&path)?;
    let module = Module::load(&bytes).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    println!("name:        {}", module.name.trim());
    println!("channels:    {}", module.get_num_channels());
    println!("instruments: {}", module.instrument.len());
    println!(
        "defaults:    {} BPM, speed {}",
        module.default_bpm, module.default_tempo
    );
    println!(
        "editor grid: {} rows/beat, {} rows/measure",
        module.pattern_highlight.beat, module.pattern_highlight.measure
    );

    let stats = Arc::new(Mutex::new(Stats::default()));
    let mut player = XmrsPlayer::new(&module, 48_000, 0);
    player.set_max_loop_count(1);
    player.add_observer(Box::new(Collector(stats.clone())));

    let envelope = Arc::new(Mutex::new(Vec::new()));
    player.add_audio_mix_observer(Box::new(LowBandProbe {
        envelope: envelope.clone(),
        lp: 0.0,
        coeff: 1.0 - (-std::f32::consts::TAU * 140.0 / 48_000.0f32).exp(),
        peak: 0.0,
        counter: 0,
    }));

    println!("duration:    {:.1} s", player.duration_seconds());

    // Pull the whole song through the mixer, discarding audio.
    let mut samples = 0u64;
    let max_samples = 48_000 * 2 * 60 * 8; // eight minutes of stereo, a hard stop
    while player.next().is_some() && samples < max_samples {
        samples += 1;
    }

    let stats = stats.lock().unwrap();
    println!("rows played: {}", stats.rows);
    println!("speeds seen: {:?}", stats.speeds);
    println!("bpms seen:   {:?}", stats.bpms);

    // A tracker "beat" is 24 ticks, so rows-per-beat falls out of the speed.
    if let Some((&speed, _)) = stats.speeds.iter().max_by_key(|(_, n)| **n)
        && let Some((&bpm, _)) = stats.bpms.iter().max_by_key(|(_, n)| **n)
    {
        let rows_per_beat = 24.0 / speed as f32;
        let row_seconds = speed as f32 * 2.5 / bpm as f32;
        println!(
            "\n=> {rows_per_beat} rows/beat, {:.0} ms/row, {:.1} musical BPM",
            row_seconds * 1000.0,
            60.0 / (row_seconds * rows_per_beat)
        );
    }

    // What the kick actually does, measured rather than assumed.
    let envelope = envelope.lock().unwrap();
    if let Some((seconds, bpm)) = dominant_period(&envelope, 100.0) {
        let speed = *stats.speeds.iter().max_by_key(|(_, n)| **n).unwrap().0;
        let module_bpm = *stats.bpms.iter().max_by_key(|(_, n)| **n).unwrap().0;
        let row_seconds = speed as f32 * 2.5 / module_bpm as f32;
        println!(
            "\nmeasured kick period: {:.3} s = {:.1} BPM = {:.2} rows",
            seconds,
            bpm,
            seconds / row_seconds
        );
    }

    println!("\nchannel activity:");
    for (channel, cs) in &stats.channels {
        let name = cs
            .pitches
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(p, _)| p.clone())
            .unwrap_or_default();

        // Which sixteenth-note slots does this channel land on? A kick sits on
        // 0/4/8/12, a snare on 4/12, hats hit everything.
        let busiest = *cs.slots.iter().max().unwrap_or(&1).max(&1);
        let shape: String = cs
            .slots
            .iter()
            .map(|&n| match n * 4 / busiest {
                0 => '.',
                1 => '-',
                2 => '+',
                _ => '#',
            })
            .collect();

        println!(
            "  ch {channel:2}: {:5} notes  |{shape}|  top note {name}",
            cs.notes
        );
    }

    Ok(())
}
