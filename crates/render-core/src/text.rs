//! Lettering overlay: draws the engine's active text slot over the frame.
//!
//! Several 8x8 bitmap fonts (from font8x8, plus derived bold/outline styles and
//! the Standard Galactic alien alphabet) are baked into 16x16 glyph atlases at
//! startup. Each frame the active sentence is uploaded as a 1-row code texture
//! and a single full-screen pass maps pixels to glyphs (see `shaders/text.frag`),
//! alpha-blended over the screen with selectable text FX.

use font8x8::legacy::{BASIC_LEGACY, SGA_LEGACY};

use crate::engine::Engine;

use super::gl::{self, FullscreenQuad, Gl, GlslFlavor, Program};

const ATLAS: i32 = 128; // 16 cols * 8px (bitmap fonts)
const TTF_CELL: i32 = 16;
const TTF_ATLAS: i32 = TTF_CELL * 16; // 256

/// Real pixel typefaces (OFL), rasterized into atlases at startup, with the
/// raster px size tuned to each face's design size.
const TTF_FONTS: &[(&[u8], f32)] = &[
    (include_bytes!("../../../assets/fonts/vt323.ttf"), 16.0),
    (include_bytes!("../../../assets/fonts/silkscreen.ttf"), 11.0),
    (include_bytes!("../../../assets/fonts/pressstart.ttf"), 10.0),
];

/// Font styles, selected by the `text.font` param. Names mirror the UI dropdown.
/// First four are the font8x8 bitmap set; the rest are rasterized TTFs.
pub const FONT_NAMES: &[&str] = &["system", "bold", "outline", "alien", "vt323", "silkscreen", "arcade"];

pub struct TextOverlay {
    gl: Gl,
    prog: Program,
    fonts: Vec<glow::Texture>,
    code_tex: glow::Texture,
    last_active: Option<usize>,
    fade_start: f32,
}

/// Bold: dilate the glyph horizontally and vertically.
fn dilate(g: [u8; 8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for y in 0..8 {
        let row = g[y] | (g[y] << 1) | (g[y] >> 1);
        out[y] = row;
    }
    let src = out;
    for y in 0..8 {
        let up = if y > 0 { src[y - 1] } else { 0 };
        let dn = if y < 7 { src[y + 1] } else { 0 };
        out[y] = src[y] | up | dn;
    }
    out
}

/// Outline: the 1px ring around the glyph (dilate AND NOT original).
fn outline(g: [u8; 8]) -> [u8; 8] {
    let d = dilate(g);
    let mut out = [0u8; 8];
    for y in 0..8 {
        out[y] = d[y] & !g[y];
    }
    out
}

/// The 8x8 bitmap for code `c` in a given style.
fn glyph(style: usize, c: usize) -> [u8; 8] {
    let base = BASIC_LEGACY[c & 0x7f];
    match style {
        1 => dilate(base),
        2 => outline(base),
        3 => {
            // Alien: map A-Z (upper or lower) to Standard Galactic, else blank.
            let ch = c as u8;
            let idx = if ch.is_ascii_uppercase() {
                Some((ch - b'A') as usize)
            } else if ch.is_ascii_lowercase() {
                Some((ch - b'a') as usize)
            } else {
                None
            };
            idx.map(|i| SGA_LEGACY[i]).unwrap_or([0; 8])
        }
        _ => base,
    }
}

/// Bake one font style into a 128x128 RGBA atlas (white ink, alpha = pixel).
fn bake(gl: &Gl, style: usize) -> glow::Texture {
    let mut atlas = vec![0u8; (ATLAS * ATLAS * 4) as usize];
    for c in 0..128usize {
        let g = glyph(style, c);
        let (cx, cy) = ((c % 16) as i32 * 8, (c / 16) as i32 * 8);
        for (gy, row) in g.iter().enumerate() {
            for gx in 0..8 {
                if (row >> gx) & 1 == 1 {
                    let o = (((cy + gy as i32) * ATLAS + cx + gx as i32) * 4) as usize;
                    atlas[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    gl::make_texture(gl, ATLAS, ATLAS, Some(&atlas), true)
}

/// Rasterize a TTF into a 256x256 atlas (16px cells), one ASCII glyph per cell,
/// centred and thresholded for a crisp pixel look. The shader samples cells in
/// normalised space, so the larger atlas mixes freely with the bitmap fonts.
fn bake_ttf(gl: &Gl, bytes: &[u8], px: f32) -> glow::Texture {
    let dim = TTF_ATLAS;
    let mut atlas = vec![0u8; (dim * dim * 4) as usize];
    if let Ok(font) = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
        for c in 32u8..127 {
            let (m, bm) = font.rasterize(c as char, px);
            let cell = TTF_CELL;
            let cx = (c as i32 % 16) * cell;
            let cy = (c as i32 / 16) * cell;
            let ox = cx + ((cell - m.width as i32) / 2).max(0);
            let oy = cy + ((cell - m.height as i32) / 2).max(0);
            for gy in 0..m.height {
                for gx in 0..m.width {
                    if bm[gy * m.width + gx] > 110 {
                        let (x, y) = (ox + gx as i32, oy + gy as i32);
                        if x >= cx && x < cx + cell && y >= cy && y < cy + cell {
                            let o = ((y * dim + x) * 4) as usize;
                            atlas[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
    gl::make_texture(gl, dim, dim, Some(&atlas), true)
}

impl TextOverlay {
    pub fn new(gl: &Gl, flavor: GlslFlavor) -> Result<Self, String> {
        // Four bitmap styles, then the rasterized TTF faces.
        let mut fonts: Vec<glow::Texture> = (0..4).map(|s| bake(gl, s)).collect();
        for (bytes, px) in TTF_FONTS {
            fonts.push(bake_ttf(gl, bytes, *px));
        }
        let code_tex = gl::make_texture(gl, 1, 1, None, true);

        let lib = include_str!("shaders/lib.glsl");
        let vert = include_str!("shaders/fullscreen.vert");
        let body = format!("{lib}\n{}", include_str!("shaders/text.frag"));
        let prog = Program::new(gl, flavor, vert, &body).map_err(|e| format!("text: {e}"))?;

        Ok(Self { gl: gl.clone(), prog, fonts, code_tex, last_active: None, fade_start: 0.0 })
    }

    /// Draw the active text (if any) over the currently-bound screen.
    pub fn draw(&mut self, quad: &FullscreenQuad, engine: &Engine, time: f32, out_w: i32, out_h: i32) {
        let active = engine.text_active();
        if active != self.last_active {
            self.fade_start = time;
            self.last_active = active;
        }
        let Some(slot) = active else { return };
        let text = engine.text_slot(slot).trim();
        if text.is_empty() {
            return;
        }

        let codes: Vec<u8> = text
            .chars()
            .flat_map(|ch| {
                let code = if (ch as u32) < 128 { ch as u8 } else { b'?' };
                [code, 0, 0, 0]
            })
            .collect();
        let count = (codes.len() / 4) as i32;
        gl::update_texture(&self.gl, self.code_tex, count, 1, &codes);

        let p = engine.params();
        let readf = |path: &str, d: f32| p.id_of(path).map(|id| p.get_f32(id)).unwrap_or(d);
        let readi = |path: &str, d: i64| p.id_of(path).map(|id| p.get(id).as_i64()).unwrap_or(d);
        let size = readf("text.size", 0.1);
        let (posx, posy) = (readf("text.posx", 0.0), readf("text.posy", 0.0));
        let hue = readf("text.hue", 0.0);
        let fxamt = readf("text.fxamt", 0.0);
        let fx = readi("text.fx", 0) as i32;
        let font = (readi("text.font", 0) as usize).min(self.fonts.len() - 1);

        let gh = size;
        let gw = size * (out_h as f32 / out_w.max(1) as f32);
        let total = gw * count as f32;
        let start_x = 0.5 + posx * 0.5 - total * 0.5;
        let start_y_top = 0.5 + posy * 0.5 + gh * 0.5;
        let alpha = ((time - self.fade_start) / 0.25).clamp(0.0, 1.0);

        gl::bind_screen(&self.gl, out_w, out_h);
        gl::set_blend(&self.gl, true);
        self.prog.bind();
        self.prog.set_texture("u_font", 0, self.fonts[font]);
        self.prog.set_texture("u_text", 1, self.code_tex);
        self.prog.set_vec2("u_start", start_x, start_y_top);
        self.prog.set_vec2("u_glyph", gw, gh);
        self.prog.set_f32("u_count", count as f32);
        self.prog.set_f32("u_alpha", alpha);
        self.prog.set_f32("u_fxamt", fxamt);
        self.prog.set_i32("u_fx", fx);
        self.prog.set_f32("u_hue", hue);
        self.prog.set_f32("u_time", time);
        self.prog.set_vec2("u_res", out_w as f32, out_h as f32);
        quad.draw();
        gl::set_blend(&self.gl, false);
    }
}
