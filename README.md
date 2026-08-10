# krengine

A demo engine in Rust and wgpu. Liquid-metal metaballs inside a procedurally
textured room, stirred by a fluid simulation, cut to the music.

```
cargo run --release -- path/to/module.xm

# start thirty seconds in, tune and timeline together, to skip the intro
cargo run --release -- path/to/module.xm 30
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

## Architecture

Two threads and one direction of travel. The audio callback owns the tune and
writes into shared state; the render thread reads it, turns it into a handful of
numbers, and every one of those numbers reaches the GPU through a single uniform
buffer. No pass has state of its own beyond its textures.

```
                    ┌─────────────────────────────────────────┐
  audio thread      │ xmrsplayer ── mix ──> sound card        │
  (cpal callback)   │      │            │                     │
                    │      │            └─> sample counter    │
                    │      └─> ring buffer (8192 mono samples)│
                    └──────────────┬──────────────────────────┘
                                   │  SharedState: frames, ring,
                                   │  row/pattern, bpm, latency
                                   │  (atomics, never blocking)
                    ┌──────────────▼──────────────────────────┐
  render thread     │ audio.rs   Music::sample(dt) ──> Sync   │
  (winit, one       │              clock, FFT, onsets, accents │
   RedrawRequested  ├─────────────────────────────────────────┤
   per frame)       │ timeline.rs Stage::at / Director / Spin  │
                    │              what is on screen, and where│
                    │              the camera is              │
                    ├─────────────────────────────────────────┤
                    │ renderer.rs Uniforms ──> uniform buffer │
                    │              one write per frame        │
                    └──────────────┬──────────────────────────┘
                                   │  @group(0) for every pass,
                                   │  vertex + fragment + compute
                    ┌──────────────▼──────────────────────────┐
  GPU               │ passes/*.rs  encode one command buffer  │
                    │ shaders/*.wgsl (common.wgsl prepended)  │
                    └─────────────────────────────────────────┘
```

`Sync` is the only thing that crosses from sound to picture, and `Uniforms` the
only thing that crosses from CPU to GPU. Anything that wants to react to the
music reads a field of the uniform block — which is why the timeline is a table
in one file rather than time comparisons scattered through the shaders.

### Targets

```
  scene ─┐
         ├──> hdr (rgba16f, RENDER_SCALE x window) ──┬──> bloom mip chain ──┐
  fluid ─┤                                            │    (½ … 1/32)        │
  text  ─┘    depth (d32f, same size)                 └──> post ◀────────────┘
                 ▲ written by the raymarch,               │  tonemap, resolve,
                 │ read by particles and fluid            ▼  transition wipe
                 └── so the SDF sorts against          swapchain (sRGB)
                     rasterised geometry
```

Bloom is allocated from the *window* size, not the supersampled scene: blurred
highlights gain nothing from the extra pixels. The middle fluid sheet's dye is
bound a second time as the transition mask, which is why post takes two
different views of fluid output.

## A frame

`render()` in `renderer.rs` is the whole of it, in order:

```
  1  Stage::at(music)          timeline → card, fades, spike, dissolve, smoke
  2  Director::update(music)   cut list → eye, target, fov
  3  Spin::update(music)       integrate yaw/tilt at a music-driven rate
  4  write_buffer(uniforms)    one upload; view_proj, audio, bands, stage
  5  acquire swapchain texture (Outdated/Lost → reconfigure and try next frame)

  6  fluid.simulate     compute, per sheet (skipped when smoke ≈ 0)
       advect velocity → splat beads in → vorticity → divergence
       → 18x jacobi → project → advect dye → splat dye in
  7  scene.draw         fullscreen raymarch → hdr + depth
  8  particles.draw     beads, depth-tested (skipped once spike ≈ 1)
  9  fluid.draw         three sheets composited back to front over hdr
 10  text.draw          intro cards from the atlas — before bloom, so
                        letterforms feed the glow
 11  bloom.draw         threshold, downsample chain, upsample
 12  post.draw          tonemap + resolve hdr, add bloom, apply the dye
                        mask as the transition wipe → swapchain

 13  submit, present
```

Steps 6, 8 and 10 drop out when the timeline says there is nothing there —
work skipped rather than multiplied by zero.

## How time flows

Nothing in the demo reads a wall clock except the clock itself. There is one
timebase in seconds and one in beats, and everything hangs off those two.

```
  wall time (Instant)         audio sample counter
        │                             │
        │  smooth, free-running       │  exact, arrives in ~21ms steps
        └──────────────┬──────────────┘
                       ▼
             Music::clock()   wall + slow-filtered error − output latency + skip
                       │      monotonic: never steps backwards
                       ▼
                 music.time  ──────────────────> intro cards, scene fade
                       │                          (CARDS, SCENE_START)
                       │ x bpm/60
                       ▼
            music.beat_phase  ──────────────────> scene changes: spike, merge,
                  ▲    │                          dissolve, winding
   phase-lock ────┘    │                          shot lengths, in beats
   from on-beat onsets └──> bar_phase (÷4)
```

And the two event streams, both from the same FFT:

```
  ring buffer ──> Hann window ──> FFT 2048 ──> 16 log bands ──> uniforms.bands
                                                    │
                       spectral flux on bands 0..4  │  flux over all 16 bands
                                    ▼               ▼
                              onset detector    accent detector
                              (sensitivity 1.8) (sensitivity 3.4)
                                    │               │
                        pulse ──> music.beat        └──> music.hard_hit
                        attack/decay envelope            single frame, true
                                    │                    on an arrangement hit
                                    ▼                         │
                     card breathing, recoil, spin rate        ▼
                                                     Director cuts to the
                                                     next shot
```

A shot's beat count is when it becomes *willing* to cut; `hard_hit` is what
actually cuts it, with `CUT_GRACE` beats of patience before it gives up and cuts
anyway. So the structure is counted in beats and the edits land on accents.

With no module on the command line there is no audio thread: `main.rs` fills a
`Sync` from wall time and leaves every band at zero. The timeline still runs —
`beat_phase` stays at zero, so the scene changes never fire and you get the
first scene, held.

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
