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

/// Bundled OFL Arabic face, loaded before system fonts so it is always
/// available when explicitly requested (`arabic_family = "Kawkab Mono"`).
pub(crate) const BUNDLED_ARABIC: &[u8] = include_bytes!("../assets/fonts/KawkabMono-Regular.ttf");

/// Families tried, in order, when the configured Arabic family is unavailable.
///
/// The first face that renders connected Arabic wins. Segoe UI leads because
/// it is the face PowerShell uses for Arabic and its connected forms are much
/// closer to the primary cell than Arial's. Faces that fail the joining check
/// are skipped. Kawkab Mono stays last: it is bundled and always available
/// when requested, but its narrow ink does not suit the default grid.
const ARABIC_FALLBACKS: &[&str] = &[
    "Segoe UI",
    "Tahoma",
    "Arial",
    "Noto Naskh Arabic",
    "Noto Sans Arabic",
    "DejaVu Sans",
    "FreeSerif",
    "Kawkab Mono",
];

/// Families tried, in order, to cover glyphs the primary face lacks (icons and
/// symbols such as `✕` and `❯`). A face is only used if it maps one of
/// [`SYMBOL_PROBES`], so text-only faces are skipped.
///
/// The first few matches are enough in practice, and every loaded face keeps
/// its whole font file in memory (`MS Gothic` alone is about 9 MB), so the
/// search stops early instead of collecting everything that matches.
const SYMBOL_FACE_LIMIT: usize = 2;
const SYMBOL_FALLBACKS: &[&str] = &[
    // Color emoji faces are deliberately absent: they are about 13 MB each and
    // the renderer masks glyphs in grayscale, so their color layers cannot be
    // used anyway.
    "Segoe UI Symbol",
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

/// Proportional families tried in order for window chrome (tab titles, menu
/// items).
///
/// Windows Terminal's tab header sets no family of its own, so it inherits the
/// WinUI theme font: `Segoe UI Variable Text` on Windows 11. Measured at 12 px,
/// the classic `Segoe UI` here drew 1 px stems where WT's are 2 px, which is why
/// the title read thinner than the real shell's.
const UI_FAMILIES: &[&str] = &["Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI"];

/// Semibold proportional families tried in order for the menu's default row.
///
/// Windows Terminal draws the row of the profile that `+` would open semibold
/// (measured on 1.24: 2–3 px stems against 1–2 px in the rows below), and the
/// regular UI face cannot fake that weight. The variable family ships a single
/// weight, so the lookup falls through to the classic family's semibold cut.
const UI_SEMIBOLD_FAMILIES: &[&str] = &[
    "Segoe UI Variable Text Semibold",
    "Segoe UI Semibold",
    "Segoe UI",
];

/// System icon families tried in order for the chrome's window controls.
///
/// Windows Terminal draws its caption buttons, the new-tab `+` and the tab's
/// `✕` from `Segoe MDL2 Assets`, so the same face gives the same shapes for
/// free. Fluent Icons is the Windows 11 successor and maps the same private-use
/// codepoints, so it is preferred when installed.
const ICON_FAMILIES: &[&str] = &["Segoe Fluent Icons", "Segoe MDL2 Assets"];

/// A family only counts as the icon face if it maps the glyphs the chrome
/// draws, so an unrelated face that happens to share a name is skipped.
const ICON_PROBES: &[char] = &['\u{E921}', '\u{E922}', '\u{E8BB}', '\u{E710}'];

/// Reserved glyph-cache face id for the proportional UI face.
///
/// Symbol faces start at `2`, so the top of the range stays unambiguous.
pub const UI_FONT_ID: u8 = u8::MAX;

/// Reserved glyph-cache face id for the system icon face.
///
/// Distinct from [`UI_FONT_ID`] because the icon face is a symbol font: its
/// `+` and letters are `.notdef`, so only the chrome's control codepoints may
/// be resolved through it.
pub const ICON_FONT_ID: u8 = u8::MAX - 1;

/// Reserved glyph-cache face id for the semibold proportional face.
///
/// Used only by the menu's default-profile row, the one Windows Terminal draws
/// semibold; every other chrome string stays on [`UI_FONT_ID`].
pub const SEMIBOLD_FONT_ID: u8 = u8::MAX - 2;

/// Codepoints the chrome draws, all from the Windows icon face. Windows
/// Terminal uses these exact glyphs, which is why its controls are not the
/// text characters `—`, `□` and `✕` drawn here before.
pub(crate) const GLYPH_MINIMIZE: &str = "\u{E921}";
pub(crate) const GLYPH_MAXIMIZE: &str = "\u{E922}";
pub(crate) const GLYPH_RESTORE: &str = "\u{E923}";
pub(crate) const GLYPH_CLOSE: &str = "\u{E8BB}";
pub(crate) const GLYPH_ADD: &str = "\u{E710}";
pub(crate) const GLYPH_CHEVRON: &str = "\u{E70D}";

/// Icon-face codepoints for the dropdown's fixed rows.
///
/// Windows Terminal's new-tab flyout shows a gear for Settings, a lightning
/// bolt for the command palette and a question mark for About; these are the
/// same glyphs from the same face.
pub(crate) const GLYPH_SETTINGS: &str = "\u{E713}";
pub(crate) const GLYPH_COMMAND_PALETTE: &str = "\u{E945}";
pub(crate) const GLYPH_HELP: &str = "\u{E897}";

/// Icon-face em size for the tab's own `✕`, in logical pixels.
///
/// Measured on Windows Terminal 1.24 at 100 %: the tab's `✕` ink box is 9x9 px,
/// smaller than the 10 px caption controls, so the two cannot share a size.
pub(crate) const TAB_CLOSE_ICON_PX: f32 = 9.0;

/// Icon-face em size for the window controls and the strip's dropdown, in
/// logical pixels.
///
/// Measured on Windows Terminal 1.24 at 100 %: the minimise ink is a single
/// 10 px row, the maximise outline and caption `✕` are 10x10, and the `⌄` ink is
/// 8 px wide. Drawn at the 14 px text size ours came out 14 px wide in every
/// case, so the icon face needs its own, smaller em. The `+` keeps the text
/// size: WT's `+` ink is 11 px wide, which is what 14 px produces.
pub(crate) const CONTROL_ICON_PX: f32 = 10.0;

/// Chrome text size in logical pixels.
///
/// 14 px is Windows Terminal's tab text at 100 %, inside its 34 px strip.
pub(crate) const UI_SIZE_PX: f32 = 14.0;

/// Converts an em size in pixels into the pixel-height scale `ab_glyph`
/// expects. `PxScale` measures the font's ascent-to-descent height, not its
/// em square, while terminals (and the shaper used here) size text by the em
/// square. Skipping this conversion makes every glyph about 15% too small and
/// leaves the cell grid narrower than the glyph advances.
pub(crate) fn em_px_scale(font: &FontVec, em_px: f32) -> PxScale {
    let units = font
        .units_per_em()
        .unwrap_or_else(|| font.height_unscaled());
    PxScale::from(em_px * font.height_unscaled() / units)
}

pub struct FontFace {
    pub font: FontVec,
    pub index: u32,
    pub family: String,
    /// Extra scale applied to this face so its ink matches the primary face.
    pub scale: f32,
}

pub struct FontSet {
    pub font: FontVec,
    pub index: u32,
    pub size_px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: f32,
    // Published metric (Phase 0 plan, Task 5); the renderer only needs ascent.
    #[allow(dead_code)]
    pub descent: f32,
    pub arabic: Option<FontFace>,
    /// Proportional face for window chrome; absent on systems without it, in
    /// which case chrome text falls back to the primary monospace face.
    pub ui: Option<FontFace>,
    /// System icon face for the window controls; absent on systems without it,
    /// in which case those glyphs fall back to symbol faces.
    pub icons: Option<FontFace>,
    /// Semibold proportional face for the menu's default-profile row; absent on
    /// systems without it, in which case that row falls back to regular weight.
    pub semibold: Option<FontFace>,
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
        database.load_font_data(BUNDLED_ARABIC.to_vec());
        database.load_system_fonts();
        let (data, index) = load_primary(&database, family)?;
        // `FontVec` owns the bytes, so no second copy is kept: every face is a
        // multi-megabyte font file and the process budget is tight.
        let font =
            FontVec::try_from_vec_and_index(data, index).map_err(|_| FontError::InvalidFont)?;

        let scale = em_px_scale(&font, size_px);
        let scaled = font.as_scaled(scale);
        let ascent = scaled.ascent();
        let descent = scaled.descent();
        let mut advance = scaled.h_advance(font.glyph_id('M'));
        if advance <= 0.0 {
            advance = scaled.h_advance(font.glyph_id('0'));
        }
        let cell_w = advance.round().max(1.0) as u32;
        let cell_h = ((ascent - descent) * line_height).ceil().max(1.0) as u32;

        let arabic = load_arabic(&database, arabic_family);
        let ui = load_ui(&database);
        let icons = load_icons(&database);
        let semibold = load_semibold(&database);
        let symbols = load_symbols(
            &database,
            family,
            arabic.as_ref().map(|face| face.family.as_str()),
        );
        Ok(Self {
            font,
            index,
            size_px,
            cell_w,
            cell_h,
            ascent,
            descent,
            arabic,
            ui,
            icons,
            semibold,
            symbols,
        })
    }

    pub fn has_arabic(&self) -> bool {
        self.arabic.is_some()
    }

    /// The ascent used to place the shared text baseline. Uses the taller of
    /// the primary and Arabic faces so Arabic and Latin sit on the same line.
    pub fn line_ascent(&self) -> f32 {
        let arabic = self.arabic.as_ref().map_or(0.0, |face| {
            face.font
                .as_scaled(em_px_scale(&face.font, self.size_px * face.scale))
                .ascent()
        });
        self.ascent.max(arabic)
    }

    /// Font data for a face id: `0` primary, `1` Arabic, `2..` symbol faces,
    /// [`UI_FONT_ID`] the proportional chrome face, [`ICON_FONT_ID`] the
    /// window-control icon face, [`SEMIBOLD_FONT_ID`] its semibold weight.
    /// Unknown ids and absent faces fall back to the primary face.
    pub fn font_for_id(&self, id: u8) -> (&FontVec, u32) {
        match id {
            0 => (&self.font, self.index),
            1 => self
                .arabic
                .as_ref()
                .map(|face| (&face.font, face.index))
                .unwrap_or((&self.font, self.index)),
            UI_FONT_ID => self
                .ui
                .as_ref()
                .map(|face| (&face.font, face.index))
                .unwrap_or((&self.font, self.index)),
            ICON_FONT_ID => self
                .icons
                .as_ref()
                .map(|face| (&face.font, face.index))
                .unwrap_or((&self.font, self.index)),
            SEMIBOLD_FONT_ID => self
                .semibold
                .as_ref()
                .or(self.ui.as_ref())
                .map(|face| (&face.font, face.index))
                .unwrap_or((&self.font, self.index)),
            id => self
                .symbols
                .get(usize::from(id - 2))
                .map(|face| (&face.font, face.index))
                .unwrap_or((&self.font, self.index)),
        }
    }

    pub fn font_for(&self, arabic: bool) -> (&FontVec, u32) {
        self.font_for_id(u8::from(arabic))
    }

    /// The face and glyph the chrome painter uses for `ch`.
    ///
    /// The icon face wins when it maps `ch`: the window controls, `+` and `✕`
    /// are drawn from it so the strip shows the same shapes Windows Terminal
    /// does. It has no letters or digits, so ordinary chrome text falls
    /// through. The UI face does not map every symbol the chrome draws, and
    /// painting those with it would produce `.notdef` boxes. Falling back to
    /// the same symbol faces the grid uses keeps measurement and ink on one
    /// face, so a centred glyph is actually centred.
    pub fn chrome_glyph(&self, ch: char) -> (u8, ab_glyph::GlyphId) {
        if let Some(icons) = self.icons.as_ref() {
            let glyph = icons.font.glyph_id(ch);
            if glyph.0 != 0 {
                return (ICON_FONT_ID, glyph);
            }
        }
        let (ui_font, _index) = self.font_for_id(UI_FONT_ID);
        let glyph = ui_font.glyph_id(ch);
        if glyph.0 != 0 {
            return (UI_FONT_ID, glyph);
        }
        self.symbol_for_char(ch).unwrap_or((UI_FONT_ID, glyph))
    }

    /// The glyph `face` maps `ch` to, or `None` when that face has no outline
    /// for it: glyph 0 is `.notdef`, which would paint a box.
    pub fn glyph_in(&self, face: u8, ch: char) -> Option<ab_glyph::GlyphId> {
        let (font, _index) = self.font_for_id(face);
        let glyph = font.glyph_id(ch);
        (glyph.0 != 0).then_some(glyph)
    }

    /// Width of `text` in the faces the chrome painter uses, at `px`.
    ///
    /// This follows [`FontSet::chrome_glyph`]'s per-character resolution, so an
    /// advance measured here is the advance the painter moves the pen by: a
    /// symbol the UI face lacks takes its fallback face's width, not the UI
    /// `.notdef` box. `px` is an em size, the unit the glyph cache rasterizes
    /// at, and already physical: the caller scales the logical `UI_SIZE_PX`
    /// once with the window's scale factor.
    pub fn ui_width(&self, text: &str, px: f32) -> f32 {
        self.width_in(UI_FONT_ID, text, px)
    }

    /// Width of `text` at `px`, measured on `face` and falling back per
    /// character to [`FontSet::chrome_glyph`] so a symbol still measures on the
    /// face that paints it.
    pub fn width_in(&self, face: u8, text: &str, px: f32) -> f32 {
        text.chars()
            .map(|ch| {
                let (face, glyph) = match self.glyph_in(face, ch) {
                    Some(glyph) => (face, glyph),
                    None => self.chrome_glyph(ch),
                };
                let (font, _index) = self.font_for_id(face);
                font.as_scaled(em_px_scale(font, px)).h_advance(glyph)
            })
            .sum()
    }

    /// Height of the UI face's line box, ascent to descent, at `px`.
    ///
    /// `draw_text` anchors its `y` to the top of this box, not to the em
    /// square, so centring a glyph that has no measurable ink must use it.
    pub fn ui_line_height(&self, px: f32) -> f32 {
        let (font, _index) = self.font_for_id(UI_FONT_ID);
        let scaled = font.as_scaled(em_px_scale(font, px));
        scaled.ascent() - scaled.descent()
    }

    /// The ink box of `text` at `px`, as `(left, top, right, bottom)` in
    /// physical pixels from the text origin: `(0, 0)` is the pen start on the
    /// line box's top edge.
    ///
    /// Chrome glyphs are centred by their ink because they come from different
    /// fallback faces and sit at different heights inside the same line box.
    /// `None` when no character in `text` has a drawable outline.
    pub fn chrome_ink_bounds(&self, text: &str, px: f32) -> Option<(f32, f32, f32, f32)> {
        self.ink_bounds_in(UI_FONT_ID, text, px)
    }

    /// The ink box of `text` at `px` on `face`, in the same frame as
    /// [`FontSet::chrome_ink_bounds`].
    ///
    /// The baseline comes from the UI face rather than from `face`: the painter
    /// anchors every chrome string to the UI line box, so measuring on any other
    /// ascent would centre a semibold row a pixel off its neighbours.
    pub fn ink_bounds_in(&self, face: u8, text: &str, px: f32) -> Option<(f32, f32, f32, f32)> {
        // The cache rasterizes at the rounded em size; measuring at the same
        // size keeps this box on the outline the blit paints.
        let size = px.round();
        let (ui_font, _index) = self.font_for_id(UI_FONT_ID);
        let ascent = ui_font.as_scaled(em_px_scale(ui_font, size)).ascent();
        let mut pen = 0.0;
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for ch in text.chars() {
            let (face, glyph) = match self.glyph_in(face, ch) {
                Some(glyph) => (face, glyph),
                None => self.chrome_glyph(ch),
            };
            let (font, _index) = self.font_for_id(face);
            let scale = em_px_scale(font, size);
            let advance = font.as_scaled(scale).h_advance(glyph);
            if let Some(outline) = font.outline_glyph(glyph.with_scale(scale)) {
                let ink = outline.px_bounds();
                let next = (
                    pen + ink.min.x,
                    ascent + ink.min.y,
                    pen + ink.max.x,
                    ascent + ink.max.y,
                );
                bounds = Some(match bounds {
                    Some((left, top, right, bottom)) => (
                        left.min(next.0),
                        top.min(next.1),
                        right.max(next.2),
                        bottom.max(next.3),
                    ),
                    None => next,
                });
            }
            pen += advance;
        }
        bounds
    }

    /// The first symbol face that maps `ch`, with the face id (`2..`) and the
    /// resolved glyph.
    pub fn symbol_for_char(&self, ch: char) -> Option<(u8, ab_glyph::GlyphId)> {
        self.symbols.iter().enumerate().find_map(|(index, face)| {
            let glyph = face.font.glyph_id(ch);
            (glyph.0 != 0).then_some(((index + 2) as u8, glyph))
        })
    }

    /// The first symbol face that maps the first character of `text`, with the
    /// face id (`2..`) and the resolved glyph.
    pub fn symbol_for(&self, text: &str) -> Option<(u8, ab_glyph::GlyphId)> {
        self.symbol_for_char(text.chars().next()?)
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

/// Loads the proportional window-chrome face, or `None` when no family is
/// installed so chrome text falls back to the primary monospace face.
fn load_ui(database: &Database) -> Option<FontFace> {
    load_proportional(
        database,
        UI_FAMILIES,
        Weight::NORMAL,
        "proportional UI face",
    )
}

/// Loads the semibold proportional face for the menu's default row, or `None`
/// when no semibold family is installed so that row stays regular.
fn load_semibold(database: &Database) -> Option<FontFace> {
    load_proportional(
        database,
        UI_SEMIBOLD_FAMILIES,
        Weight::SEMIBOLD,
        "semibold UI face",
    )
}

/// Parses the first installed family in `families` into a face of at least
/// `weight`.
///
/// Every proportional face is wanted for the same reason — chrome text should
/// look like the WinUI theme font — so one loader keeps the fallback behaviour
/// identical for the regular and the semibold weight.
fn load_proportional(
    database: &Database,
    families: &[&str],
    weight: Weight,
    what: &str,
) -> Option<FontFace> {
    for family in families {
        let list = [Family::Name(family)];
        let query = Query {
            families: &list,
            weight,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        // A family without the requested cut answers with its closest weight,
        // which would win the lookup with a regular face. Skipping it lets the
        // semibold search reach the family that actually has one.
        if database
            .face(id)
            .is_some_and(|face| face.weight.0 < weight.0)
        {
            continue;
        }
        let Some((data, index)) = database.with_face_data(id, |data, index| (data.to_vec(), index))
        else {
            continue;
        };
        let Ok(font) = FontVec::try_from_vec_and_index(data, index) else {
            continue;
        };
        tracing::debug!(family = *family, face = what, "proportional face loaded");
        return Some(FontFace {
            font,
            index,
            family: (*family).to_string(),
            scale: 1.0,
        });
    }
    tracing::warn!(
        families = ?families,
        face = what,
        "no proportional face found; chrome text uses the monospace face"
    );
    None
}

/// Loads the system icon face the window controls are drawn from, or `None`
/// when neither family is installed so those glyphs fall back to symbols.
fn load_icons(database: &Database) -> Option<FontFace> {
    for family in ICON_FAMILIES {
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
        let Ok(font) = FontVec::try_from_vec_and_index(data, index) else {
            continue;
        };
        // A family of the same name can exist without the control glyphs;
        // probing the codepoints keeps an empty face from winning.
        if !ICON_PROBES.iter().any(|ch| font.glyph_id(*ch).0 != 0) {
            continue;
        }
        tracing::debug!(family = *family, "icon face loaded");
        return Some(FontFace {
            font,
            index,
            family: (*family).to_string(),
            scale: 1.0,
        });
    }
    None
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
        let Ok(font) = FontVec::try_from_vec_and_index(data, index) else {
            continue;
        };
        if !SYMBOL_PROBES.iter().any(|ch| font.glyph_id(*ch).0 != 0) {
            continue;
        }
        tracing::debug!(family = *family, "loaded symbol fallback face");
        symbols.push(FontFace {
            font,
            index,
            family: (*family).to_string(),
            scale: 1.0,
        });
        if symbols.len() == SYMBOL_FACE_LIMIT {
            break;
        }
    }
    symbols
}

fn load_arabic(database: &Database, configured: &str) -> Option<FontFace> {
    let candidates: Vec<&str> = if configured.is_empty() {
        ARABIC_FALLBACKS.to_vec()
    } else {
        std::iter::once(configured)
            .chain(ARABIC_FALLBACKS.iter().copied())
            .collect()
    };
    for family in candidates {
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
        let Ok(font) = FontVec::try_from_vec_and_index(data, index) else {
            continue;
        };
        if configured.is_empty() || family == configured {
            tracing::debug!(family, "Arabic face loaded");
        } else {
            tracing::warn!(
                requested = configured,
                using = family,
                "configured Arabic font unavailable; using a system fallback"
            );
        }
        return Some(FontFace {
            font,
            index,
            family: family.to_string(),
            scale: 1.0,
        });
    }
    tracing::warn!(
        requested = configured,
        "no Arabic-capable system font found; Arabic text uses the primary face"
    );
    None
}

/// Whether a face can render connected Arabic text: it must map the letters
/// and connect lam-alef, either as one ligature or as contextual forms.
fn can_render_arabic(data: &[u8], index: u32) -> bool {
    has_arabic_coverage(data, index) && ligates_lam_alef(data, index) && joins_arabic(data, index)
}

/// Rejects faces that expose Arabic letters but do not apply cursive joining:
/// shaping `مرحبا` must produce at least one contextual form that differs from
/// the isolated glyphs. Without this, text renders as disconnected letters.
fn joins_arabic(data: &[u8], index: u32) -> bool {
    let Some(face) = rustybuzz::Face::from_slice(data, index) else {
        return false;
    };
    let isolated: Vec<u32> = "مرحبا"
        .chars()
        .filter_map(|ch| face.glyph_index(ch))
        .map(|glyph| u32::from(glyph.0))
        .collect();
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str("مرحبا");
    buffer.set_direction(rustybuzz::Direction::RightToLeft);
    buffer.set_script(rustybuzz::script::ARABIC);
    let shaped = rustybuzz::shape(&face, &[], buffer);
    shaped
        .glyph_infos()
        .iter()
        .any(|info| !isolated.contains(&info.glyph_id))
}

fn has_arabic_coverage(data: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(data, index) else {
        return false;
    };
    ['ا', 'ل', 'م', 'ب', 'س']
        .iter()
        .all(|ch| face.glyph_index(*ch).is_some())
}

/// Some faces, including Segoe UI, connect lam-alef with two contextual
/// glyphs instead of a single ligature. Accept those as long as the shaped
/// glyphs differ from the isolated letters; a face that simply drops both
/// isolated letters side by side is still rejected.
fn ligates_lam_alef(data: &[u8], index: u32) -> bool {
    let Some(face) = rustybuzz::Face::from_slice(data, index) else {
        return false;
    };
    let Some(lam) = face.glyph_index('ل') else {
        return false;
    };
    let Some(alef) = face.glyph_index('ا') else {
        return false;
    };
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str("لا");
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(&face, &[], buffer);
    shaped.len() == 1
        || shaped
            .glyph_infos()
            .iter()
            .any(|info| info.glyph_id != u32::from(lam.0) && info.glyph_id != u32::from(alef.0))
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
    fn font_size_is_the_em_size() {
        let font = FontSet::load("Cascadia Mono", "Arial", 16.0, 1.0)
            .or_else(|_| FontSet::load("Consolas", "Arial", 16.0, 1.0))
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Arial", 16.0, 1.0))
            .expect("a monospace font must exist");
        let face = ttf_parser::Face::parse(font.font.as_slice(), font.index).expect("valid face");
        let units = f32::from(face.units_per_em());
        let advance = face
            .glyph_hor_advance(face.glyph_index('M').expect("primary face maps M"))
            .expect("M has an advance");
        let expected_w = (16.0 * f32::from(advance) / units).round() as u32;
        let expected_h = (16.0 * (f32::from(face.ascender()) - f32::from(face.descender())) / units)
            .ceil() as u32;
        assert_eq!(font.cell_w, expected_w, "cell width must use the em scale");
        assert_eq!(font.cell_h, expected_h, "cell height must use the em scale");
    }

    #[test]
    fn missing_configured_arabic_family_still_loads() {
        let font = load("Consolas").expect("a monospace font must exist");
        assert_eq!(font.font_for(false).1, font.index);
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
    fn auto_arabic_face_uses_natural_advances() {
        let font = FontSet::load("Consolas", "", 16.0, 1.0)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "", 16.0, 1.0))
            .expect("a monospace font must exist");
        let Some(face) = font.arabic.as_ref() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(
            face.scale, 1.0,
            "{} must render at its natural em size",
            face.family
        );
    }

    #[test]
    fn configured_arabic_family_is_still_respected() {
        let font = FontSet::load("Consolas", "Arial", 16.0, 1.0)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Arial", 16.0, 1.0))
            .expect("a monospace font must exist");
        let Some(face) = font.arabic.as_ref() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(face.family, "Arial");
        assert_eq!(face.scale, 1.0);
    }

    #[test]
    fn segoe_ui_connects_arabic_without_a_single_lam_alef_ligature() {
        let Ok(data) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") else {
            eprintln!("skipping: Segoe UI is not installed");
            return;
        };
        assert!(
            can_render_arabic(&data, 0),
            "Segoe UI must be accepted for its contextual Arabic glyphs"
        );
    }

    #[test]
    fn bundled_kawkab_is_available_when_requested() {
        let font = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        let face = font.arabic.as_ref().expect("bundled Kawkab must load");
        assert_eq!(face.family, "Kawkab Mono");
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
        assert_eq!(fonts.font_for_id(0).1, fonts.index);

        match &fonts.arabic {
            Some(arabic) => {
                let (font, index) = fonts.font_for_id(1);
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
            let (font, index) = fonts.font_for_id(id);
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

        match &fonts.ui {
            Some(ui) => {
                let (font, index) = fonts.font_for_id(UI_FONT_ID);
                assert!(
                    std::ptr::eq(font, &ui.font),
                    "the UI id must be the proportional face"
                );
                assert_eq!(index, ui.index);
            }
            None => assert!(
                std::ptr::eq(fonts.font_for_id(UI_FONT_ID).0, &fonts.font),
                "the UI id must fall back to the primary face when the face is absent"
            ),
        }
    }

    #[test]
    fn the_semibold_face_is_its_own_cache_id() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        match &fonts.semibold {
            Some(face) => {
                let (font, index) = fonts.font_for_id(SEMIBOLD_FONT_ID);
                assert!(
                    std::ptr::eq(font, &face.font),
                    "the semibold id must not reuse the regular UI face"
                );
                assert_eq!(index, face.index);
            }
            None => {
                let (fallback, _index) = fonts.font_for_id(SEMIBOLD_FONT_ID);
                let (ui, _index) = fonts.font_for_id(UI_FONT_ID);
                assert!(
                    std::ptr::eq(fallback, ui),
                    "without a semibold face the default row must fall back to the regular UI face"
                );
            }
        }
    }

    #[test]
    fn a_face_only_maps_the_characters_it_has_outlines_for() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        assert!(
            fonts.glyph_in(UI_FONT_ID, 'M').is_some(),
            "the UI face maps letters"
        );
        if fonts.icons.is_none() {
            eprintln!("skipping: no icon face installed");
            return;
        }
        assert!(
            fonts.glyph_in(ICON_FONT_ID, 'M').is_none(),
            "the icon face has no letters, so a menu label must never measure through it"
        );
        assert!(
            (fonts.width_in(ICON_FONT_ID, "M", UI_SIZE_PX) - fonts.ui_width("M", UI_SIZE_PX)).abs()
                < 0.01,
            "a character a face lacks falls back to the face that paints it"
        );
    }

    #[test]
    fn ui_width_measures_with_the_proportional_face() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        if fonts.ui.is_none() {
            eprintln!("skipping: no proportional UI face installed");
            return;
        }
        assert_eq!(fonts.ui_width("", UI_SIZE_PX), 0.0);
        assert!(
            fonts.ui_width("M", UI_SIZE_PX) > fonts.ui_width("i", UI_SIZE_PX),
            "a proportional face gives narrow letters narrower advances"
        );
    }

    #[test]
    fn ui_width_falls_back_to_the_primary_face() {
        let Some(mut fonts) = load("Consolas") else {
            return;
        };
        // Without the proportional face the chrome is painted with the primary
        // monospace face, so measurement must use that face's advances.
        fonts.ui = None;
        let (font, _index) = fonts.font_for_id(0);
        let advance = font
            .as_scaled(em_px_scale(font, UI_SIZE_PX))
            .h_advance(font.glyph_id('a'));
        assert!((fonts.ui_width("abc", UI_SIZE_PX) - 3.0 * advance).abs() < 0.01);
    }

    #[test]
    fn a_chrome_symbol_is_measured_on_the_face_that_paints_it() {
        let Some(fonts) = load("Consolas") else {
            return;
        };
        let mut checked = 0;
        for ch in ['✕', '⌄'] {
            let (face, glyph) = fonts.chrome_glyph(ch);
            if face == UI_FONT_ID {
                // This machine's UI face happens to map the symbol.
                continue;
            }
            let (font, _index) = fonts.font_for_id(face);
            let scaled = font.as_scaled(em_px_scale(font, UI_SIZE_PX));
            let advance = scaled.h_advance(glyph);
            let (ui_font, _index) = fonts.font_for_id(UI_FONT_ID);
            let notdef = ui_font
                .as_scaled(em_px_scale(ui_font, UI_SIZE_PX))
                .h_advance(ui_font.glyph_id(ch));
            let text = ch.to_string();
            assert!(
                (fonts.ui_width(&text, UI_SIZE_PX) - advance).abs() < 0.01,
                "{ch:?} must be measured on its fallback face"
            );
            assert!(
                (advance - notdef).abs() > 0.01,
                "the fallback advance must differ from the UI's .notdef"
            );
            checked += 1;
        }
        assert!(checked > 0, "no chrome symbol exercised the fallback path");
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
