//! Font discovery, loading, and metrics.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("no usable monospace font found")]
    NoFont,
    #[error("font data is invalid")]
    InvalidFont,
}

/// Families tried, in order, when the configured Arabic family is unavailable.
///
/// `Segoe UI` is last on purpose: it covers Arabic letters but does not ligate
/// lam-alef, so it is only usable for isolated glyphs. Faces that fail the
/// lam-alef check are skipped anyway.
const ARABIC_FALLBACKS: &[&str] = &[
    "Arial",
    "Tahoma",
    "Noto Naskh Arabic",
    "Noto Sans Arabic",
    "DejaVu Sans",
    "FreeSerif",
    "Segoe UI",
];

/// Families tried, in order, to cover glyphs the primary face lacks (icons and
/// symbols such as `✕` and `❯`). A face is only used if it maps one of
/// [`SYMBOL_PROBES`], so text-only faces are skipped.
const SYMBOL_FALLBACKS: &[&str] = &[
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Segoe UI",
    "Cascadia Mono",
    "Cascadia Code",
    "MS Gothic",
    "DejaVu Sans",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Symbola",
];

const SYMBOL_PROBES: &[char] = &['✕', '●', '❯', '\u{E0B0}', '\u{E000}'];

pub struct FontFace {
    pub font: FontVec,
    pub data: Vec<u8>,
    pub index: u32,
    pub family: String,
}

pub struct FontSet {
    pub font: FontVec,
    pub data: Vec<u8>,
    pub index: u32,
    pub size_px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: f32,
    // Published metric (Phase 0 plan, Task 5); the renderer only needs ascent.
    #[allow(dead_code)]
    pub descent: f32,
    pub arabic: Option<FontFace>,
    pub symbols: Vec<FontFace>,
}

impl FontSet {
    pub fn load(
        family: &str,
        arabic_family: &str,
        size_px: f32,
        line_height: f32,
    ) -> Result<Self, FontError> {
        let mut database = Database::new();
        database.load_system_fonts();
        let (data, index) = load_primary(&database, family)?;
        let font = FontVec::try_from_vec_and_index(data.clone(), index)
            .map_err(|_| FontError::InvalidFont)?;

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

        let arabic = load_arabic(&database, arabic_family);
        let symbols = load_symbols(
            &database,
            family,
            arabic.as_ref().map(|face| face.family.as_str()),
        );
        Ok(Self {
            font,
            data,
            index,
            size_px,
            cell_w,
            cell_h,
            ascent,
            descent,
            arabic,
            symbols,
        })
    }

    pub fn has_arabic(&self) -> bool {
        self.arabic.is_some()
    }

    /// Font data for a face id: `0` primary, `1` Arabic, `2..` symbol faces.
    /// Unknown ids fall back to the primary face.
    pub fn font_for_id(&self, id: u8) -> (&FontVec, &[u8], u32) {
        match id {
            0 => (&self.font, &self.data, self.index),
            1 => self
                .arabic
                .as_ref()
                .map(|face| (&face.font, face.data.as_slice(), face.index))
                .unwrap_or((&self.font, &self.data, self.index)),
            id => self
                .symbols
                .get(usize::from(id - 2))
                .map(|face| (&face.font, face.data.as_slice(), face.index))
                .unwrap_or((&self.font, &self.data, self.index)),
        }
    }

    pub fn font_for(&self, arabic: bool) -> (&FontVec, &[u8], u32) {
        self.font_for_id(u8::from(arabic))
    }

    /// The first symbol face that maps the first character of `text`, with the
    /// face id (`2..`) and the resolved glyph.
    pub fn symbol_for(&self, text: &str) -> Option<(u8, ab_glyph::GlyphId)> {
        let ch = text.chars().next()?;
        self.symbols.iter().enumerate().find_map(|(index, face)| {
            let glyph = face.font.glyph_id(ch);
            (glyph.0 != 0).then_some(((index + 2) as u8, glyph))
        })
    }
}

fn load_primary(database: &Database, family: &str) -> Result<(Vec<u8>, u32), FontError> {
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
    database
        .with_face_data(id, |data, index| (data.to_vec(), index))
        .ok_or(FontError::NoFont)
}

fn load_symbols(database: &Database, primary: &str, arabic: Option<&str>) -> Vec<FontFace> {
    let mut symbols = Vec::new();
    for family in SYMBOL_FALLBACKS {
        if primary.eq_ignore_ascii_case(family)
            || arabic.is_some_and(|name| name.eq_ignore_ascii_case(family))
        {
            continue;
        }
        let query = Query {
            families: &[Family::Name(family)],
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let Some((data, index)) = database.with_face_data(id, |data, index| (data.to_vec(), index))
        else {
            continue;
        };
        let Ok(font) = FontVec::try_from_vec_and_index(data.clone(), index) else {
            continue;
        };
        if !SYMBOL_PROBES.iter().any(|ch| font.glyph_id(*ch).0 != 0) {
            continue;
        }
        tracing::debug!(family = *family, "loaded symbol fallback face");
        symbols.push(FontFace {
            font,
            data,
            index,
            family: (*family).to_string(),
        });
        if symbols.len() == 6 {
            break;
        }
    }
    symbols
}

fn load_arabic(database: &Database, configured: &str) -> Option<FontFace> {
    let candidates = std::iter::once(configured).chain(ARABIC_FALLBACKS.iter().copied());
    for family in candidates {
        if family.is_empty() {
            continue;
        }
        let families = [Family::Name(family)];
        let query = Query {
            families: &families,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let Some((data, index)) = database.with_face_data(id, |data, index| (data.to_vec(), index))
        else {
            continue;
        };
        if !can_render_arabic(&data, index) {
            continue;
        }
        let Ok(font) = FontVec::try_from_vec_and_index(data.clone(), index) else {
            continue;
        };
        if family != configured {
            tracing::warn!(
                requested = configured,
                using = family,
                "configured Arabic font unavailable; using a system fallback"
            );
        }
        let face = FontFace {
            font,
            data,
            index,
            family: family.to_string(),
        };
        tracing::debug!(family = %face.family, "Arabic face loaded");
        return Some(face);
    }
    tracing::warn!(
        requested = configured,
        "no Arabic-capable system font found; Arabic text uses the primary face"
    );
    None
}

/// Whether a face can render connected Arabic text: it must map the letters
/// and shape `لا` into a single lam-alef ligature. The ligature check keeps
/// faces such as `Segoe UI` that cover the letters but render lam-alef as two
/// disconnected glyphs.
fn can_render_arabic(data: &[u8], index: u32) -> bool {
    has_arabic_coverage(data, index) && ligates_lam_alef(data, index)
}

fn has_arabic_coverage(data: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(data, index) else {
        return false;
    };
    ['ا', 'ل', 'م', 'ب', 'س']
        .iter()
        .all(|ch| face.glyph_index(*ch).is_some())
}

fn ligates_lam_alef(data: &[u8], index: u32) -> bool {
    let Some(face) = rustybuzz::Face::from_slice(data, index) else {
        return false;
    };
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str("لا");
    buffer.guess_segment_properties();
    rustybuzz::shape(&face, &[], buffer).len() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font;

    fn load(family: &str) -> Option<FontSet> {
        FontSet::load(family, "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .ok()
    }

    #[test]
    fn loads_a_font_with_positive_metrics() {
        let font = load("Consolas").expect("a monospace font must exist");
        assert!(font.cell_w > 0);
        assert!(font.cell_h > 0);
        assert!(font.ascent > 0.0);
        assert!(font.descent < 0.0);
        assert!(font.font.glyph_id('A').0 != 0);
        assert!(font.font.glyph_id('M').0 != 0);
    }

    #[test]
    fn missing_configured_arabic_family_still_loads() {
        let font = load("Consolas").expect("a monospace font must exist");
        assert_eq!(font.font_for(false).2, font.index);
        assert!(font.size_px > 0.0);
    }

    #[test]
    fn arabic_face_covers_arabic_letters() {
        let Some(font) = load("Consolas") else {
            eprintln!("skipping: no monospace font installed");
            return;
        };
        let Some(face) = font.arabic.as_ref() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert!(face.font.glyph_id('م').0 != 0);
        assert!(face.font.glyph_id('ل').0 != 0);
        assert!(face.font.glyph_id('ا').0 != 0);
        assert!(!face.family.is_empty());
        assert!(font.has_arabic());
    }

    #[test]
    fn font_for_id_maps_primary_arabic_and_symbol_faces() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        assert!(
            std::ptr::eq(fonts.font_for_id(0).0, &fonts.font),
            "id 0 must be the primary face"
        );
        assert_eq!(fonts.font_for_id(0).2, fonts.index);

        match &fonts.arabic {
            Some(arabic) => {
                let (font, _, index) = fonts.font_for_id(1);
                assert!(
                    std::ptr::eq(font, &arabic.font),
                    "id 1 must be the Arabic face"
                );
                assert_eq!(index, arabic.index);
            }
            None => assert!(
                std::ptr::eq(fonts.font_for_id(1).0, &fonts.font),
                "id 1 must fall back to the primary face when Arabic is absent"
            ),
        }

        for (offset, face) in fonts.symbols.iter().enumerate() {
            let id = u8::try_from(offset + 2).expect("symbol face id");
            let (font, _, index) = fonts.font_for_id(id);
            assert!(
                std::ptr::eq(font, &face.font),
                "id {id} must be a symbol face"
            );
            assert_eq!(index, face.index);
        }

        let unknown = u8::try_from(fonts.symbols.len() + 3).expect("unknown id");
        assert!(
            std::ptr::eq(fonts.font_for_id(unknown).0, &fonts.font),
            "an unknown id must fall back to the primary face"
        );
    }

    #[test]
    fn symbol_fallback_covers_a_missing_glyph() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        if fonts.symbols.is_empty() {
            eprintln!("skipping: no symbol fallback font installed");
            return;
        }
        let (face, glyph) = fonts
            .symbol_for("✕")
            .expect("symbol fallback should cover U+2715");
        assert!(face >= 2, "symbol faces start at index 2, got {face}");
        assert_ne!(glyph.0, 0, "fallback must resolve a real glyph");
    }

    #[test]
    fn symbol_for_returns_none_for_an_uncovered_character() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        assert!(fonts.symbol_for("\u{10FFFF}").is_none());
    }

    #[test]
    fn unknown_arabic_family_uses_the_fallback_chain() {
        let fonts = FontSet::load("Consolas", "Definitely Not A Font 12345", 14.0, 1.2)
            .or_else(|_| {
                FontSet::load("DejaVu Sans Mono", "Definitely Not A Font 12345", 14.0, 1.2)
            })
            .expect("a monospace font must exist");
        if let Some(face) = &fonts.arabic {
            assert_ne!(face.family, "Definitely Not A Font 12345");
        }
    }
}
