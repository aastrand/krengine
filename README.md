# krengine

A demo engine in Rust and wgpu. Liquid-metal metaballs inside a procedurally
textured room, stirred by a fluid simulation, cut to the music.

```
cargo run --release -- path/to/module.xm
```

The tune is a command-line argument, not an asset — it runs silent without one,
driving the visuals from wall time instead.

| key | |
|-----|-----|
| `B` | spectrum overlay: 16 bars, for picking a band to drive something with |
| `Esc` | quit |

`KR_DEBUG=1` reports frame times and the measured audio latency.

## Layout

```
src/
  main.rs        window, event loop, frame clock
  gpu.rs         instance / adapter / device / queue / surface
  renderer.rs    uniforms, render targets, frame orchestration
  audio.rs       playback, the demo clock, FFT, beat and accent detection
  timeline.rs    when things happen: intro cards, camera cut list
  text.rs        font rasterisation into an atlas
  shader.rs      prepends common.wgsl to every module
  passes/        one file per pass
  shaders/       one WGSL file per pass, plus the shared prelude
examples/
  analyze.rs     dumps a module's tempo, kick period and per-channel layout
```

A frame runs: scene → fluid → particles → text → bloom → post.

## How it works

Notes on the parts where the obvious approach is the wrong one.

**The clock is the audio, but it runs on wall time.** Sample counts are exact
but arrive in buffer-sized steps, so driving animation from them directly makes
everything advance in ~21ms jerks. Wall time is smooth but free-runs against the
sound card's crystal. So the clock runs on wall time and a slow filter steers it
toward the audio position: smooth frame to frame, exact over a four-minute tune.
Output latency is read from the backend and subtracted, because samples handed
to the device are not yet audible.

**Beats are detected, not counted.** A metronome locked to the tune's BPM has
the right period but no phase — it ticks happily between the kicks forever.
Onsets are found by spectral flux on the low bands instead, so the visuals land
on what you can hear, and no tempo analysis is needed for the next track. A
second detector watches the whole spectrum at a much higher threshold; camera
cuts hang off that one, so they land on accents rather than on every kick.

**The fluid is 2D, on purpose.** Filament detail scales with cells-per-axis, and
in 3D that cost is cubic: a 64³ grid is only 64 cells across and numerical
diffusion flattens every filament within a few frames. The same budget buys a
thousand cells per axis in 2D. Three sheets at different depth bands give the
volume back — a bead stirs the sheet its own distance falls in, and the sheets
composite back to front against the depth buffer.

**Beads are obstacles, not emitters.** Injecting velocity makes a jet, which
billows. Blending the fluid *toward* a bead's own velocity makes flow around a
solid and sheds a vortex street. Injection is rasterised — one small quad per
bead — because a per-cell loop costs cells × beads and caps the swarm at a
sample of itself. Alpha blending then computes the constraint for free:
`dst*(1-a) + src*a` is exactly `mix(fluid, bead velocity, grip)`.

**The dye is shaded by its gradient.** Density alone reads as flat fog; its
slope is what exposes the shear layers between vortices.

**Camera cuts, and they land on the music.** A shot's beat count is when it
becomes willing to cut, not when it does — it then waits for the next accent.
Dollies arc while they travel, since a move straight along the view axis reads
as a zoom, and the arc stays linear even where the push has eased, so the camera
never fully stops.

## Tuning

Most knobs are named constants at the top of the file that owns them.

| | |
|---|---|
| `audio.rs` | `LATENCY_OFFSET_MS` if the sync reads early or late by eye; `ONSET_BANDS`, `ONSET_SENSITIVITY`, `ACCENT_*` |
| `timeline.rs` | `CARDS`, `SCENE_START`, the `SHOTS` cut list |
| `shaders/fluid.wgsl` | `VORTICITY`, `DYE_DISSIPATION` |
| `shaders/splat.wgsl` | `COUPLING` (how hard beads grip), `EMISSION` |
| `shaders/common.wgsl` | `VEIN_*`, `BLOB_*` |
| `renderer.rs` | `RENDER_SCALE` — supersampling, drop to 1 to reclaim frame time |

## Assets and licensing

`assets/DejaVuSans-Bold.ttf` ships with its license alongside it.

No music is committed. The track is passed on the command line, and shipping one
means clearing it with whoever wrote it.
