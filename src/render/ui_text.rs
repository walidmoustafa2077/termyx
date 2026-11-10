//! Proportional text for window chrome.
//!
//! The terminal grid is monospaced and goes through shaping; chrome text is a
//! handful of short strings in one direction, so it walks the characters and
//! asks the font for each advance.

use ab_glyph::{Font, GlyphId, ScaleFont};

use crate::fonts::{em_px_scale, UI_FONT_ID};
use crate::render::Renderer;
use crate::theme::Rgb;

/// Draws `text` at `(x, y)` — the top-left corner, not the baseline — and
/// returns the width consumed, clipped to `max_width`.
///
/// `px` is an em size, the same unit the glyph cache rasterizes at. Measuring
/// with a raw [`ab_glyph::PxScale`] instead would make the advances about a
/// quarter too tight and overlap the glyph ink. It is already physical: the
/// caller scales the logical chrome size once, with the window's scale factor.
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
        let (font, _index) = renderer.fonts_face(UI_FONT_ID);
        let scaled = font.as_scaled(em_px_scale(font, px));
        for ch in text.chars() {
            let (face, id, advance) = glyph_for(renderer, ch, px);
            if pen + advance > x + max_width {
                break;
            }
            glyphs.push((face, id, advance));
            pen += advance;
        }
        y + scaled.ascent()
    };

    pen = x;
    for (face, id, advance) in glyphs {
        if let Some(slot) = renderer.cache_get(face, id, px.round() as u32) {
            renderer.blit(slot, pen, baseline, color);
        }
        pen += advance;
    }
    pen - x
}

/// Resolves the face id, glyph, and advance (in `px` em units) that will
/// rasterize `ch`.
///
/// The UI face does not map every symbol the chrome draws (`✕`, `⌄`), and
/// painting those with it would produce `.notdef` boxes. When it has no glyph,
/// the same symbol-face lookup the grid uses picks a fallback, so the measured
/// advance and the painted ink come from one face.
fn glyph_for(renderer: &Renderer, ch: char, px: f32) -> (u8, GlyphId, f32) {
    let (ui_font, _index) = renderer.fonts_face(UI_FONT_ID);
    let ui_glyph = ui_font.glyph_id(ch);
    let (face, glyph) = if ui_glyph.0 == 0 {
        renderer
            .fonts()
            .symbol_for_char(ch)
            .unwrap_or((UI_FONT_ID, ui_glyph))
    } else {
        (UI_FONT_ID, ui_glyph)
    };
    let (font, _index) = renderer.fonts_face(face);
    let scaled = font.as_scaled(em_px_scale(font, px));
    (face, glyph, scaled.h_advance(glyph))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::UI_SIZE_PX;

    #[test]
    fn chrome_glyphs_resolve_to_a_face_that_maps_them() {
        let renderer = crate::render::tests::default_renderer();
        for ch in ['+', '✕', '⌄'] {
            let (face, glyph, advance) = glyph_for(&renderer, ch, UI_SIZE_PX);
            let (font, _index) = renderer.fonts_face(face);
            assert_ne!(glyph.0, 0, "the resolved glyph for {ch:?} must exist");
            assert_ne!(
                font.glyph_id(ch).0,
                0,
                "face {face} must rasterize {ch:?}, not .notdef"
            );
            assert!(advance > 0.0, "{ch:?} must have a positive advance");
        }
    }

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
    fn clipped_text_paints_nothing_past_the_available_width() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(200, 60);
        let color = renderer.theme.foreground;
        let limit = 40.0;
        draw_text(
            &mut renderer,
            "a very long tab title",
            2.0,
            2.0,
            UI_SIZE_PX,
            color,
            limit,
        );
        renderer.sync();
        let width = renderer.pixmap.width() as usize;
        let background = crate::render::tests::packed(renderer.theme.background);
        let rightmost = renderer
            .pixels()
            .iter()
            .enumerate()
            .filter(|(_, pixel)| **pixel != background)
            .map(|(index, _)| index % width)
            .max()
            .expect("the text must paint");
        assert!(
            rightmost < (2.0 + limit) as usize,
            "ink must stay inside the clip width (rightmost column {rightmost})"
        );
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
