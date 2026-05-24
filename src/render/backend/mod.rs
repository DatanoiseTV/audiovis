//! Output backends.
//!
//! - [`window`] - desktop window via winit + glutin (macOS, Linux desktop).
//! - `drm` - Linux direct framebuffer via DRM/KMS + GBM + EGL (added in a later
//!   milestone, compiled only on Linux).

pub mod window;

/// Save an RGBA8 buffer (GL bottom-up order) as a binary PPM (P6), flipping it
/// the right way up. PPM keeps the binary dependency-free; convert to PNG with
/// `ffmpeg -i out.ppm out.png` if a viewable image is wanted.
pub fn save_ppm(path: &str, width: u32, height: u32, rgba_bottom_up: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let (w, h) = (width as usize, height as usize);
    let mut out = Vec::with_capacity(w * h * 3 + 32);
    write!(out, "P6\n{width} {height}\n255\n")?;
    // GL returns rows bottom-to-top; emit top-to-bottom and drop alpha.
    for y in (0..h).rev() {
        let row = &rgba_bottom_up[y * w * 4..(y + 1) * w * 4];
        for px in row.chunks_exact(4) {
            out.extend_from_slice(&px[0..3]);
        }
    }
    std::fs::write(path, out)
}
