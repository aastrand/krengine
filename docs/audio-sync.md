# Rebuilding soundtrack sync markers

The demo does not guess its important edits from a metronome. The fractal drop,
signal-cube notes, and final cube-sea drums were measured from isolated stems,
then written into `src/timeline.rs`. This keeps the authored visuals stable at
runtime while still landing on events that are not perfectly quantized.

This file records the source offset, analysis commands, conversions, and manual
choices needed to repeat that work after replacing or recutting the music.

## Current timebase

- Original master and stems start at the same zero point.
- The bundled Ogg starts **176.0 seconds** into that master.
- The bundled tempo is **180 BPM**, from `assets/neuro_architecture.bpm`.
- Therefore:

```text
cut_seconds = source_or_stem_seconds - 176.0
timeline_beats = cut_seconds * 180 / 60
timeline_beats = cut_seconds * 3
```

Always measure against the isolated stem first, then confirm the result against
`assets/neuro_architecture.ogg`. The latter catches a changed cut offset or an
encoding/alignment mistake.

## Rebuilding the bundled Ogg

The current asset is stereo Vorbis at 48 kHz, starts at master time `176.0s`,
and lasts `101.333333s`—304 beats at 180 BPM. A compatible recut is:

```sh
master="$HOME/Downloads/Neuro Architecture.wav"
ffmpeg -y -ss 176.0 -t 101.333333 -i "$master" \
  -c:a libvorbis -q:a 8 -ar 48000 -ac 2 \
  assets/neuro_architecture.ogg
```

Vorbis quality can change without affecting the authored timings, but changing
`-ss`, resampling, adding silence, or exporting stems from another time origin
does affect them. Verify the resulting asset before measuring:

```sh
ffprobe -v error \
  -show_entries format=duration:stream=codec_name,sample_rate,channels \
  -of default=nw=1 assets/neuro_architecture.ogg
```

If the demo length changes, calculate the new audio duration from its terminal
timeline beat rather than trimming by ear:

```text
duration_seconds = DEMO_END_BEATS * 60 / BPM
```

## Fractal drop

The intended drop is not the earlier downbeat inside the DnB build. It is the
large isolated drum slam after the short quiet/noise pocket.

The current drum stem places that transient at approximately `211.20s` in the
master:

```text
211.20 - 176.0 = 35.20 cut seconds
35.20 * 3 = beat 105.60
```

That produces `FRACTAL_DROP_BEATS = 105.6`.

To inspect the isolated drums around the hit in 50 ms windows:

```sh
stem="$HOME/Downloads/Neuro Architecture Stems/1 Drums.wav"
for offset in 210.80 210.90 211.00 211.10 211.15 211.20 211.25 211.30; do
  ffmpeg -hide_banner -loglevel info -ss "$offset" -t 0.05 -i "$stem" \
    -af 'astats=metadata=1:reset=0' -f null - 2>&1 \
    | sed -n "s/.*RMS level dB: /$offset /p" | tail -1
done
```

Confirm the visible jump in the final Ogg around cut time `35.20s`:

```sh
for offset in 35.00 35.05 35.10 35.15 35.20 35.25 35.30; do
  ffmpeg -hide_banner -loglevel info -ss "$offset" -t 0.05 \
    -i assets/neuro_architecture.ogg \
    -af 'highpass=f=80,lowpass=f=12000,astats=metadata=1:reset=0' \
    -f null - 2>&1 \
    | sed -n "s/.*RMS level dB: /$offset /p" | tail -1
done
```

`COLLAPSE_BEATS` is intentionally earlier than the drop. It puts the hidden
geometry swap inside the quiet white/noise pocket. Only the white cover's exit
is keyed directly to `FRACTAL_DROP_BEATS`, so the first visible fractal frame
lands on the slam.

## Signal-cube notes

The 14 lead notes are slightly off the 180 BPM grid, so they remain measured in
seconds:

```text
SIGNAL_TEXT_START   = 43.776 cut seconds
SIGNAL_TEXT_CADENCE = 0.514 seconds
SIGNAL_TEXT_HOLD    = 0.420 seconds
```

These came from the isolated synth lead. When changing music, find the first
note onset, measure several successive onset-to-onset intervals, and use their
median as the cadence. Do not convert this motif to beats unless the replacement
is demonstrably quantized.

## Lens focus pulls

Lens-to-lens focus changes use selected strong low-drum accents rather than an
even eight-beat timer. The camera chooses a comfortably framed near/far subject
on each cue, follows that membrane's animated front surface, and holds it until
the next cue.

| Cut seconds | Timeline beats |
| ---: | ---: |
| 54.44 | 163.32 |
| 56.66 | 169.98 |
| 59.42 | 178.26 |
| 62.14 | 186.42 |
| 64.90 | 194.70 |

These values form `LENS_FOCUS_CUES`. They were selected from the low-passed
drum candidates below: approximately one musically strong change every two to
three seconds, not a rack on every kick.

```sh
stem="$HOME/Downloads/Neuro Architecture Stems/1 Drums.wav"
ffmpeg -hide_banner -loglevel info -ss 228 -t 16 -i "$stem" \
  -af 'lowpass=f=220,asetnsamples=n=960:p=0,astats=metadata=1:reset=1,ametadata=print:key=lavfi.astats.Overall.RMS_level' \
  -f null - 2>&1 \
  | sed -n 's/.*pts_time:\([^ ]*\).*/TIME \1/p; s/.*RMS_level=\(.*\)/RMS \1/p' \
  | paste - - \
  | awk '
      BEGIN { last = -1 }
      {
        time = $2
        level = $4
        if (previous > before_previous && previous >= level && previous > -22 && previous_time - last > 0.30) {
          cut_time = 52 + previous_time
          printf "cut %.3f  beat %.2f  rms %.1f\n", cut_time, cut_time * 3, previous
          last = previous_time
        }
        before_previous = previous
        previous = level
        previous_time = time
      }
    '
```

## Cube-sea transition and crests

The final transition uses three drum-stem events:

| Role | Cut seconds | Timeline beats |
| --- | ---: | ---: |
| Cover starts | 79.98 | 239.94 |
| Cover fills; scene/camera swap | 80.48 | 241.44 |
| Cube sea is uncovered | 81.34 | 244.02 |

Those values are `CUBE_TRANSITION_START_BEATS`,
`CUBE_TRANSITION_COVER_BEATS`, and `CUBE_TRANSITION_END_BEATS`.

The continuing kick and low-tom map is `CUBE_DRUM_HITS`. Each entry is
`(cut_seconds, strength)`. Times are measured; strengths are deliberately
art-directed. Main kicks use roughly `0.85-1.0`, secondary hits use lower
values, and the real `84.10-87.32s` drum gap is left empty so the sea settles.
These hits add travelling sine crests to the continuous concentric/diagonal
sea; they do not replace its phase with independent per-cube launches.

The following command reproduces the candidate low-frequency peaks. It reads
20 ms windows, low-passes the drum stem at 220 Hz, keeps local maxima above
`-30 dB`, and enforces a 200 ms cooldown:

```sh
stem="$HOME/Downloads/Neuro Architecture Stems/1 Drums.wav"
cut_source_seconds=176
analysis_source_seconds=254
ffmpeg -hide_banner -loglevel info \
  -ss "$analysis_source_seconds" -t 14 -i "$stem" \
  -af 'lowpass=f=220,asetnsamples=n=960:p=0,astats=metadata=1:reset=1,ametadata=print:key=lavfi.astats.Overall.RMS_level' \
  -f null - 2>&1 \
  | sed -n 's/.*pts_time:\([^ ]*\).*/TIME \1/p; s/.*RMS_level=\(.*\)/RMS \1/p' \
  | paste - - \
  | awk -v source_start="$analysis_source_seconds" -v cut_start="$cut_source_seconds" '
      BEGIN { last = -1 }
      {
        time = $2
        level = $4
        if (previous > before_previous && previous >= level && previous > -30 && previous_time - last > 0.20) {
          source_time = source_start + previous_time
          printf "source %.3f  cut %.3f  rms %.1f\n", source_time, source_time - cut_start, previous
          last = previous_time
        }
        before_previous = previous
        previous = level
        previous_time = time
      }
    '
```

Review those candidates by ear. Do not blindly paste every peak into the map:
hats, ringing tails, and close double peaks can pass an RMS threshold without
being useful crest cues. Preserve meaningful empty sections rather than
filling them with a synthetic beat grid.

## Replacement checklist

1. Export the new master and all stems from the same zero point.
2. Record the master-time offset used to cut the bundled Ogg.
3. Update `assets/neuro_architecture.bpm` if the tempo changed.
4. Measure the semantic events from the relevant isolated stems.
5. Convert stem time to cut seconds, then to beats only for genuinely
   beat-based sections.
6. Update the named constants and `CUBE_DRUM_HITS` in `src/timeline.rs`.
7. Update the timeline tests that pin the measured boundaries and drum gap.
8. Build release mode, then audition focused windows with:

```sh
KR_BENCH=8 target/release/krengine 33
KR_BENCH=15 target/release/krengine 78
```

9. Finally watch the complete demo. Focused skips prove local sync, but only a
full run reveals whether the preceding musical buildup makes the edit feel
correct.
