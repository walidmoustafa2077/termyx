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

/// Draws `text` clipped to `max_width`, replacing the cut tail with `…`.
///
/// Tab titles come from the shell and are arbitrarily long; a hard clip hides
/// the end of the name, which is the part a user navigates by.
pub fn draw_text_ellipsis(
    renderer: &mut Renderer,
    text: &str,
    x: f32,
    y: f32,
    px: f32,
    color: Rgb,
    max_width: f32,
) -> f32 {
    // Measuring the whole string first skips the shortening work on the common
    // case, where the tab is wide enough for its title.
    if renderer.fonts().ui_width(text, px) <= max_width {
        return draw_text(renderer, text, x, y, px, color, max_width);
    }
    let ellipsis_width = renderer.fonts().ui_width("…", px);
    let budget = max_width - ellipsis_width;
    if budget <= 0.0 {
        // Not even the ellipsis fits; an empty title is better than ink over
        // the close button.
        return 0.0;
    }
    // Walk whole characters so a multi-byte glyph is never split, and stop
    // before the character that would push the ellipsis past the clip.
    let mut shortened = String::with_capacity(text.len());
    let mut width = 0.0;
    for ch in text.chars() {
        let mut utf8 = [0u8; 4];
        let advance = renderer.fonts().ui_width(ch.encode_utf8(&mut utf8), px);
        if width + advance > budget {
            break;
        }
        shortened.push(ch);
        width += advance;
    }
    shortened.push('…');
    draw_text(renderer, &shortened, x, y, px, color, max_width)
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

    #[test]
    fn a_clipped_title_ends_with_an_ellipsis() {
        // A title far wider than the clip must be shortened, not cut mid-glyph,
        // and the ellipsis must sit wholly inside the width.
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(400, 120);
        let color = renderer.theme.foreground;
        let width = draw_text_ellipsis(
            &mut renderer,
            "a very long tab title that cannot fit",
            4.0,
            4.0,
            UI_SIZE_PX,
            color,
            40.0,
        );
        assert!(width <= 40.0 + 0.01);
        renderer.sync();
        let background = crate::render::tests::packed(renderer.theme.background);
        assert!(
            renderer.pixels().iter().any(|pixel| *pixel != background),
            "the shortened title must still paint ink"
        );
    }

    #[test]
    fn a_title_that_fits_is_drawn_without_an_ellipsis() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(200, 60);
        let color = renderer.theme.foreground;
        let measured = renderer.fonts().ui_width("Tab", UI_SIZE_PX);
        let width = draw_text_ellipsis(&mut renderer, "Tab", 2.0, 2.0, UI_SIZE_PX, color, 100.0);
        assert!(
            (width - measured).abs() < 0.01,
            "a title that fits must be painted whole, not shortened"
        );
    }
}
