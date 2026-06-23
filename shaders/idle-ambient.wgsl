// Idle ambient fallback — slow motion when audio energy is low.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    idle_blend: f32,
    bass: f32,
    mid: f32,
    high: f32,
    energy: f32,
    spectrum0: vec4<f32>,
    spectrum1: vec4<f32>,
    spectrum2: vec4<f32>,
    spectrum3: vec4<f32>,
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
    let uv = frag_coord.xy / u.resolution;
    let t = u.time * 0.15;

    let wave = sin(uv.x * 4.0 + t) * 0.5 + sin(uv.y * 3.0 - t * 0.7) * 0.5;
    let depth = 0.12 + wave * 0.06;

    let top = vec3<f32>(0.04, 0.08, 0.16);
    let bottom = vec3<f32>(0.01, 0.02, 0.05);
    var color = mix(bottom, top, uv.y + wave * 0.05);

    // Blend toward aurora shader when audio returns
    let glow = smoothstep(0.2, 0.8, u.energy) * (1.0 - u.idle_blend);
    color += vec3<f32>(0.02, 0.06, 0.12) * glow;

    return vec4<f32>(color, 1.0);
}
