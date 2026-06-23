// Phase 1 prototype shader for cosmic-ext-bg (time + resolution uniforms only).
// Simulates audio pulses until the custom daemon provides real FFT bands.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(pos[vertex_index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    var uv = frag_coord.xy / u.resolution;
    let t = u.time;

    let bass = sin(t * 2.1) * 0.5 + 0.5;
    let mid = sin(t * 3.7 + 1.2) * 0.5 + 0.5;

    uv.x += sin(uv.y * 12.0 + t * 1.4) * bass * 0.06;
    uv.y += cos(uv.x * 10.0 + t * 1.1) * mid * 0.05;

    var value = sin(uv.x * 10.0 + t) + sin(uv.y * 9.0 + t * 0.8);
    value = value * 0.25 + 0.5;

    let r = sin(value * 6.28 + t * 0.3) * 0.5 + 0.5;
    let g = sin(value * 6.28 + 2.1 + t * 0.2) * 0.5 + 0.5;
    let b = sin(value * 6.28 + 4.2 + t * 0.25) * 0.5 + 0.5;

    return vec4<f32>(r * 0.6, g * 0.7, b * 0.9, 1.0);
}
