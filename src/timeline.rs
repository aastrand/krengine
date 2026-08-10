//! When things happen.
//!
//! The demo's structure lives here rather than being scattered through the
//! shaders: a section list keyed on the audio clock, producing the handful of
//! values the renderer needs this frame. Sections and camera direction will
//! extend this same table.

use crate::audio::Sync;

/// Cards shown before the scene appears, as (start, end) in seconds.
const CARDS: [(f32, f32); 3] = [(0.4, 2.9), (3.2, 5.5), (5.8, 8.1)];
/// Apparent size of each card. The name sits closest to the camera; the rest
/// fall back behind it.
const CARD_SCALES: [f32; 3] = [1.9, 1.0, 1.25];
/// How long a card takes to fade in and out.
const CARD_FADE: f32 = 0.35;
/// When the scene begins to appear, and how long it takes.
const SCENE_START: f32 = 7.7;
const SCENE_FADE: f32 = 2.4;
/// How far a card travels while it is on screen, in cap heights.
const SCROLL_RANGE: f32 = 1.5;

/// What the timeline says about this frame.
#[derive(Clone, Copy, Default)]
pub struct Stage {
    /// Which card is showing, or -1 for none.
    pub card: i32,
    /// Card opacity, including its beat pulse.
    pub card_alpha: f32,
    /// How far the scene has faded up, 0 to 1.
    pub scene: f32,
    /// Horizontal drift of the card, in cap heights. Positive is right.
    pub scroll: f32,
    /// How large the card reads, which is how near it feels.
    pub scale: f32,
}

impl Stage {
    pub fn at(music: &Sync) -> Self {
        let t = music.time;

        let mut card = -1;
        let mut card_alpha = 0.0;
        let mut scroll = 0.0;
        let mut scale = 1.0;
        for (index, (start, end)) in CARDS.iter().enumerate() {
            if t >= *start && t < *end {
                card = index as i32;
                // Fade in, hold, fade out.
                let in_edge = smoothstep(*start, start + CARD_FADE, t);
                let out_edge = 1.0 - smoothstep(end - CARD_FADE, *end, t);
                card_alpha = in_edge * out_edge;

                // Drifts right to left across its life, slowly.
                let progress = (t - start) / (end - start);
                scroll = SCROLL_RANGE * (0.5 - progress);

                // Drifts very slightly toward the camera while it is up.
                scale = CARD_SCALES[index] * (1.0 + progress * 0.06);
            }
        }

        // Cards breathe on the beat rather than sitting flat.
        card_alpha *= 0.82 + music.beat * 0.35;

        Self {
            card,
            card_alpha,
            scene: smoothstep(SCENE_START, SCENE_START + SCENE_FADE, t),
            scroll,
            scale,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
