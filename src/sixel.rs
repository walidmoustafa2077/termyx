//! Sixel decoding: the DEC image format carried in a DCS string.
//!
//! A sixel character holds one column of six pixels, lowest bit on top, so a
//! stream paints in bands six rows tall. Colours are palette indices defined
//! inline as percentages.

use crate::graphics::Image;

/// How many pixel rows one sixel character covers.
const BAND: usize = 6;

/// Decodes the data between `DCS … q` and `ST`.
///
/// Returns `None` when the stream painted nothing.
pub fn decode(data: &[u8]) -> Option<Image> {
    let mut palette = [[0u8; 3]; 256];
    let mut color = 0usize;
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut width = 0usize;
    let mut band = 0usize;
    let mut x = 0usize;
    let mut index = 0;

    while index < data.len() {
        match data[index] {
            b'"' => {
                // Raster attributes; the painted pixels already give the size.
                index += 1;
                let _ = numbers(data, &mut index);
            }
            b'#' => {
                index += 1;
                let values = numbers(data, &mut index);
                if let Some(&selected) = values.first() {
                    color = selected as usize % palette.len();
                }
                // `#Pc;Pu;Px;Py;Pz` with Pu=2 is red, green, blue; Pu=1 is HLS,
                // which no encoder sends any more.
                if values.len() >= 5 && values[1] == 2 {
                    for channel in 0..3 {
                        palette[color][channel] = percent(values[2 + channel]);
                    }
                }
            }
            b'!' => {
                index += 1;
                let count = numbers(data, &mut index).first().copied().unwrap_or(1) as usize;
                let Some(bits) = data.get(index).and_then(|byte| sixel_bits(*byte)) else {
                    break;
                };
                index += 1;
                for _ in 0..count.max(1) {
                    paint(&mut rows, &mut width, band, x, bits, palette[color]);
                    x += 1;
                }
            }
            b'$' => {
                index += 1;
                x = 0;
            }
            b'-' => {
                index += 1;
                x = 0;
                band += 1;
            }
            byte => {
                if let Some(bits) = sixel_bits(byte) {
                    paint(&mut rows, &mut width, band, x, bits, palette[color]);
                    x += 1;
                }
                index += 1;
            }
        }
    }

    assemble(rows, width)
}

/// Sets the pixels a sixel character stands for.
fn paint(
    rows: &mut Vec<Vec<u8>>,
    width: &mut usize,
    band: usize,
    x: usize,
    bits: u8,
    color: [u8; 3],
) {
    for bit in 0..BAND {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let y = band * BAND + bit;
        while rows.len() <= y {
            rows.push(Vec::new());
        }
        let row = &mut rows[y];
        let offset = x * 4;
        if row.len() < offset + 4 {
            row.resize(offset + 4, 0);
        }
        row[offset..offset + 3].copy_from_slice(&color);
        row[offset + 3] = 255;
    }
    *width = (*width).max(x + 1);
}

fn assemble(rows: Vec<Vec<u8>>, width: usize) -> Option<Image> {
    let height = rows.len();
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = vec![0u8; width * height * 4];
    for (y, row) in rows.iter().enumerate() {
        let row_start = y * width * 4;
        let length = row.len().min(width * 4);
        rgba[row_start..row_start + length].copy_from_slice(&row[..length]);
    }
    Some(Image {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// A sixel character's six pixel bits, or `None` for anything else.
fn sixel_bits(byte: u8) -> Option<u8> {
    (0x3F..=0x7E).contains(&byte).then(|| byte - 0x3F)
}

/// Reads the semicolon-separated parameters that follow a command character.
fn numbers(data: &[u8], index: &mut usize) -> Vec<u32> {
    let mut values = Vec::new();
    let mut value: Option<u32> = None;
    while let Some(&byte) = data.get(*index) {
        match byte {
            b'0'..=b'9' => {
                value = Some(value.unwrap_or(0).saturating_mul(10) + u32::from(byte - b'0'));
                *index += 1;
            }
            b';' => {
                values.push(value.take().unwrap_or(0));
                *index += 1;
            }
            _ => break,
        }
    }
    if let Some(value) = value {
        values.push(value);
    }
    values
}

fn percent(value: u32) -> u8 {
    (value.min(100) * 255 / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sixel_character_paints_one_column_of_six_rows() {
        let image = decode(b"#0;2;100;0;0#0~").expect("a red column");
        assert_eq!((image.width, image.height), (1, 6));
        assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn the_lowest_bit_is_the_top_row_of_the_band() {
        let image = decode(b"#0;2;0;0;100#0@").expect("one pixel");
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.rgba, vec![0, 0, 255, 255]);
    }

    #[test]
    fn a_repeat_count_stands_in_for_repeated_characters() {
        let image = decode(b"#0;2;100;0;0#0!4@").expect("four pixels");
        assert_eq!(image.width, 4);
    }

    #[test]
    fn a_carriage_return_restarts_the_column_within_the_band() {
        let image = decode(b"#0;2;100;0;0#1;2;0;100;0#0@$#1@").expect("one pixel");
        assert_eq!(image.width, 1);
        assert_eq!(&image.rgba[..3], &[0, 255, 0]);
    }

    #[test]
    fn a_band_break_moves_down_six_rows() {
        let image = decode(b"#0;2;100;0;0#0@-#0@").expect("two pixels");
        assert_eq!(image.height, 7);
        assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
        let second = 6 * 4;
        assert_eq!(&image.rgba[second..second + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn raster_attributes_do_not_disturb_the_pixels() {
        let image = decode(b"\"1;1;10;10#0;2;100;0;0#0@").expect("one pixel");
        assert_eq!((image.width, image.height), (1, 1));
    }
}
