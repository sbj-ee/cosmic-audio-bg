use smithay_client_toolkit::output::OutputInfo;
use smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput;
use smithay_client_toolkit::shell::wlr_layer::{LayerSurface, LayerSurfaceConfigure};
use smithay_client_toolkit::shm::slot::SlotPool;

/// Matches cosmic-bg: fractional scale uses 120 as the unit denominator.
pub const FRACTIONAL_SCALE_UNIT: u32 = 120;

pub struct BgLayer {
    pub surface: LayerSurface,
    pub viewport: smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport,
    pub wl_output: WlOutput,
    pub output_info: OutputInfo,
    pub pool: Option<SlotPool>,
    /// Logical size from the compositor configure event.
    pub size: Option<(u32, u32)>,
    pub fractional_scale: Option<u32>,
    pub needs_redraw: bool,
}

impl BgLayer {
    /// Physical pixel dimensions for the SHM buffer / GPU target.
    pub fn buffer_size(&self) -> Option<(u32, u32)> {
        let (w, h) = self.size?;
        let scale = self.fractional_scale?;
        Some((w * scale / FRACTIONAL_SCALE_UNIT, h * scale / FRACTIONAL_SCALE_UNIT))
    }
}

pub fn handle_configure(
    layer: &mut BgLayer,
    configure: &LayerSurfaceConfigure,
    shm: &smithay_client_toolkit::shm::Shm,
) {
    let (mut w, mut h) = configure.new_size;
    if w == 0 || h == 0 {
        if let Some((lw, lh)) = layer.output_info.logical_size {
            w = lw as u32;
            h = lh as u32;
        } else {
            return;
        }
    }

    layer.size = Some((w, h));
    layer.needs_redraw = true;

    layer
        .viewport
        .set_destination(w as i32, h as i32);

    ensure_pool(layer, shm);
}

pub fn ensure_pool(layer: &mut BgLayer, shm: &smithay_client_toolkit::shm::Shm) {
    let Some((w, h)) = layer.buffer_size().or(layer.size) else {
        return;
    };

    let Some(bytes) = (w as usize)
        .checked_mul(h as usize)
        .and_then(|s| s.checked_mul(4))
    else {
        tracing::error!(w, h, "buffer size overflow");
        return;
    };

    if let Some(pool) = layer.pool.as_mut() {
        if pool.len() < bytes {
            if let Err(err) = pool.resize(bytes) {
                tracing::error!(?err, "failed to resize slot pool");
                layer.pool = None;
            }
        }
    }

    if layer.pool.is_none() {
        match SlotPool::new(bytes, shm) {
            Ok(pool) => layer.pool = Some(pool),
            Err(err) => tracing::error!(?err, "failed to create slot pool"),
        }
    }
}
