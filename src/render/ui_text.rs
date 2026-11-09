//! Proportional text for window chrome.
//!
//! The terminal grid is monospaced and goes through shaping; chrome text is a
//! handful of short strings in one direction, so it walks the characters and
//! asks the font for each advance.

use ab_glyph::{Font, ScaleFont};

use crate::fonts::UI_FONT_ID;
use crate::render::Renderer;
use crate::theme::Rgb;

/// Draws `text` at `(x, y)` — the top-left corner, not the baseline — and
/// returns the width consumed, clipped to `max_width`.
pub fn draw_text(
    renderer: &mut Renderer,
    text: &str,
    x: f32,
    y: f32,
    px: f32,
    color: Rgb,
    max_width: f32,
) -> f32 {
    // Measure before painting: the advances borrow the face, while every blit
    // needs the renderer mutably, so the glyphs are copied out first.
    let mut glyphs = Vec::new();
    let mut pen = x;
    let baseline = {
        let Some((font, _index)) = renderer.fonts_face(UI_FONT_ID) else {
            return 0.0;
        };
        let scaled = font.as_scaled(px);
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let advance = scaled.h_advance(id);
            if pen + advance > x + max_width {
                break;
            }
            glyphs.push((id, advance));
            pen += advance;
        }
        y + scaled.ascent()
    };

    pen = x;
    for (id, advance) in glyphs {
        if let Some(slot) = renderer.cache_get(UI_FONT_ID, id, px.round() as u32) {
            renderer.blit(slot, pen, baseline, color);
        }
        pen += advance;
    }
    pen - x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::UI_SIZE_PX;

    #[test]
    fn text_is_clipped_to_the_available_width() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(200, 60);
        renderer.pixmap.fill(renderer.theme.background.to_tiny());
        let color = renderer.theme.foreground;
        let width = draw_text(
            &mut renderer,
            "a very long tab title",
            2.0,
            2.0,
            UI_SIZE_PX,
            color,
            40.0,
        );
        assert!(width <= 40.0, "the reported width never exceeds the limit");
    }

    #[test]
    fn text_paints_ink_and_reports_its_width() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(200, 60);
        let color = renderer.theme.foreground;
        let width = draw_text(&mut renderer, "Tab", 2.0, 2.0, UI_SIZE_PX, color, 100.0);
        renderer.sync();
        let background = crate::render::tests::packed(renderer.theme.background);
        assert!(width > 0.0, "drawn text must report a positive width");
        assert!(
            renderer.pixels().iter().any(|pixel| *pixel != background),
            "the glyphs must paint over the background"
        );
    }
}
