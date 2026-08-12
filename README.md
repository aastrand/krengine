# krengine

An audio-reactive demo built with Rust and wgpu. It combines raymarched liquid
metal, a fractal landscape, living lenses, a morphing tunnel, particle strings,
fluid simulation, bloom, and depth of field.

## Run

```sh
cargo run --release -- path/to/module.xm
```

Pass a second argument to start both the music and timeline at a given number
of seconds:

```sh
cargo run --release -- path/to/module.xm 30
```

The demo can run without a module, but audio-reactive motion and beat-driven
timeline changes require one.

Controls:

- `B` toggles the FFT-band overlay.
- `Esc` exits.
- `KR_DEBUG=1` logs frame timing and measured audio latency.

## Project structure

```text
src/
  main.rs             window, event loop, and command-line arguments
  audio.rs            module playback, FFT analysis, beats, and demo clock
  timeline.rs         section timing, edit decisions, and shared camera state
  timeline/cameras.rs lens and tunnel camera rigs and safety constraints
  renderer.rs         per-frame uniforms and render-pass ordering
  fractal.rs          fractal paths and particle-string corridors
  text.rs             font atlas and text geometry
  gpu.rs              wgpu device, surface, and swapchain setup
  shader.rs           WGSL source composition and shader validation

  passes/              Rust-side GPU pipeline and resource management
  shaders/             WGSL for particles, fluid, bloom, text, and post
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
3. Advance the fluid simulation when it is visible.
4. Raymarch the active 3D scene into HDR color and depth.
5. Draw particles, fluid sheets, and text.
6. Extract bloom.
7. Apply depth of field, transitions, tonemapping, and output to the window.

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
cargo test
cargo check --release
```

The tests parse and validate the fully composed WGSL scene shader, check that
CPU and GPU uniform array sizes match, and exercise camera/fractal constraints.

Useful tuning locations:

- `audio.rs`: onset, accent, and latency tuning.
- `timeline.rs`: section lengths and edit timing.
- `timeline/cameras.rs`: lens and tunnel camera behavior.
- `renderer.rs`: supersampling, focus response, and particle counts.
- `shaders/scenes/`: effect geometry, materials, and audio response.
- `shaders/post.wgsl` and `shaders/bloom.wgsl`: final image treatment.

## Assets

`assets/DejaVuSans-Bold.ttf` ships with its license. No music is included; the
module is supplied at runtime.
