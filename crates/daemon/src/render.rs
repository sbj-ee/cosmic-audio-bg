//! wgpu offscreen shader renderer for wallpaper frames.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use cosmic_audio_bg_audio::AudioLevels;
use image::{DynamicImage, ImageBuffer, Rgba};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub energy: f32,
    /// Per-band blended level (audio + idle sim), 16 bands packed as 4 vec4s.
    pub levels: [[f32; 4]; 4],
    /// Per-band continuously-integrated wave phase (radians, wrapped to TAU),
    /// 16 bands packed as 4 vec4s. Integrating the phase on the CPU keeps the
    /// animation seamless: changing a band's speed no longer teleports the wave.
    pub phases: [[f32; 4]; 4],
    /// Visualization mode flag: 0 = stripes (one wave per band), 1 = composite
    /// (single superposition wave). Padded to keep 16-byte std140 alignment.
    pub mode: u32,
    pub _pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<Uniforms>() == 160);

const BAND_COUNT: usize = cosmic_audio_bg_audio::SPECTRUM_BANDS;

pub struct ShaderRenderer {
    shader_path: PathBuf,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    output_texture: wgpu::Texture,
    output_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    start_time: Instant,
    last_elapsed: f64,
    /// Continuously-integrated wave phase per band (radians).
    phases: [f64; BAND_COUNT],
    /// Continuously-integrated idle-simulation phase per band (radians).
    idle_phases: [f64; BAND_COUNT],
    /// Visualization mode flag forwarded to the shader (0 stripes, 1 composite).
    mode: u32,
}

impl ShaderRenderer {
    pub fn new(shader_path: &Path, width: u32, height: u32, mode: u32) -> Result<Self> {
        let shader_source = fs::read_to_string(shader_path)
            .with_context(|| format!("failed to read shader {}", shader_path.display()))?;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .context("no suitable GPU adapter found")?;

        tracing::info!(
            adapter = ?adapter.get_info().name,
            backend = ?adapter.get_info().backend,
            "GPU adapter selected"
        );

        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_texture_dimension_2d = 8192;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("cosmic-audio-bg"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wallpaper shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("output texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let bytes_per_row = aligned_bytes_per_row(width);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output buffer"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            shader_path: shader_path.to_path_buf(),
            device,
            queue,
            pipeline,
            uniform_buffer,
            bind_group,
            output_texture,
            output_buffer,
            width,
            height,
            start_time: Instant::now(),
            last_elapsed: 0.0,
            phases: [0.0; BAND_COUNT],
            idle_phases: [0.0; BAND_COUNT],
            mode,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.width == width && self.height == height {
            return Ok(());
        }
        *self = Self::new(&self.shader_path, width, height, self.mode)?;
        Ok(())
    }

    pub fn render_frame(&mut self, audio: AudioLevels, idle_blend: f32) -> Result<DynamicImage> {
        use std::f64::consts::TAU;

        // Advance phases by the real elapsed delta. Clamp dt so a stall (e.g.
        // the daemon being paused) can't produce a huge one-frame phase jump.
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let dt = (elapsed - self.last_elapsed).clamp(0.0, 0.1);
        self.last_elapsed = elapsed;

        let idle_blend_f = idle_blend as f64;
        let mut levels = [0.0f32; BAND_COUNT];
        let mut phases = [0.0f32; BAND_COUNT];

        for i in 0..BAND_COUNT {
            let fi = i as f64;
            // norm_hz = log(center_hz/40)/log(400) reduces exactly to this.
            let norm_hz = (fi + 0.5) / BAND_COUNT as f64;

            // Idle simulation as a continuously-integrated sine (bounded phase
            // => seamless, no f32 precision drift over long runtimes).
            let idle_omega = 0.6 + fi * 0.09;
            self.idle_phases[i] = (self.idle_phases[i] + dt * idle_omega).rem_euclid(TAU);
            let sim = ((self.idle_phases[i] + fi * 1.1).sin() * 0.5 + 0.5) * 0.12;

            // Blend live spectrum with the idle sim exactly as before.
            let spectrum = audio.bands[i] as f64;
            let level = spectrum + (sim - spectrum) * idle_blend_f;

            // Integrate the wave's temporal phase. The angular speed depends on
            // the (changing) level, but because we accumulate the phase instead
            // of computing `t * speed`, a speed change only changes how fast the
            // wave moves from its current position — it never teleports.
            let omega = 0.15 + norm_hz * 1.2 + level * 3.0;
            self.phases[i] = (self.phases[i] + dt * omega).rem_euclid(TAU);

            levels[i] = level as f32;
            phases[i] = self.phases[i] as f32;
        }

        let pack = |a: &[f32; BAND_COUNT], o: usize| [a[o], a[o + 1], a[o + 2], a[o + 3]];
        let uniforms = Uniforms {
            resolution: [self.width as f32, self.height as f32],
            time: elapsed.rem_euclid(TAU) as f32,
            energy: audio.energy,
            levels: [
                pack(&levels, 0),
                pack(&levels, 4),
                pack(&levels, 8),
                pack(&levels, 12),
            ],
            phases: [
                pack(&phases, 0),
                pack(&phases, 4),
                pack(&phases, 8),
                pack(&phases, 12),
            ],
            mode: self.mode,
            _pad: [0; 3],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let view = self
            .output_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let bytes_per_row = aligned_bytes_per_row(self.width);
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = self.output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .context("buffer map channel closed")?
            .context("buffer map failed")?;

        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in 0..self.height {
            let start = (row * bytes_per_row) as usize;
            let end = start + (self.width * 4) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        self.output_buffer.unmap();

        let img = ImageBuffer::<Rgba<u8>, _>::from_raw(self.width, self.height, pixels)
            .context("failed to build image buffer")?;
        Ok(DynamicImage::ImageRgba8(img))
    }
}

fn aligned_bytes_per_row(width: u32) -> u32 {
    let unaligned = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (unaligned + align - 1) / align * align
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::path::Path;

    #[test]
    fn rendered_frame_covers_full_buffer() {
        let shader = Path::new("shaders/sinusoids.wgsl");
        if !shader.exists() {
            return;
        }

        let mut renderer = ShaderRenderer::new(shader, 1920, 1200, 0).expect("renderer");
        let img = renderer
            .render_frame(AudioLevels::default(), 0.0)
            .expect("frame");

        let corners = [
            (0u32, 0u32, "top-left"),
            (1919, 0, "top-right"),
            (0, 1199, "bottom-left"),
            (1919, 1199, "bottom-right"),
        ];
        let center = img.get_pixel(960, 600);

        for (x, y, name) in corners {
            let px = img.get_pixel(x, y);
            assert!(
                px[0] > 0 || px[1] > 0 || px[2] > 0,
                "{name} still clear black: {px:?}"
            );
        }
        assert!(
            center[0] > 0 || center[1] > 0 || center[2] > 0,
            "center still clear black"
        );

        let black = img
            .pixels()
            .filter(|(_, _, px)| px[0] == 0 && px[1] == 0 && px[2] == 0)
            .count();
        let total = (img.width() * img.height()) as usize;
        assert!(
            black * 100 / total.max(1) < 5,
            "too many untouched pixels ({black}/{total}) — vertex coverage likely wrong"
        );
    }
}
