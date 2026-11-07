//! Font discovery, loading, and metrics.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("no usable monospace font found")]
    NoFont,
    #[error("font data is invalid")]
    InvalidFont,
}

pub struct FontSet {
    pub font: FontVec,
    pub size_px: f32,
    pub cell_w: u32,
    // Grid row layout arrives in Task 6; until then only `cell_w` is needed.
    #[allow(dead_code)]
    pub cell_h: u32,
    pub ascent: f32,
    // No Phase 1 consumer; kept because `FontSet` publishes the metric pair
    // and the Task 6 grid layout will use it.
    #[allow(dead_code)]
    pub descent: f32,
}

impl FontSet {
    pub fn load(family: &str, size_px: f32, line_height: f32) -> Result<Self, FontError> {
        let mut database = Database::new();
        database.load_system_fonts();
        let requested = [Family::Name(family)];
        let monospace = [Family::Monospace];
        let mut query = Query {
            families: &requested,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let id = match database.query(&query) {
            Some(id) => id,
            None => {
                tracing::warn!("font family {family:?} not found; using the system monospace font");
                query.families = &monospace;
                database.query(&query).ok_or(FontError::NoFont)?
            }
        };
        let (data, index) = database
            .with_face_data(id, |data, index| (data.to_vec(), index))
            .ok_or(FontError::NoFont)?;
        let font =
            FontVec::try_from_vec_and_index(data, index).map_err(|_| FontError::InvalidFont)?;

        let scale = PxScale::from(size_px);
        let scaled = font.as_scaled(scale);
        let ascent = scaled.ascent();
        let descent = scaled.descent();
        let mut advance = scaled.h_advance(font.glyph_id('M'));
        if advance <= 0.0 {
            advance = scaled.h_advance(font.glyph_id('0'));
        }
        let cell_w = advance.ceil().max(1.0) as u32;
        let cell_h = ((ascent - descent) * line_height).ceil().max(1.0) as u32;

        Ok(Self {
            font,
            size_px,
            cell_w,
            cell_h,
            ascent,
            descent,
        })
    }

    pub fn glyph(&self, ch: char) -> Option<GlyphId> {
        let glyph = self.font.glyph_id(ch);
        (glyph.0 != 0).then_some(glyph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_font_with_positive_metrics() {
        let font = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        assert!(font.cell_w > 0);
        assert!(font.cell_h > 0);
        assert!(font.ascent > 0.0);
        assert!(font.descent < 0.0);
        assert!(font.glyph('A').is_some());
        assert!(font.glyph('M').is_some());
    }
}
