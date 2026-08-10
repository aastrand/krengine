pub mod particles;
pub mod post;
pub mod scene;

/// HDR target the scene and particles accumulate into.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Linear distance-along-ray written by the scene pass, read by particles.
pub const DIST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
