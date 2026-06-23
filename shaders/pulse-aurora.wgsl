// Phase 1 prototype shader — simulates audio with time-based pulses.
// Phase 2: bass/mid/high/energy/idle_blend uniforms from PipeWire FFT.

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

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let c = v * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - abs(hp - 2.0 * floor(hp / 2.0) - 1.0));
    let m = v - c;
    var rgb = vec3<f32>(0.0);
    if hp < 1.0 {
        rgb = vec3<f32>(c, x, 0.0);
    } else if hp < 2.0 {
        rgb = vec3<f32>(x, c, 0.0);
    } else if hp < 3.0 {
        rgb = vec3<f32>(0.0, c, x);
    } else if hp < 4.0 {
        rgb = vec3<f32>(0.0, x, c);
    } else if hp < 5.0 {
        rgb = vec3<f32>(x, 0.0, c);
    } else {
        rgb = vec3<f32>(c, 0.0, x);
    }
    return rgb + vec3<f32>(m);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    var uv = frag_coord.xy / u.resolution;
    let t = u.time;

    // Simulated audio when idle_blend is high (Phase 1 stand-in)
    let sim_bass = (sin(t * 2.1) * 0.5 + 0.5) * 0.35;
    let sim_mid = (sin(t * 3.7 + 1.2) * 0.5 + 0.5) * 0.25;
    let sim_high = (sin(t * 5.3 + 2.4) * 0.5 + 0.5) * 0.2;
    let sim_energy = (sim_bass + sim_mid + sim_high) / 3.0;

    let bass = mix(u.bass, sim_bass, u.idle_blend);
    let mid = mix(u.mid, sim_mid, u.idle_blend);
    let high = mix(u.high, sim_high, u.idle_blend);
    let energy = mix(u.energy, sim_energy, u.idle_blend);

    // Bass warps the field
    let warp = bass * 0.08;
    uv.x += sin(uv.y * 12.0 + t * 1.4) * warp;
    uv.y += cos(uv.x * 10.0 + t * 1.1) * warp;

    var value = 0.0;
    value += sin(uv.x * 10.0 + t + bass * 6.0);
    value += sin(uv.y * 9.0 + t * 0.8 + mid * 5.0);
    value += sin((uv.x + uv.y) * 8.0 + t * 0.6);
    let cx = uv.x - 0.5;
    let cy = uv.y - 0.5;
    value += sin(length(vec2<f32>(cx, cy)) * 18.0 - t * 2.0 - bass * 8.0);
    value = value / 4.0 + 0.5;

    let hue = 200.0 + mid * 80.0 + high * 40.0 + sin(t * 0.2) * 15.0;
    let sat = 0.55 + energy * 0.35;
    let val = 0.22 + value * 0.35 + bass * 0.25;
    var color = hsv_to_rgb(hue, sat, val);

    // Subtle aurora band
    let band = smoothstep(0.35, 0.65, uv.y + sin(uv.x * 6.0 + t) * 0.08);
    color += vec3<f32>(0.05, 0.12, 0.2) * band * (0.4 + energy);

    return vec4<f32>(color, 1.0);
}
