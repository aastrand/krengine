/// WGSL has no `#include`, so we prepend a shared prelude to every module.
/// If this grows beyond one prelude, swap it for `naga_oil`'s `#import` system.
const COMMON: &str = include_str!("shaders/common.wgsl");

pub fn module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    let src = format!("{COMMON}\n{source}");
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}
