pub mod bloom;
pub mod particles;
pub mod post;
pub mod scene;
pub mod text;

/// HDR target the scene and particles accumulate into.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Shared depth buffer. The raymarch writes it explicitly from the hit
/// distance, so rasterized geometry sorts against the SDF correctly.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
