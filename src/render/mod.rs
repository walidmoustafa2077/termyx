mod atlas;
mod glyph_cache;

use tiny_skia::Pixmap;

use crate::theme::Theme;

pub struct Renderer {
    pixmap: Pixmap,
    pixels: Vec<u32>,
    theme: Theme,
    _padding: u32,
}

impl Renderer {
    pub fn new(theme: Theme, padding: u32) -> Self {
        Self {
            pixmap: Pixmap::new(1, 1).expect("1x1 pixmap"),
            pixels: vec![0],
            theme,
            _padding: padding,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.pixmap = Pixmap::new(width, height).expect("pixmap allocation");
        self.pixels = vec![0; (width as usize) * (height as usize)];
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync_pixels();
    }

    pub fn draw(&mut self) {
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync_pixels();
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    fn sync_pixels(&mut self) {
        for (index, pixel) in self.pixmap.data().chunks_exact(4).enumerate() {
            self.pixels[index] =
                ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        }
    }
}
