//! xcursor theme loading and frame selection for the compositor cursor.
//!
//! Loads the system `left_ptr` cursor (honouring `XCURSOR_THEME`/`XCURSOR_SIZE`)
//! into RGBA8888 frames the GLES renderer can upload as a memory buffer, with a
//! built-in arrow fallback when no theme is found.

use xcursor::parser::Image as XcursorImage;
use xcursor::CursorTheme;

/// Loaded cursor data: RGBA8888 frames (byte order `R,G,B,A`, matching
/// [`smithay::backend::allocator::Fourcc::Abgr8888`]) with hotspot and
/// per-frame delay.
pub struct CursorFrames {
    pub frames: Vec<CursorFrame>,
}

pub struct CursorFrame {
    /// RGBA8888, row-major (byte order `R,G,B,A`).
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    pub delay_ms: u32,
}

impl CursorFrames {
    /// Load the `left_ptr` cursor from the system theme, falling back to a
    /// built-in minimal arrow if no theme is found.
    pub fn load_default() -> Self {
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(24);
        let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
        let theme = CursorTheme::load(&theme_name);

        for name in &["left_ptr", "default", "arrow", "top_left_arrow"] {
            let Some(path) = theme.load_icon(name) else { continue };
            let Ok(data) = std::fs::read(&path) else { continue };
            let Some(images) = xcursor::parser::parse_xcursor(&data) else { continue };
            let frames = pick_size_frames(&images, size);
            if !frames.is_empty() {
                return Self { frames };
            }
        }

        Self { frames: vec![builtin_arrow()] }
    }

    /// Select the current frame index based on time (milliseconds).
    pub fn frame_at(&self, time_ms: u32) -> &CursorFrame {
        if self.frames.len() == 1 {
            return &self.frames[0];
        }
        let total: u32 = self.frames.iter().map(|f| f.delay_ms).sum();
        if total == 0 {
            return &self.frames[0];
        }
        let t = time_ms % total;
        let mut acc = 0u32;
        for frame in &self.frames {
            acc += frame.delay_ms;
            if t < acc {
                return frame;
            }
        }
        &self.frames[0]
    }
}

/// Pick the animation frames for the size closest to `size`, in file order.
fn pick_size_frames(images: &[XcursorImage], size: u32) -> Vec<CursorFrame> {
    let Some(best_size) = images.iter().map(|img| img.size).min_by_key(|&s| (s as i64 - size as i64).abs()) else {
        return Vec::new();
    };
    images
        .iter()
        .filter(|img| img.size == best_size)
        .map(|img| CursorFrame {
            pixels: img.pixels_rgba.clone(),
            width: img.width,
            height: img.height,
            hotspot_x: img.xhot as i32,
            hotspot_y: img.yhot as i32,
            delay_ms: img.delay.max(1),
        })
        .collect()
}

/// A built-in 16x16 sharp arrow cursor — RGBA8888 pixel data (byte order
/// `R,G,B,A`). Rendered with a white outline + near-black fill for maximum
/// visibility against any background.
fn builtin_arrow() -> CursorFrame {
    const W: usize = 16;
    const H: usize = 16;
    // 1=fill (near-black), 2=outline (white), 0=transparent.
    #[rustfmt::skip]
    const MASK: [[u8; W]; H] = [
        [2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [2,2,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [2,1,2,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [2,1,1,2,0,0,0,0,0,0,0,0,0,0,0,0],
        [2,1,1,1,2,0,0,0,0,0,0,0,0,0,0,0],
        [2,1,1,1,1,2,0,0,0,0,0,0,0,0,0,0],
        [2,1,1,1,1,1,2,0,0,0,0,0,0,0,0,0],
        [2,1,1,1,1,1,1,2,0,0,0,0,0,0,0,0],
        [2,1,1,1,1,1,1,1,2,0,0,0,0,0,0,0],
        [2,1,1,1,1,1,2,2,2,0,0,0,0,0,0,0],
        [2,1,1,2,1,1,2,0,0,0,0,0,0,0,0,0],
        [2,1,2,0,2,1,1,2,0,0,0,0,0,0,0,0],
        [2,2,0,0,0,2,1,1,2,0,0,0,0,0,0,0],
        [0,0,0,0,0,0,2,1,1,2,0,0,0,0,0,0],
        [0,0,0,0,0,0,0,2,2,0,0,0,0,0,0,0],
        [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
    ];

    let mut pixels = vec![0u8; W * H * 4];
    for (y, row) in MASK.iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            let i = (y * W + x) * 4;
            match v {
                1 => pixels[i..i + 4].copy_from_slice(&[10, 10, 10, 255]),
                2 => pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]),
                _ => {}
            }
        }
    }

    CursorFrame { pixels, width: W as u32, height: H as u32, hotspot_x: 0, hotspot_y: 0, delay_ms: 0 }
}
