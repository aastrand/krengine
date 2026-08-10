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

/// Every WGSL module, so tests can compile them without a GPU. The passes
/// include their own sources at the point of use, so this is a second list —
/// a shader missing from it is simply not covered by the test below.
#[cfg(test)]
const MODULES: [(&str, &str); 8] = [
    ("scene", include_str!("shaders/scene.wgsl")),
    ("particles", include_str!("shaders/particles.wgsl")),
    ("fluid", include_str!("shaders/fluid.wgsl")),
    ("fluid_view", include_str!("shaders/fluid_view.wgsl")),
    ("splat", include_str!("shaders/splat.wgsl")),
    ("bloom", include_str!("shaders/bloom.wgsl")),
    ("text", include_str!("shaders/text.wgsl")),
    ("post", include_str!("shaders/post.wgsl")),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaders are compiled at startup, so a typo in one is otherwise only
    /// found by running the demo and watching it panic. This is the same
    /// front end wgpu uses, on the same concatenated source.
    #[test]
    fn every_shader_compiles() {
        for (label, source) in MODULES {
            let src = format!("{COMMON}\n{source}");
            let module = naga::front::wgsl::parse_str(&src)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&src)));

            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&src)));
        }
    }
}
