//! Shelf-packed coverage atlas for rasterized glyphs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    x: u32,
}

pub struct Atlas {
    width: u32,
    height: u32,
    data: Vec<u8>,
    shelves: Vec<Shelf>,
}

impl Atlas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize)],
            shelves: Vec::new(),
        }
    }

    pub fn alloc(&mut self, w: u32, h: u32) -> Option<Rect> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        for shelf in &mut self.shelves {
            if shelf.height >= h && shelf.x + w <= self.width {
                let rect = Rect {
                    x: shelf.x,
                    y: shelf.y,
                    w,
                    h,
                };
                shelf.x += w;
                return Some(rect);
            }
        }
        let y = self
            .shelves
            .last()
            .map_or(0, |shelf| shelf.y + shelf.height);
        if y + h > self.height {
            return None;
        }
        self.shelves.push(Shelf { y, height: h, x: w });
        Some(Rect { x: 0, y, w, h })
    }

    pub fn upload(&mut self, rect: Rect, mask: &[u8]) {
        debug_assert_eq!(mask.len(), (rect.w * rect.h) as usize);
        for row in 0..rect.h {
            let src = (row * rect.w) as usize;
            let dst = ((rect.y + row) * self.width + rect.x) as usize;
            self.data[dst..dst + rect.w as usize]
                .copy_from_slice(&mask[src..src + rect.w as usize]);
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn width(&self) -> u32 {
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_do_not_overlap() {
        let mut atlas = Atlas::new(8, 8);
        let a = atlas.alloc(4, 4).unwrap();
        let b = atlas.alloc(4, 4).unwrap();
        let c = atlas.alloc(4, 4).unwrap();
        let d = atlas.alloc(4, 4).unwrap();
        assert_eq!(
            a,
            Rect {
                x: 0,
                y: 0,
                w: 4,
                h: 4
            }
        );
        assert_eq!(
            b,
            Rect {
                x: 4,
                y: 0,
                w: 4,
                h: 4
            }
        );
        assert_eq!(
            c,
            Rect {
                x: 0,
                y: 4,
                w: 4,
                h: 4
            }
        );
        assert_eq!(
            d,
            Rect {
                x: 4,
                y: 4,
                w: 4,
                h: 4
            }
        );
        assert!(atlas.alloc(1, 1).is_none());
    }

    #[test]
    fn upload_copies_mask_into_atlas() {
        let mut atlas = Atlas::new(4, 4);
        let rect = atlas.alloc(2, 2).unwrap();
        atlas.upload(rect, &[1, 2, 3, 4]);
        assert_eq!(&atlas.data()[0..2], &[1, 2]);
        assert_eq!(&atlas.data()[4..6], &[3, 4]);
    }
}
