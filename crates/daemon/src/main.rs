mod draw;
mod layer;
mod menu;
mod power;
mod render;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser;
use cosmic_audio_bg_audio::AudioAnalyzer;
use cosmic_audio_bg_config::{Config, OutputMode};
use layer::BgLayer;
use menu::Menu;
use render::ShaderRenderer;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::delegate_compositor;
use smithay_client_toolkit::delegate_keyboard;
use smithay_client_toolkit::delegate_layer;
use smithay_client_toolkit::delegate_output;
use smithay_client_toolkit::delegate_pointer;
use smithay_client_toolkit::delegate_registry;
use smithay_client_toolkit::delegate_seat;
use smithay_client_toolkit::delegate_shm;
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::LoopSignal;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::protocol::wl_keyboard::WlKeyboard;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{self, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{delegate_noop, Connection, Dispatch, Proxy, QueueHandle, Weak};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{Shm, ShmHandler};

#[derive(Parser, Debug)]
#[command(name = "cosmic-audio-bg", about = "Audio-reactive COSMIC desktop background")]
struct Args {
    #[arg(short, long, default_value = "config/default.ron")]
    config: PathBuf,

    #[arg(long)]
    shader: Option<PathBuf>,
}

pub struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm_state: Shm,
    layer_state: LayerShell,
    seat_state: SeatState,
    viewporter: wp_viewporter::WpViewporter,
    fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    qh: QueueHandle<AppState>,
    config: Config,
    /// Path the config was loaded from; selections are persisted back here.
    config_path: PathBuf,
    layers: Vec<BgLayer>,
    renderer: Option<ShaderRenderer>,
    audio: AudioAnalyzer,
    _audio_handle: std::thread::JoinHandle<()>,
    loop_signal: LoopSignal,
    exit: bool,
    idle_since: Option<Instant>,
    idle_blend: f32,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    /// Latest keyboard modifier state (for detecting Super/logo held).
    modifiers: Modifiers,
    menu: Menu,
}

impl AppState {
    fn fps(&self) -> u32 {
        let on_battery = power::on_battery();
        self.config.effective_fps(on_battery)
    }

    fn should_pause(&self) -> bool {
        self.config.power.pause_on_lid_closed && power::lid_closed()
    }

    fn update_idle_blend(&mut self) {
        let levels = self.audio.levels();
        if levels.energy < self.config.idle_energy_threshold {
            if self.idle_since.is_none() {
                self.idle_since = Some(Instant::now());
            }
            if let Some(since) = self.idle_since {
                let elapsed = since.elapsed().as_secs_f32();
                let target = (elapsed / self.config.idle_seconds).clamp(0.0, 1.0);
                self.idle_blend = self.idle_blend + (target - self.idle_blend) * 0.05;
            }
        } else {
            self.idle_since = None;
            self.idle_blend = self.idle_blend + (0.0 - self.idle_blend) * 0.15;
        }
    }

    fn redraw_all(&mut self) {
        if self.should_pause() {
            return;
        }

        self.update_idle_blend();
        let audio = self.audio.levels();
        let idle_blend = self.idle_blend;

        for layer in &mut self.layers {
            let Some((width, height)) = layer.buffer_size() else {
                continue;
            };
            if layer.fractional_scale.is_none() {
                continue;
            }
            if width == 0 || height == 0 {
                continue;
            }

            layer::ensure_pool(layer, &self.shm_state);
            let Some(pool) = layer.pool.as_mut() else {
                continue;
            };

            if self.renderer.is_none() {
                let mode = self.config.visualization.shader_flag();
                match ShaderRenderer::new(&self.config.shader_path, width, height, mode) {
                    Ok(renderer) => self.renderer = Some(renderer),
                    Err(err) => {
                        tracing::error!(?err, "failed to create renderer");
                        return;
                    }
                }
            }

            if let Some(renderer) = self.renderer.as_mut() {
                if let Err(err) = renderer.resize(width, height) {
                    tracing::error!(?err, "renderer resize failed");
                    continue;
                }

                match renderer.render_frame(audio, idle_blend) {
                    Ok(mut image) => {
                        if self.menu.open
                            && self.menu.belongs_to(layer.surface.wl_surface())
                        {
                            if let Some(rgba) = image.as_mut_rgba8() {
                                let scale = layer
                                    .fractional_scale
                                    .map(|s| s as f32 / layer::FRACTIONAL_SCALE_UNIT as f32)
                                    .unwrap_or(1.0);
                                self.menu.draw(rgba, scale, self.config.visualization);
                            }
                        }
                        let stride = (width * 4) as i32;
                        match draw::canvas(pool, &image, width as i32, height as i32, stride) {
                            Ok(buffer) => {
                                draw::present_layer(
                                    layer,
                                    &self.qh,
                                    &buffer,
                                    width as i32,
                                    height as i32,
                                );
                            }
                            Err(err) => tracing::error!(?err, "canvas creation failed"),
                        }
                    }
                    Err(err) => tracing::error!(?err, "render failed"),
                }
            }

            layer.needs_redraw = false;
        }
    }

    fn create_layer(&mut self, output: WlOutput, output_info: OutputInfo) -> BgLayer {
        let surface = self.compositor_state.create_surface(&self.qh);

        let layer_surface = self.layer_state.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Background,
            Some("wallpaper".to_string()),
            Some(&output),
        );

        layer_surface.set_anchor(Anchor::all());
        layer_surface.set_exclusive_zone(-1);
        // Leave the input region at its default (the whole surface) so the
        // background can receive pointer button events for the right-click
        // menu. `OnDemand` keyboard interactivity lets the surface gain
        // keyboard focus when clicked, enabling Esc to dismiss the menu,
        // without permanently grabbing the keyboard.
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        surface.commit();

        let viewport = self.viewporter.get_viewport(&surface, &self.qh, ());

        let fractional_scale = if let Some(mngr) = self.fractional_scale_manager.as_ref() {
            mngr.get_fractional_scale(&surface, &self.qh, surface.downgrade());
            None
        } else {
            Some(
                output_info.scale_factor.max(1) as u32 * layer::FRACTIONAL_SCALE_UNIT,
            )
        };

        BgLayer {
            surface: layer_surface,
            viewport,
            wl_output: output,
            output_info,
            pool: None,
            size: None,
            fractional_scale,
            needs_redraw: false,
        }
    }

    fn output_allowed(&self, name: &str) -> bool {
        match &self.config.output {
            OutputMode::All => true,
            OutputMode::Named(expected) => expected == name,
        }
    }

    /// Find the layer whose surface matches the given Wayland surface.
    fn layer_for_surface(&self, surface: &WlSurface) -> Option<&BgLayer> {
        self.layers
            .iter()
            .find(|l| l.surface.wl_surface() == surface)
    }

    /// Apply a newly chosen visualization mode: update the in-memory config,
    /// force the renderer to be rebuilt with the new mode, and persist the
    /// choice to the user config file so it survives a restart.
    fn select_mode(&mut self, mode: cosmic_audio_bg_config::VisualizationMode) {
        if self.config.visualization != mode {
            self.config.visualization = mode;
            // The renderer bakes the mode in at creation; drop it so the next
            // frame rebuilds it with the new mode.
            self.renderer = None;
            if let Err(err) = self.persist_config() {
                tracing::warn!(?err, "failed to persist visualization mode");
            } else {
                tracing::info!(?mode, "visualization mode changed");
            }
        }
    }

    fn persist_config(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pretty = ron::ser::PrettyConfig::new();
        let text = ron::ser::to_string_pretty(&self.config, pretty)?;
        std::fs::write(&self.config_path, text)?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cosmic_audio_bg=info".into()),
        )
        .init();

    let args = Args::parse();
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| "laptop".into());

    let mut config = Config::load_with_machine_override(&args.config, &hostname)?;
    if let Some(shader) = args.shader {
        config.shader_path = shader;
    }

    if !config.shader_path.exists() {
        if let Ok(root) = std::env::var("COSMIC_AUDIO_BG_ROOT") {
            let root_shader = PathBuf::from(&root).join(&config.shader_path);
            if root_shader.exists() {
                config.shader_path = root_shader;
            }
        }
    }

    if !config.shader_path.exists() {
        // Try relative to project when running from source tree
        let dev_shader = PathBuf::from("shaders/sinusoids.wgsl");
        if dev_shader.exists() {
            config.shader_path = dev_shader;
        }
    }

    tracing::info!(shader = %config.shader_path.display(), hostname = %hostname, "starting cosmic-audio-bg");

    let (audio, audio_handle) = AudioAnalyzer::start(config.audio_sensitivity)?;

    let conn = Connection::connect_to_env().context("failed to connect to Wayland")?;
    let (globals, event_queue) = registry_queue_init(&conn).context("registry init failed")?;
    let qh = event_queue.handle();

    let mut event_loop = smithay_client_toolkit::reexports::calloop::EventLoop::try_new()?;
    let loop_signal = event_loop.get_signal();

    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.error)?;

    let mut state = AppState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state: CompositorState::bind(&globals, &qh)?,
        shm_state: Shm::bind(&globals, &qh)?,
        layer_state: LayerShell::bind(&globals, &qh)?,
        seat_state: SeatState::new(&globals, &qh),
        viewporter: globals.bind(&qh, 1..=1, ())?,
        fractional_scale_manager: globals.bind(&qh, 1..=1, ()).ok(),
        qh,
        config,
        config_path: args.config.clone(),
        layers: Vec::new(),
        renderer: None,
        audio,
        _audio_handle: audio_handle,
        loop_signal,
        exit: false,
        idle_since: None,
        idle_blend: 0.0,
        pointer: None,
        keyboard: None,
        modifiers: Modifiers::default(),
        menu: Menu::new(),
    };

    schedule_frame_timer(&mut event_loop, &state);

    loop {
        event_loop.dispatch(None, &mut state)?;
        if state.exit {
            break;
        }
    }

    Ok(())
}

fn schedule_frame_timer(
    event_loop: &mut smithay_client_toolkit::reexports::calloop::EventLoop<AppState>,
    state: &AppState,
) {
    let fps = state.fps().max(1);
    let period = Duration::from_millis(1000 / fps as u64);
    let timer = Timer::from_duration(period);
    event_loop
        .handle()
        .insert_source(timer, move |_, _, state| {
            state.redraw_all();
            let fps = state.fps().max(1);
            TimeoutAction::ToDuration(Duration::from_millis(1000 / fps as u64))
        })
        .expect("failed to insert frame timer");
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        if self.fractional_scale_manager.is_none() {
            for layer in &mut self.layers {
                if layer.surface.wl_surface() == surface {
                    layer.fractional_scale =
                        Some(new_factor.max(1) as u32 * layer::FRACTIONAL_SCALE_UNIT);
                    layer.needs_redraw = true;
                    self.renderer = None;
                    break;
                }
            }
        }
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let name = info.name.clone().unwrap_or_default();
        if !self.output_allowed(&name) {
            tracing::info!(output = %name, "skipping output per config");
            return;
        }
        tracing::info!(
            output = %name,
            scale = info.scale_factor,
            logical = ?info.logical_size,
            "output detected"
        );
        let new_layer = self.create_layer(output, info);
        self.layers.push(new_layer);
        self.renderer = None;
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        if self.fractional_scale_manager.is_none() {
            if let Some(info) = self.output_state.info(&output) {
                for layer in &mut self.layers {
                    if layer.wl_output == output {
                        layer.output_info = info.clone();
                        layer.fractional_scale = Some(
                            info.scale_factor.max(1) as u32 * layer::FRACTIONAL_SCALE_UNIT,
                        );
                        layer.needs_redraw = true;
                        self.renderer = None;
                    }
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: WlOutput,
    ) {
        self.layers.retain(|l| l.wl_output != output);
        self.renderer = None;
    }
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        for l in &mut self.layers {
            if l.surface.wl_surface() == layer.wl_surface() {
                layer::handle_configure(l, &configure, &self.shm_state);
                tracing::info!(
                    logical = ?l.size,
                    buffer = ?l.buffer_size(),
                    fractional_scale = ?l.fractional_scale,
                    "layer configured"
                );
                self.renderer = None;
                self.redraw_all();
                break;
            }
        }
    }
}

impl ShmHandler for AppState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl SeatHandler for AppState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer if self.pointer.is_none() => {
                match self.seat_state.get_pointer(qh, &seat) {
                    Ok(pointer) => {
                        self.pointer = Some(pointer);
                        tracing::info!("pointer acquired (right-click menu enabled)");
                    }
                    Err(err) => tracing::warn!(?err, "failed to create pointer"),
                }
            }
            Capability::Keyboard if self.keyboard.is_none() => {
                match self
                    .seat_state
                    .get_keyboard::<AppState, AppState>(qh, &seat, None)
                {
                    Ok(keyboard) => {
                        self.keyboard = Some(keyboard);
                        tracing::info!("keyboard acquired (Esc dismiss enabled)");
                    }
                    Err(err) => tracing::warn!(?err, "failed to create keyboard"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            }
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}
}

impl PointerHandler for AppState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Press { button, .. } if button == BTN_RIGHT => {
                    // Right-click (with or without Super held) opens the menu
                    // at the cursor. Super is detected via the tracked keyboard
                    // modifiers; it is logged but not required, since plain
                    // right-click on the background layer is the most reliable
                    // trigger.
                    let size = self.layer_for_surface(&event.surface).and_then(|l| l.size);
                    if let Some((lw, lh)) = size {
                        let surface = event.surface.clone();
                        let super_held = self.modifiers.logo;
                        self.menu.open_at(surface, x, y, lw as f32, lh as f32);
                        tracing::info!(super_held, x, y, "visualization mode menu opened");
                        self.redraw_all();
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if self.menu.open && self.menu.belongs_to(&event.surface) {
                        if let Some(idx) = self.menu.item_at(x, y) {
                            let mode = self.menu.items[idx].mode;
                            self.menu.close();
                            self.select_mode(mode);
                            self.redraw_all();
                        } else if !self.menu.contains(x, y) {
                            self.menu.close();
                            self.redraw_all();
                        }
                    }
                }
                PointerEventKind::Motion { .. } => {
                    if self.menu.open && self.menu.belongs_to(&event.surface) {
                        let hovered = self.menu.item_at(x, y);
                        if hovered != self.menu.hovered {
                            self.menu.hovered = hovered;
                            self.redraw_all();
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl KeyboardHandler for AppState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape && self.menu.open {
            self.menu.close();
            self.redraw_all();
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_layer!(AppState);
delegate_shm!(AppState);
delegate_seat!(AppState);
delegate_pointer!(AppState);
delegate_keyboard!(AppState);
delegate_registry!(AppState);

delegate_noop!(AppState: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(AppState: wp_viewporter::WpViewporter);
delegate_noop!(AppState: wp_viewport::WpViewport);

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, Weak<WlSurface>> for AppState {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        surface: &Weak<WlSurface>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };

        let Ok(surface) = surface.upgrade() else {
            return;
        };

        for layer in &mut state.layers {
            if layer.surface.wl_surface() == &surface {
                layer.fractional_scale = Some(scale);
                layer.needs_redraw = true;
                state.renderer = None;
                layer::ensure_pool(layer, &state.shm_state);
                tracing::info!(scale, buffer = ?layer.buffer_size(), "fractional scale updated");
                state.redraw_all();
                break;
            }
        }
    }
}
