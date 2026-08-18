# krengine

An audio-reactive demo built with Rust and wgpu. It combines raymarched liquid
metal, a fractal landscape, living lenses, a morphing tunnel, a gravitational
cube sea, particle strings, bloom, and depth of field.

## Run

```sh
cargo run --release
```

Pass an optional number of seconds to start both the music and timeline there:

```sh
cargo run --release -- 30
```

The demo always uses the bundled Ogg soundtrack.

Controls:

- `B` toggles the FFT-band overlay.
- `F` toggles borderless fullscreen.
- `Esc` exits.

Environment:

- `KR_DEBUG=1` logs frame timing and measured audio latency.
- `KR_SCALE=<factor>` overrides the supersampling factor. The scene pass is
  around 95% of the frame and scales linearly with pixel count, so this is the
  one knob that matters on a slower GPU. Halving it roughly triples the frame
  rate.
- `KR_BENCH=<seconds>` exits after that long, for unattended timing runs.

## Project structure

```text
src/
  main.rs             window, event loop, and command-line arguments
  audio.rs            Ogg playback, FFT analysis, beats, and demo clock
  timeline.rs         section timing, edit decisions, and shared camera state
  timeline/cameras.rs lens and tunnel camera rigs and safety constraints
  renderer.rs         per-frame uniforms and render-pass ordering
  fractal.rs          fractal paths and particle-string corridors
  text.rs             font atlas and text geometry
  gpu.rs              wgpu device, surface, and swapchain setup
  shader.rs           WGSL source composition and shader validation

  passes/              Rust-side GPU pipeline and resource management
  shaders/             WGSL for scenes, particles, bloom, text, and post
  shaders/scenes/      one WGSL fragment per raymarched 3D effect
```

The main 3D effects live in `shaders/scenes/{blob,fractal,lenses,tunnel}.wgsl`.
Rust concatenates those fragments with `shaders/scene.wgsl` into one shader
module and executes it through a single fullscreen scene pass. This preserves
covered transitions between effects without putting every effect in one file.

## Frame pipeline

Each frame is encoded in this order:

1. Sample audio and evaluate the timeline.
2. Update the camera, focus, motion, and shared uniform buffer.
3. Raymarch the active 3D scene into HDR color and depth.
4. Draw particles and text.
5. Extract bloom.
6. Apply depth of field, transitions, tonemapping, and output to the window.

The ownership boundaries are:

- `timeline/`: when and where things happen.
- `shaders/scenes/`: what each main 3D effect looks like.
- `passes/`: how GPU pipelines and textures are configured.
- `renderer.rs`: the order in which GPU work runs.

Audio analysis produces a `Sync` value for the timeline. The renderer packs
the resulting stage, camera, FFT bands, and motion values into one uniform
buffer shared by the GPU passes.

## Development

Run the validation suite with:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo build --release --all-targets
cargo test --release
```

That is exactly what CI gates on, in the same order. Running less than all four
is how a red build gets pushed.

The tests parse and validate the fully composed WGSL scene shader, check that
CPU and GPU uniform array sizes match, and exercise camera/fractal constraints.

Useful tuning locations:

- `audio.rs`: onset, accent, and latency tuning.
- `timeline.rs`: section lengths and edit timing.
- `timeline/cameras.rs`: lens and tunnel camera behavior.
- `renderer.rs`: supersampling, focus response, and particle counts.
- `shaders/scenes/`: effect geometry, materials, and audio response.
- `shaders/post.wgsl` and `shaders/bloom.wgsl`: final image treatment.

The stem-analysis process and exact timebase conversions behind the authored
sync markers are recorded in [`docs/audio-sync.md`](docs/audio-sync.md). Use it
when replacing or recutting the soundtrack rather than retiming by eye from
scratch.

## Assets

`assets/DejaVuSans-Bold.ttf` ships with its license. No music is included; the
module is supplied at runtime.
