//! Text as textured quads, rasterised from a real typeface at startup.
//!
//! The atlas is built once into a single-channel texture, and each glyph
//! becomes an instance: position, size, and where to find it in the atlas.

use std::collections::HashMap;

use fontdue::{Font, FontSettings};

/// Height the glyphs are rasterised at. Text is scaled from here, so this only
/// sets how much detail the atlas holds.
const RASTER_SIZE: f32 = 96.0;
/// Space around each glyph in the atlas, so filtering can't bleed neighbours in.
const PADDING: u32 = 2;

pub struct Glyph {
    /// Where the glyph sits in the atlas, in texels.
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// Size and bearing in raster pixels.
    pub size: [f32; 2],
    pub bearing: [f32; 2],
    pub advance: f32,
}

/// One glyph of a laid-out string, ready to draw.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphInstance {
    /// Position and size, in units where 1.0 is the text's cap height.
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// Index within the string, so shaders can vary an effect per letter.
    pub index: f32,
    pub _pad: [f32; 3],
}

pub struct FontAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    glyphs: HashMap<char, Glyph>,
}

impl FontAtlas {
    /// Rasterise every character the demo needs into one texture.
    pub fn new(font_bytes: &[u8], characters: &str) -> anyhow::Result<Self> {
        let font = Font::from_bytes(font_bytes, FontSettings::default())
            .map_err(|e| anyhow::anyhow!("loading font: {e}"))?;

        // Rasterise first so the atlas can be sized to what's actually needed.
        let rendered: Vec<(char, fontdue::Metrics, Vec<u8>)> = characters
            .chars()
            .map(|c| {
                let (metrics, bitmap) = font.rasterize(c, RASTER_SIZE);
                (c, metrics, bitmap)
            })
            .collect();

        // A single row is simplest and, at this glyph count, not wasteful.
        let width: u32 = rendered
            .iter()
            .map(|(_, m, _)| m.width as u32 + PADDING * 2)
            .sum();
        let height: u32 = rendered
            .iter()
            .map(|(_, m, _)| m.height as u32 + PADDING * 2)
            .max()
            .unwrap_or(1);

        let mut pixels = vec![0u8; (width * height) as usize];
        let mut glyphs = HashMap::new();
        let mut pen = 0u32;

        for (character, metrics, bitmap) in &rendered {
            let x = pen + PADDING;
            let y = PADDING;

            for row in 0..metrics.height {
                for column in 0..metrics.width {
                    let target = (y as usize + row) * width as usize + x as usize + column;
                    pixels[target] = bitmap[row * metrics.width + column];
                }
            }

            glyphs.insert(
                *character,
                Glyph {
                    uv_min: [x as f32 / width as f32, y as f32 / height as f32],
                    uv_max: [
                        (x + metrics.width as u32) as f32 / width as f32,
                        (y + metrics.height as u32) as f32 / height as f32,
                    ],
                    size: [metrics.width as f32, metrics.height as f32],
                    bearing: [metrics.xmin as f32, metrics.ymin as f32],
                    advance: metrics.advance_width,
                },
            );

            pen += metrics.width as u32 + PADDING * 2;
        }

        Ok(Self {
            width,
            height,
            pixels,
            glyphs,
        })
    }

    /// Lay a string out on a baseline at the origin, centred horizontally.
    /// Units are fractions of the raster size, so the caller scales freely.
    pub fn layout(&self, text: &str) -> Vec<GlyphInstance> {
        let mut instances = Vec::new();
        let mut pen = 0.0;

        for (index, character) in text.chars().enumerate() {
            if let Some(glyph) = self.glyphs.get(&character) {
                if glyph.size[0] > 0.0 {
                    instances.push(GlyphInstance {
                        rect: [
                            (pen + glyph.bearing[0]) / RASTER_SIZE,
                            glyph.bearing[1] / RASTER_SIZE,
                            glyph.size[0] / RASTER_SIZE,
                            glyph.size[1] / RASTER_SIZE,
                        ],
                        uv: [
                            glyph.uv_min[0],
                            glyph.uv_min[1],
                            glyph.uv_max[0],
                            glyph.uv_max[1],
                        ],
                        index: index as f32,
                        _pad: [0.0; 3],
                    });
                }
                pen += glyph.advance;
            }
        }

        // Centre on the origin.
        let width = pen / RASTER_SIZE;
        for instance in &mut instances {
            instance.rect[0] -= width * 0.5;
        }
        instances
    }
}
