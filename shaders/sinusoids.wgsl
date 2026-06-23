// Audio-reactive green sinusoids, full screen.
//
// Two modes (selected by u.mode):
//   0 = stripes:    sixteen sinusoids, one log-spaced FFT band per horizontal
//                   stripe.
//   1 = composite:  a single wave that is the superposition (sum) of all
//                   sixteen band sinusoids, drawn as one glowing green line.

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    energy: f32,
    // Per-band blended level (audio + idle sim), 16 bands packed as 4 vec4s.
    levels: array<vec4<f32>, 4>,
    // Per-band continuously-integrated wave phase (radians), packed as 4 vec4s.
    // The phase is integrated on the CPU so changing a band's speed never makes
    // the wave jump — the animation stays smooth and seam-free.
    phases: array<vec4<f32>, 4>,
    // 0 = stripes (one wave per band), 1 = composite (single superposition wave).
    mode: u32,
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

const BAND_COUNT: u32 = 16u;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    // Oversized triangle that covers the full clip-space square.
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

fn vec4_component(v: vec4<f32>, j: u32) -> f32 {
    if j == 0u { return v.x; }
    if j == 1u { return v.y; }
    if j == 2u { return v.z; }
    return v.w;
}

fn band_level(i: u32) -> f32 {
    return vec4_component(u.levels[i / 4u], i % 4u);
}

fn band_phase(i: u32) -> f32 {
    return vec4_component(u.phases[i / 4u], i % 4u);
}

fn wave_glow(dist: f32, thickness: f32) -> f32 {
    return exp(-dist * dist / (thickness * thickness));
}

// Spatial frequency for a band, matching the stripe layout so the composite
// wave is built from the exact same per-band sinusoids.
fn band_spatial_freq(i: u32, level: f32) -> f32 {
    let fi = f32(i);
    let norm_hz = (fi + 0.5) / f32(BAND_COUNT);
    return 0.8 + norm_hz * 5.0 + level * 3.0;
}

// Composite mode: render ONE wave that is the superposition (sum) of all the
// per-band sinusoids, using their continuous integrated phases.
fn render_composite(uv: vec2<f32>, x: f32, bg: vec3<f32>) -> vec3<f32> {
    var color = bg;

    // y_offset(x) = sum_b level[b] * sin(spatial_freq[b]*x + phase[b]).
    // Dividing by the total level keeps the result a weighted average of sines
    // (bounded in [-1, 1]) so the wave always stays on-screen regardless of how
    // many bands are loud at once.
    var wave = 0.0;
    var amp_sum = 0.0;
    for (var i = 0u; i < BAND_COUNT; i++) {
        let fi = f32(i);
        let level = band_level(i);
        let spatial_freq = band_spatial_freq(i, level);
        let phase = band_phase(i) + fi * 0.31;
        wave += level * sin(x * spatial_freq + phase);
        amp_sum += level;
    }
    let norm = max(amp_sum, 0.6);
    let w = clamp(wave / norm, -1.0, 1.0);

    // Single wave centered on screen; amplitude swells with overall energy.
    let amplitude = 0.20 + u.energy * 0.18;
    let y_center = 0.5 + w * amplitude;
    let dist = uv.y - y_center;

    let thickness = 0.012 + u.energy * 0.012;
    let glow = wave_glow(dist, thickness);
    // A tight bright core gives the line a crisp center inside the soft glow.
    let core = wave_glow(dist, thickness * 0.4);

    // Stay within the green palette (hue 88–140°); brighter where audio is hot.
    let hue = 104.0 + w * 16.0;
    let sat = 0.55 + u.energy * 0.25;
    let val = 0.18 + glow * (0.5 + u.energy * 0.5) + core * 0.6;
    let wave_color = hsv_to_rgb(hue, sat, clamp(val, 0.0, 1.0));

    color = mix(color, wave_color, clamp(glow + core, 0.0, 1.0));
    return color;
}

fn render_stripes(uv: vec2<f32>, x: f32, bg: vec3<f32>) -> vec3<f32> {
    var color = bg;

    let band_h = 1.0 / f32(BAND_COUNT);
    let thickness = band_h * (0.22 + u.energy * 0.08);

    for (var i = 0u; i < BAND_COUNT; i++) {
        let fi = f32(i);
        let level = band_level(i);

        // Each FFT band owns a horizontal stripe across the full screen.
        let band_center = (fi + 0.5) * band_h;
        let y_in_band = uv.y - band_center;

        // norm_hz = log(center_hz/40)/log(400) reduces exactly to this.
        let norm_hz = (fi + 0.5) / f32(BAND_COUNT);
        // Louder bands oscillate faster (more cycles); the wave's travel speed
        // is baked into the CPU-integrated phase so changing speed is seamless.
        let spatial_freq = 0.8 + norm_hz * 5.0 + level * 3.0;
        let phase = band_phase(i) + fi * 0.31;

        // Quiet bands sit nearly flat; loud bands swing across most of the
        // stripe, giving each sinusoid a clear, audio-driven amplitude.
        let amp = band_h * (0.04 + level * 0.95);
        let curve = amp * sin(x * spatial_freq + phase);
        let glow = wave_glow(y_in_band - curve, thickness);

        // Varying green hues: yellow-green (low bands) to blue-green (high bands).
        let hue = 88.0 + norm_hz * 52.0;
        let sat = 0.5 + norm_hz * 0.15 + level * 0.35;
        let val = 0.18 + glow * (0.45 + level * 0.55);
        let wave_color = hsv_to_rgb(hue, sat, val);

        // Triangular stripe window of half-width `band_h`. Adjacent windows form
        // a partition of unity (they sum to 1 everywhere), so neighbouring band
        // tints cross-fade smoothly and leave no seam line between stripes.
        let stripe = max(0.0, 1.0 - abs(uv.y - band_center) / band_h);
        let stripe_tint = hsv_to_rgb(hue, 0.22 + norm_hz * 0.1 + level * 0.2, 0.06 + level * 0.12);

        color = mix(color, stripe_tint, stripe * 0.55);
        color = mix(color, wave_color, glow * (0.5 + level * 0.45));
    }

    return color;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy / u.resolution;
    let aspect = u.resolution.x / u.resolution.y;
    let x = (uv.x - 0.5) * aspect * 6.28318;

    // Full-screen background gradient (dark yellow-green at bottom, deep teal at top).
    let bg_hue = 105.0 + uv.y * 35.0;
    let bg = hsv_to_rgb(bg_hue, 0.4, 0.07 + uv.y * 0.04);

    var color: vec3<f32>;
    if u.mode == 1u {
        color = render_composite(uv, x, bg);
    } else {
        color = render_stripes(uv, x, bg);
    }

    return vec4<f32>(color, 1.0);
}
