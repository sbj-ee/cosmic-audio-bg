use image::DynamicImage;
use image::GenericImageView;
use smithay_client_toolkit::reexports::client::{protocol::wl_shm, QueueHandle};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer, CreateBufferError, SlotPool};

use crate::layer::BgLayer;

pub fn canvas(
    pool: &mut SlotPool,
    image: &DynamicImage,
    width: i32,
    height: i32,
    stride: i32,
) -> Result<Buffer, CreateBufferError> {
    let (buffer, canvas) = pool.create_buffer(width, height, stride, wl_shm::Format::Xrgb8888)?;

    for (pos, (_, _, pixel)) in image.pixels().enumerate() {
        let index = pos * 4;
        let [r, g, b, _] = pixel.0;
        let packed = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        canvas[index..index + 4].copy_from_slice(&packed.to_le_bytes());
    }

    Ok(buffer)
}

pub fn present_layer(
    layer: &mut BgLayer,
    qh: &QueueHandle<crate::AppState>,
    buffer: &Buffer,
    buffer_width: i32,
    buffer_height: i32,
) {
    let Some((logical_w, logical_h)) = layer.size else {
        return;
    };

    let surface = layer.surface.wl_surface();

    surface.damage_buffer(0, 0, buffer_width, buffer_height);
    surface.frame(qh, surface.clone());

    if let Err(err) = buffer.attach_to(surface) {
        tracing::error!(?err, "failed to attach buffer");
    }

    // Match cosmic-bg + SCTK viewporter: scale the physical buffer to logical size.
    layer.viewport.set_source(
        0.0,
        0.0,
        buffer_width as f64,
        buffer_height as f64,
    );
    layer
        .viewport
        .set_destination(logical_w as i32, logical_h as i32);

    surface.commit();
}
