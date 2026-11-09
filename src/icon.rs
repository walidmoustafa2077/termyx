//! Window icon shared with the executable's shell icon.

use anyhow::Context;

/// 256×256 RGBA rendering of `assets/icons/termyx.ico`.
const ICON_PNG: &[u8] = include_bytes!("../assets/icons/termyx.png");

/// Decodes the bundled icon for the window manager.
pub fn window_icon() -> anyhow::Result<winit::window::Icon> {
    let (rgba, size) = decode_png(ICON_PNG)?;
    winit::window::Icon::from_rgba(rgba, size, size).context("build window icon")
}

fn decode_png(bytes: &[u8]) -> anyhow::Result<(Vec<u8>, u32)> {
    let mut reader = png::Decoder::new(bytes)
        .read_info()
        .context("read icon header")?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).context("decode icon")?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba,
        "icon must be RGBA, found {:?}",
        info.color_type
    );
    anyhow::ensure!(info.width == info.height, "icon must be square");
    buffer.truncate(info.buffer_size());
    Ok((buffer, info.width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icon_is_a_square_rgba_image() {
        let (rgba, size) = decode_png(ICON_PNG).expect("decode icon");

        assert_eq!(size, 256);
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn decoding_rejects_a_non_png() {
        assert!(decode_png(b"not a png").is_err());
    }
}
