mod audio;
mod gpu;
mod passes;
mod renderer;
mod shader;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use audio::{Music, Sync};
use gpu::Gpu;
use renderer::{Frame, Renderer};

struct State {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    /// Fallback clock, used only when there's no music to run off.
    start: Instant,
    last_frame: Instant,
    frame_times: Vec<f32>,
    prev_music_time: f32,
    debug: bool,
}

#[derive(Default)]
struct App {
    state: Option<State>,
    music: Option<Music>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("krengine")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let gpu = pollster::block_on(Gpu::new(window.clone())).expect("gpu init");
        let renderer = Renderer::new(&gpu);

        let now = Instant::now();
        self.state = Some(State {
            window,
            gpu,
            renderer,
            start: now,
            last_frame: now,
            frame_times: Vec::with_capacity(256),
            prev_music_time: 0.0,
            debug: std::env::var("KR_DEBUG").is_ok(),
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        PhysicalKey::Code(KeyCode::KeyB) => {
                            state.renderer.show_bands = !state.renderer.show_bands;
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::Resized(size) => {
                state.gpu.resize(size.width, size.height);
                state.renderer.resize(&state.gpu);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - state.last_frame).as_secs_f32();
                state.last_frame = now;

                // The audio clock is the master clock — wall time drifts out of
                // sync over the length of a tune.
                let music = match self.music.as_mut() {
                    Some(music) => music.sample(dt),
                    None => Sync {
                        time: (now - state.start).as_secs_f32(),
                        dt,
                        ..Default::default()
                    },
                };

                // How evenly does the demo clock advance compared to real time?
                // 1.0 means perfectly smooth; spread here is visible judder.
                // Enable with KR_DEBUG=1.
                if state.debug && dt > 0.0 {
                    state.frame_times.push(dt * 1000.0);
                }
                state.prev_music_time = music.time;

                if state.debug && state.frame_times.len() >= 240 {
                    let mut sorted = std::mem::take(&mut state.frame_times);
                    sorted.sort_by(f32::total_cmp);
                    let at = |q: f32| sorted[(sorted.len() as f32 * q) as usize % sorted.len()];
                    log::info!(
                        "frame ms: p50 {:.1}  p95 {:.1}  max {:.1}  ({:.0} fps, audio latency {:.0} ms)",
                        at(0.5), at(0.95), at(0.999), 1000.0 / at(0.5),
                        music.output_latency * 1000.0
                    );
                }
                if let Frame::Reconfigure = state.renderer.render(&state.gpu, &music) {
                    state.gpu.reconfigure();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    // krengine [path/to/module.xm] — runs silent if no tune is given.
    let track = std::env::args().nth(1).map(PathBuf::from);
    let music = match track {
        Some(path) => match Music::start(&path) {
            Ok(music) => Some(music),
            Err(e) => {
                log::error!("no music: {e:#}");
                None
            }
        },
        None => None,
    };

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App {
        music,
        ..Default::default()
    })?;
    Ok(())
}
