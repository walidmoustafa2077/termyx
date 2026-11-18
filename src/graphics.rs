//! Images an application asked the terminal to draw, and how they reach the
//! pixmap.

use std::sync::Arc;

/// A decoded image, straight RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Where the grid cursor stood when the image arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub line: i32,
    pub col: usize,
}

/// An image pinned to a grid cell, so it scrolls with the line it was drawn on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub image: Arc<Image>,
    pub line: i32,
    pub col: usize,
    /// Whether the cells under the image held nothing but blanks the last time
    /// the grid was checked. An application shows a picture by leaving its cell
    /// blank and drawing text over it when the picture is gone.
    pub blank: bool,
}

/// The images the grid currently shows.
#[derive(Debug, Default)]
pub struct Graphics {
    placements: Vec<Placement>,
}

/// An application that repaints re-sends the same image at the same cell; the
/// newest one wins so a redrawing screen does not pile images up.
const MAX_PLACEMENTS: usize = 64;

impl Graphics {
    pub fn place(&mut self, image: Image, origin: Origin, blank: bool) {
        self.placements
            .retain(|placement| placement.line != origin.line || placement.col != origin.col);
        if self.placements.len() >= MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        self.placements.push(Placement {
            image: Arc::new(image),
            line: origin.line,
            col: origin.col,
            blank,
        });
    }

    /// Drops an image once text is painted where it used to stand on blank
    /// cells, which is how an application takes a picture back off the screen:
    /// it redraws the text the picture was hiding.
    pub fn retain_blank(&mut self, regions: &[(i32, usize, bool)]) {
        self.placements.retain_mut(|placement| {
            let Some((_, _, blank)) = regions
                .iter()
                .find(|(line, col, _)| *line == placement.line && *col == placement.col)
            else {
                return true;
            };
            let covered = placement.blank && !*blank;
            placement.blank = *blank;
            !covered
        });
    }

    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    pub fn clear(&mut self) {
        self.placements.clear();
    }
}

/// Draws an image pixel for pixel at (`x`, `y`) into `target`, which holds
/// premultiplied RGBA the way tiny-skia stores it.
///
/// The image may hang off any edge, so pixels outside the target are skipped
/// rather than clamped.
pub fn blit(target: &mut [u8], target_width: usize, image: &Image, x: i32, y: i32) {
    if target_width == 0 || image.width == 0 || image.height == 0 {
        return;
    }
    // `target` counts RGBA bytes, so a row spans four bytes per pixel.
    let target_height = target.len() / (target_width * 4);
    let stride = image.width as usize;
    for row in 0..image.height as i32 {
        let dest_y = y + row;
        if dest_y < 0 || dest_y as usize >= target_height {
            continue;
        }
        for column in 0..image.width as i32 {
            let dest_x = x + column;
            if dest_x < 0 || dest_x as usize >= target_width {
                continue;
            }
            let texel = (row as usize * stride + column as usize) * 4;
            let alpha = u32::from(image.rgba[texel + 3]);
            if alpha == 0 {
                continue;
            }
            let index = (dest_y as usize * target_width + dest_x as usize) * 4;
            let keep = 255 - alpha;
            for channel in 0..3 {
                let source = u32::from(image.rgba[texel + channel]) * alpha / 255;
                target[index + channel] =
                    (source + u32::from(target[index + channel]) * keep / 255).min(255) as u8;
            }
            target[index + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        Image {
            width,
            height,
            rgba: pixel
                .iter()
                .copied()
                .cycle()
                .take((width * height * 4) as usize)
                .collect(),
        }
    }

    #[test]
    fn an_opaque_image_replaces_the_cells_under_it() {
        let mut target = vec![0u8; 2 * 4];
        blit(&mut target, 2, &image(1, 1, [255, 0, 255, 255]), 0, 0);
        assert_eq!(&target[..4], &[255, 0, 255, 255]);
        assert_eq!(&target[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_transparent_pixel_keeps_what_is_under_it() {
        let mut target = vec![10u8, 20, 30, 255];
        blit(&mut target, 1, &image(1, 1, [255, 255, 255, 0]), 0, 0);
        assert_eq!(target, vec![10, 20, 30, 255]);
    }

    #[test]
    fn an_image_that_hangs_off_the_edge_paints_only_what_fits() {
        let mut target = vec![0u8; 4 * 4];
        blit(&mut target, 2, &image(2, 2, [1, 2, 3, 255]), -1, -1);
        assert_eq!(&target[..4], &[1, 2, 3, 255]);
        assert_eq!(&target[4..8], &[0, 0, 0, 0]);
        assert_eq!(target.len(), 16);
    }

    #[test]
    fn an_image_below_the_target_paints_nothing() {
        let mut target = vec![0u8; 2 * 2 * 4];
        blit(&mut target, 2, &image(1, 1, [255, 0, 255, 255]), 0, 2);
        assert_eq!(target, vec![0u8; 2 * 2 * 4]);
    }

    #[test]
    fn a_repaint_at_the_same_cell_replaces_the_older_image() {
        let mut graphics = Graphics::default();
        let origin = Origin { line: 3, col: 2 };
        graphics.place(image(1, 1, [255, 0, 0, 255]), origin, true);
        graphics.place(image(1, 1, [0, 255, 0, 255]), origin, false);
        assert_eq!(graphics.placements().len(), 1);
        assert_eq!(graphics.placements()[0].image.rgba[1], 255);
        assert!(!graphics.placements()[0].blank);
    }

    #[test]
    fn clearing_forgets_every_placement() {
        let mut graphics = Graphics::default();
        graphics.place(
            image(1, 1, [255, 0, 0, 255]),
            Origin { line: 0, col: 0 },
            true,
        );
        graphics.clear();
        assert!(graphics.placements().is_empty());
    }

    #[test]
    fn an_image_stays_while_its_cells_stay_blank() {
        let mut graphics = Graphics::default();
        graphics.place(
            image(1, 1, [255, 0, 0, 255]),
            Origin { line: 2, col: 5 },
            true,
        );
        graphics.retain_blank(&[(2, 5, true)]);
        assert_eq!(graphics.placements().len(), 1);
    }

    #[test]
    fn an_image_dies_when_text_is_painted_over_it() {
        let mut graphics = Graphics::default();
        graphics.place(
            image(1, 1, [255, 0, 0, 255]),
            Origin { line: 2, col: 5 },
            true,
        );
        graphics.retain_blank(&[(2, 5, false)]);
        assert!(graphics.placements().is_empty());
    }

    #[test]
    fn text_that_arrives_after_the_cells_were_blanked_removes_the_image() {
        let mut graphics = Graphics::default();
        graphics.place(
            image(1, 1, [255, 0, 0, 255]),
            Origin { line: 2, col: 5 },
            false,
        );
        graphics.retain_blank(&[(2, 5, true)]);
        assert_eq!(graphics.placements().len(), 1, "blanking keeps it");
        graphics.retain_blank(&[(2, 5, false)]);
        assert!(graphics.placements().is_empty(), "text takes it back");
    }

    #[test]
    fn an_image_off_screen_is_kept() {
        let mut graphics = Graphics::default();
        graphics.place(
            image(1, 1, [255, 0, 0, 255]),
            Origin { line: 2, col: 5 },
            true,
        );
        graphics.retain_blank(&[]);
        assert_eq!(graphics.placements().len(), 1);
    }

    #[test]
    fn the_oldest_placement_makes_room_when_the_limit_is_reached() {
        let mut graphics = Graphics::default();
        for line in 0..MAX_PLACEMENTS + 1 {
            graphics.place(
                image(1, 1, [255, 0, 0, 255]),
                Origin {
                    line: line as i32,
                    col: 0,
                },
                true,
            );
        }
        assert_eq!(graphics.placements().len(), MAX_PLACEMENTS);
        assert_eq!(graphics.placements()[0].line, 1);
    }
}
