//! A tiny, self-contained right-click menu for choosing the visualization
//! mode. The menu is composited (CPU side, alpha-blended) directly onto the
//! rendered wallpaper frame before it is uploaded to the SHM buffer, so it
//! needs no extra Wayland surface, popup or font dependency.
//!
//! Coordinates: the menu stores its geometry in *logical* pixels (the surface
//! coordinate space that `wl_pointer` reports). When drawing we multiply by the
//! surface scale to reach physical buffer pixels.

use cosmic_audio_bg_config::VisualizationMode;
use image::RgbaImage;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;

/// One selectable row in the menu.
pub struct MenuItem {
    pub mode: VisualizationMode,
    pub label: &'static str,
}

pub struct Menu {
    pub open: bool,
    /// Surface (layer) the menu is anchored to. Only that layer draws it.
    surface: Option<WlSurface>,
    /// Top-left of the panel, in logical pixels.
    anchor: (f32, f32),
    /// Row index currently under the pointer (for highlight).
    pub hovered: Option<usize>,
    pub items: Vec<MenuItem>,
}

// Layout constants, in logical pixels.
const PANEL_W: f32 = 220.0;
const TITLE_H: f32 = 28.0;
const ROW_H: f32 = 32.0;
const BORDER: f32 = 2.0;
const LEFT_PAD: f32 = 26.0; // room for the "current mode" marker
const FONT_PX: f32 = 2.0; // logical px per glyph pixel
const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

impl Menu {
    pub fn new() -> Self {
        Self {
            open: false,
            surface: None,
            anchor: (0.0, 0.0),
            hovered: None,
            items: vec![
                MenuItem {
                    mode: VisualizationMode::Stripes,
                    label: "STRIPES",
                },
                MenuItem {
                    mode: VisualizationMode::Composite,
                    label: "COMPOSITE",
                },
            ],
        }
    }

    fn panel_height(&self) -> f32 {
        BORDER * 2.0 + TITLE_H + self.items.len() as f32 * ROW_H
    }

    /// Open (or reposition) the menu at the given pointer location, clamping it
    /// to stay fully on screen.
    pub fn open_at(&mut self, surface: WlSurface, x: f32, y: f32, logical_w: f32, logical_h: f32) {
        let w = PANEL_W;
        let h = self.panel_height();
        let ax = x.min((logical_w - w).max(0.0)).max(0.0);
        let ay = y.min((logical_h - h).max(0.0)).max(0.0);
        self.anchor = (ax, ay);
        self.surface = Some(surface);
        self.open = true;
        self.hovered = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.surface = None;
        self.hovered = None;
    }

    /// Does this menu belong to (draw on) the given surface?
    pub fn belongs_to(&self, surface: &WlSurface) -> bool {
        self.surface.as_ref() == Some(surface)
    }

    /// True if the logical point is anywhere inside the panel.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (ax, ay) = self.anchor;
        x >= ax && x <= ax + PANEL_W && y >= ay && y <= ay + self.panel_height()
    }

    /// Index of the menu row at the given logical point, if any.
    pub fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let (ax, ay) = self.anchor;
        if x < ax || x > ax + PANEL_W {
            return None;
        }
        let rows_top = ay + BORDER + TITLE_H;
        if y < rows_top || y > ay + self.panel_height() - BORDER {
            return None;
        }
        let idx = ((y - rows_top) / ROW_H) as usize;
        (idx < self.items.len()).then_some(idx)
    }

    /// Composite the menu onto `img`. `scale` converts logical -> physical px.
    pub fn draw(&self, img: &mut RgbaImage, scale: f32, current: VisualizationMode) {
        let (ax, ay) = self.anchor;
        let w = PANEL_W;
        let h = self.panel_height();

        // Drop shadow + border + panel background.
        fill_rect(img, (ax + 3.0) * scale, (ay + 3.0) * scale, w * scale, h * scale, [0, 0, 0], 0.35);
        fill_rect(img, ax * scale, ay * scale, w * scale, h * scale, [56, 168, 92], 0.95);
        fill_rect(
            img,
            (ax + BORDER) * scale,
            (ay + BORDER) * scale,
            (w - BORDER * 2.0) * scale,
            (h - BORDER * 2.0) * scale,
            [12, 22, 16],
            0.93,
        );

        // Title.
        draw_text(
            img,
            (ax + LEFT_PAD * 0.5) * scale,
            (ay + BORDER + 8.0) * scale,
            "MODE",
            scale,
            [150, 235, 170],
        );
        // Title underline.
        fill_rect(
            img,
            (ax + BORDER) * scale,
            (ay + TITLE_H) * scale,
            (w - BORDER * 2.0) * scale,
            1.0_f32.max(scale),
            [56, 168, 92],
            0.6,
        );

        let rows_top = ay + BORDER + TITLE_H;
        for (i, item) in self.items.iter().enumerate() {
            let row_top = rows_top + i as f32 * ROW_H;

            if self.hovered == Some(i) {
                fill_rect(
                    img,
                    (ax + BORDER) * scale,
                    row_top * scale,
                    (w - BORDER * 2.0) * scale,
                    ROW_H * scale,
                    [44, 92, 60],
                    0.85,
                );
            }

            // Marker for the currently active mode.
            if item.mode == current {
                let m = 9.0;
                fill_rect(
                    img,
                    (ax + 9.0) * scale,
                    (row_top + (ROW_H - m) * 0.5) * scale,
                    m * scale,
                    m * scale,
                    [120, 255, 150],
                    0.95,
                );
            }

            let text_color = if item.mode == current {
                [205, 255, 215]
            } else {
                [175, 205, 185]
            };
            draw_text(
                img,
                (ax + LEFT_PAD) * scale,
                (row_top + (ROW_H - GLYPH_H as f32 * FONT_PX) * 0.5) * scale,
                item.label,
                scale,
                text_color,
            );
        }
    }
}

/// Alpha-blend a filled rectangle (physical px) onto the image.
fn fill_rect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, color: [u8; 3], alpha: f32) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = x.round() as i32;
    let y0 = y.round() as i32;
    let x1 = (x + w).round() as i32;
    let y1 = (y + h).round() as i32;
    for py in y0.max(0)..y1.min(ih) {
        for px in x0.max(0)..x1.min(iw) {
            blend(img, px as u32, py as u32, color, alpha);
        }
    }
}

fn blend(img: &mut RgbaImage, x: u32, y: u32, color: [u8; 3], alpha: f32) {
    let p = img.get_pixel_mut(x, y);
    for c in 0..3 {
        p.0[c] = (color[c] as f32 * alpha + p.0[c] as f32 * (1.0 - alpha)).round() as u8;
    }
    p.0[3] = 255;
}

/// Draw an uppercase ASCII string with the embedded 5x7 bitmap font.
fn draw_text(img: &mut RgbaImage, x: f32, y: f32, text: &str, scale: f32, color: [u8; 3]) {
    let block = (FONT_PX * scale).max(1.0);
    let advance = (GLYPH_W as f32 + 1.0) * FONT_PX * scale;
    // `x` and `y` are already in physical pixels.
    let mut cx = x;
    for ch in text.chars() {
        if let Some(rows) = glyph(ch) {
            for (r, row) in rows.iter().enumerate() {
                for (c, cell) in row.bytes().enumerate() {
                    if cell == b'#' {
                        let gx = cx + c as f32 * block;
                        let gy = y + r as f32 * block;
                        for dy in 0..block.round() as i32 {
                            for dx in 0..block.round() as i32 {
                                let px = gx + dx as f32;
                                let py = gy + dy as f32;
                                if px >= 0.0 && py >= 0.0 && (px as u32) < img.width() && (py as u32) < img.height() {
                                    blend(img, px as u32, py as u32, color, 1.0);
                                }
                            }
                        }
                    }
                }
            }
        }
        cx += advance;
    }
}

/// 5x7 uppercase bitmap glyphs. Unknown characters render as blank.
fn glyph(c: char) -> Option<[&'static str; GLYPH_H]> {
    let g = match c.to_ascii_uppercase() {
        ' ' => [".....", ".....", ".....", ".....", ".....", ".....", "....."],
        'A' => [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
        'B' => ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
        'C' => [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
        'D' => ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
        'E' => ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
        'F' => ["#####", "#....", "#....", "####.", "#....", "#....", "#...."],
        'G' => [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###."],
        'H' => ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
        'I' => ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####"],
        'J' => ["..###", "...#.", "...#.", "...#.", "#..#.", "#..#.", ".##.."],
        'K' => ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
        'L' => ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
        'M' => ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
        'N' => ["#...#", "##..#", "#.#.#", "#.#.#", "#..##", "#...#", "#...#"],
        'O' => [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
        'P' => ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
        'Q' => [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"],
        'R' => ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
        'S' => [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
        'T' => ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
        'U' => ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
        'V' => ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
        'W' => ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"],
        'X' => ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"],
        'Y' => ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
        'Z' => ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"],
        '0' => [".###.", "#..##", "#.#.#", "#.#.#", "##..#", "#...#", ".###."],
        '1' => ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", "#####"],
        '2' => [".###.", "#...#", "....#", "..##.", ".#...", "#....", "#####"],
        '3' => ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
        '4' => ["#...#", "#...#", "#...#", "#####", "....#", "....#", "....#"],
        '5' => ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."],
        '6' => [".###.", "#....", "#....", "####.", "#...#", "#...#", ".###."],
        '7' => ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."],
        '8' => [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
        '9' => [".###.", "#...#", "#...#", ".####", "....#", "....#", ".###."],
        _ => return None,
    };
    Some(g)
}
