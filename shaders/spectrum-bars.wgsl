// Optional spectrum visualization overlay mode.

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
    let t = u.time;

    // Dark gradient background
    var color = mix(vec3<f32>(0.02, 0.03, 0.08), vec3<f32>(0.05, 0.07, 0.14), uv.y);

    // Three band bars at bottom
    let bar_h = 0.18;
    if uv.y < bar_h {
        let x = uv.x;
        var level = 0.0;
        var tint = vec3<f32>(0.2, 0.5, 1.0);
        if x < 0.33 {
            level = u.bass;
            tint = vec3<f32>(0.9, 0.25, 0.35);
        } else if x < 0.66 {
            level = u.mid;
            tint = vec3<f32>(0.3, 0.85, 0.55);
        } else {
            level = u.high;
            tint = vec3<f32>(0.35, 0.55, 1.0);
        }

        let fill = level * bar_h * 0.9;
        if uv.y < fill {
            color = mix(color, tint, 0.85);
        }
    }

    // Soft aurora above bars driven by energy
    let aurora = sin(uv.x * 8.0 + t * 0.5) * u.energy * 0.15;
    color += vec3<f32>(0.08, 0.15, 0.3) * aurora * (1.0 - uv.y);

    return vec4<f32>(color, 1.0);
}
