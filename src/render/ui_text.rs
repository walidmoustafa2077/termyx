//! Proportional text for window chrome.
//!
//! The terminal grid is monospaced and goes through shaping; chrome text is a
//! handful of short strings in one direction, so it walks the characters and
//! asks the font for each advance.

use ab_glyph::{Font, GlyphId, ScaleFont};

use crate::fonts::{em_px_scale, FontSet, UI_FONT_ID};
use crate::render::Renderer;
use crate::theme::Rgb;
use crate::ui::tab_bar::Rect;

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

/// Draws `text` with its ink centred in `rect`, clipped to the rect's width.
///
/// `draw_text` anchors the top of the line box, and the chrome glyphs come
/// from different fallback faces whose ink sits at different heights inside it.
/// Centring by the em size or the line height therefore leaves `+` and `⌄`
/// visibly low; the ink box is what the eye centres, so measure that instead.
pub fn draw_text_centred(
    renderer: &mut Renderer,
    text: &str,
    rect: Rect,
    px: f32,
    color: Rgb,
) -> f32 {
    let (x, y) = {
        let fonts = renderer.fonts();
        match fonts.chrome_ink_bounds(text, px) {
            Some((left, top, right, bottom)) => (
                rect.x + (rect.w - (right + left)) / 2.0,
                rect.y + (rect.h - (bottom - top)) / 2.0 - top,
            ),
            None => {
                // Nothing to measure: fall back to the line box, which is what
                // `draw_text` lays its `y` against.
                let width = fonts.ui_width(text, px);
                let line_h = fonts.ui_line_height(px);
                (
                    rect.x + (rect.w - width) / 2.0,
                    rect.y + (rect.h - line_h) / 2.0,
                )
            }
        }
    };
    draw_text(renderer, text, x, y, px, color, rect.w)
}

/// Shortens `text` to fit `max_width`, replacing the cut tail with `…`.
///
/// Returns `text` unchanged when it already fits. A clip too narrow for the
/// ellipsis itself yields an empty string: there is nothing honest to paint.
fn elide(fonts: &FontSet, text: &str, px: f32, max_width: f32) -> String {
    if fonts.ui_width(text, px) <= max_width {
        return text.to_string();
    }
    let ellipsis_width = fonts.ui_width("…", px);
    let budget = max_width - ellipsis_width;
    if budget <= 0.0 {
        return String::new();
    }
    // Walk whole characters so a multi-byte glyph is never split, and stop
    // before the character that would push the ellipsis past the clip.
    let mut shortened = String::with_capacity(text.len());
    let mut width = 0.0;
    for ch in text.chars() {
        let mut utf8 = [0u8; 4];
        let advance = fonts.ui_width(ch.encode_utf8(&mut utf8), px);
        if width + advance > budget {
            break;
        }
        shortened.push(ch);
        width += advance;
    }
    shortened.push('…');
    shortened
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
    let shortened = elide(renderer.fonts(), text, px, max_width);
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
    let (face, glyph) = renderer.fonts().chrome_glyph(ch);
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
        for glyph in [
            crate::fonts::GLYPH_ADD,
            crate::fonts::GLYPH_CHEVRON,
            crate::fonts::GLYPH_CLOSE,
            crate::fonts::GLYPH_MINIMIZE,
            crate::fonts::GLYPH_MAXIMIZE,
            crate::fonts::GLYPH_RESTORE,
        ] {
            let ch = glyph.chars().next().expect("a control glyph is one char");
            let (face, id, advance) = glyph_for(&renderer, ch, UI_SIZE_PX);
            let (font, _index) = renderer.fonts_face(face);
            assert_ne!(id.0, 0, "the resolved glyph for {ch:?} must exist");
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

    #[test]
    fn elide_ends_with_an_ellipsis_only_when_it_has_to() {
        let renderer = crate::render::tests::default_renderer();
        let fonts = renderer.fonts();

        let whole = elide(fonts, "Tab", UI_SIZE_PX, 100.0);
        assert_eq!(
            whole, "Tab",
            "a title that fits must come back untouched, with no ellipsis"
        );

        let clipped = elide(
            fonts,
            "a very long tab title that cannot fit",
            UI_SIZE_PX,
            40.0,
        );
        assert!(
            clipped.ends_with('…'),
            "a shortened title must end with the cut mark, got {clipped:?}"
        );
        let width = fonts.ui_width(&clipped, UI_SIZE_PX);
        assert!(
            width <= 40.0 + 0.01,
            "the shortened title must fit its budget (width {width})"
        );
    }

    #[test]
    fn elide_gives_up_when_even_the_ellipsis_does_not_fit() {
        let renderer = crate::render::tests::default_renderer();
        let fonts = renderer.fonts();
        let ellipsis_width = fonts.ui_width("…", UI_SIZE_PX);
        let narrow = ellipsis_width / 2.0;
        assert_eq!(
            elide(fonts, "some title", UI_SIZE_PX, narrow),
            "",
            "a clip narrower than the ellipsis must yield no text, not a panic or loop"
        );
    }
}
