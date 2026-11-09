//! Bidi-run shaping with rustybuzz and cluster-to-cell mapping.

use std::collections::HashMap;
use std::ops::Range;

use rustybuzz::{script, BufferClusterLevel, Direction, Face, UnicodeBuffer};

use crate::bidi::RowText;
use crate::visual::VisualMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub glyph_id: u32,
    pub cluster: usize,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    pub cells: Range<usize>,
    pub slots: Range<usize>,
    pub glyphs: Vec<Glyph>,
    /// The source text this cluster covers, used to pick a fallback face when
    /// the primary face has no glyph for it.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedRun {
    pub slots: Range<usize>,
    pub arabic: bool,
    pub units_per_em: i32,
    pub clusters: Vec<Cluster>,
}

pub struct Faces<'a> {
    pub primary: Face<'a>,
    pub arabic: Option<Face<'a>>,
}

impl<'a> Faces<'a> {
    pub fn new(primary: Face<'a>, arabic: Option<Face<'a>>) -> Self {
        Self { primary, arabic }
    }

    /// Borrows the loaded font bytes. The caller must keep the [`FontSet`]
    /// alive for as long as the returned faces are used.
    ///
    /// [`FontSet`]: crate::fonts::FontSet
    pub fn from_fonts(fonts: &'a crate::fonts::FontSet) -> Option<Self> {
        let primary = Face::from_slice(&fonts.data, fonts.index)?;
        let arabic = if fonts.has_arabic() {
            fonts
                .arabic
                .as_ref()
                .and_then(|face| Face::from_slice(&face.data, face.index))
        } else {
            None
        };
        Some(Self::new(primary, arabic))
    }
}

/// Per-frame memo of rustybuzz output for repeated text segments.
///
/// The renderer clears this at the start of every frame. Font size is not part
/// of the key: rustybuzz reports font-unit metrics, so scaling happens at draw
/// time and cannot change the shaping result.
#[derive(Default)]
pub struct ShapeCache {
    entries: HashMap<ShapeKey, Shaped>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: String,
    rtl: bool,
    /// 0 = primary face, 1 = Arabic fallback face, 2.. = symbol fallback faces.
    font: u8,
}

struct Shaped {
    units_per_em: i32,
    /// Glyph clusters are relative to the start of the shaped text.
    glyphs: Vec<Glyph>,
}

impl ShapeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn shape(&mut self, text: &str, rtl: bool, font: u8, face: &Face<'_>) -> &Shaped {
        let key = ShapeKey {
            text: text.to_string(),
            rtl,
            font,
        };
        self.entries
            .entry(key)
            .or_insert_with(|| shape_text(text, rtl, face))
    }
}

fn shape_text(text: &str, rtl: bool, face: &Face<'_>) -> Shaped {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if rtl {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    });
    if contains_arabic(text) {
        buffer.set_script(script::ARABIC);
    }
    buffer.set_cluster_level(BufferClusterLevel::MonotoneGraphemes);
    let shaped = rustybuzz::shape(face, &[], buffer);

    let glyphs = shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, position)| Glyph {
            glyph_id: info.glyph_id,
            cluster: info.cluster as usize,
            x_advance: position.x_advance,
            x_offset: position.x_offset,
            y_offset: position.y_offset,
        })
        .collect();

    Shaped {
        units_per_em: face.units_per_em(),
        glyphs,
    }
}

fn is_arabic_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF
    )
}

fn contains_arabic(text: &str) -> bool {
    text.chars().any(is_arabic_char)
}

/// Fallback direction for identity maps (bidi disabled): use the first strong
/// character instead of a bidi level.
fn first_strong_is_rtl(text: &str) -> bool {
    for ch in text.chars() {
        if is_arabic_char(ch) {
            return true;
        }
        if ch.is_ascii_alphabetic() {
            return false;
        }
    }
    false
}

/// Splits a visual run where the script changes so Latin cells keep the
/// primary face even when bidi is disabled and the row is one identity run.
fn script_segments(row: &RowText, cells: Range<usize>) -> Vec<(Range<usize>, bool)> {
    let mut segments = Vec::new();
    let mut start = cells.start;
    let mut current: Option<bool> = None;

    for cell in cells.clone() {
        // A wide character's continuation owns no bytes; it belongs to the
        // segment of its base cell.
        if row.widths.get(cell).copied().unwrap_or(0) == 0 && cell > start {
            continue;
        }
        let arabic = contains_arabic(&row.text[row.cell_starts[cell]..row.cell_starts[cell + 1]]);
        match current {
            None => current = Some(arabic),
            Some(previous) if previous != arabic => {
                segments.push((start..cell, previous));
                start = cell;
                current = Some(arabic);
            }
            Some(_) => {}
        }
    }
    if let Some(arabic) = current {
        segments.push((start..cells.end, arabic));
    }
    segments
}

/// Shapes one row into visual runs.
///
/// `row` and `map` must describe the same row: `row.cell_starts.len()` is
/// `map.cols() + 1` and every run lies inside the row.
pub fn shape_row(
    row: &RowText,
    map: &VisualMap,
    faces: &Faces<'_>,
    cache: &mut ShapeCache,
) -> Vec<ShapedRun> {
    debug_assert_eq!(
        row.cell_starts.len(),
        map.cols() + 1,
        "row and map must describe the same row"
    );
    let mut out = Vec::new();

    for run in map.runs() {
        if run.slots.is_empty() {
            continue;
        }
        debug_assert!(run.cells.end <= map.cols(), "run must lie inside the row");

        for (cells, has_arabic) in script_segments(row, run.cells.clone()) {
            let start = row.cell_starts[cells.start];
            let end = row.cell_starts[cells.end];
            let text = &row.text[start..end];
            if text.is_empty() {
                continue;
            }

            let arabic = has_arabic && faces.arabic.is_some();
            let font = u8::from(arabic);
            let face = match (arabic, &faces.arabic) {
                (true, Some(face)) => face,
                _ => &faces.primary,
            };
            // Identity maps carry no levels, so their direction comes from the
            // segment's first strong character. Analyzed maps always trust the
            // bidi level, even when it is even and the text is Arabic.
            // Arabic segments always shape right-to-left: the script's joining
            // forms require the RTL direction even when an outer bidi level is
            // even (for example inside an explicit LTR embedding). Other
            // segments trust the bidi level, and identity maps fall back to
            // their first strong character.
            let rtl = if map.is_bidi_analyzed() {
                run.level % 2 == 1
            } else if has_arabic {
                true
            } else {
                first_strong_is_rtl(text)
            };
            debug_assert_eq!(
                run.level % 2 == 1,
                map.levels()
                    .get(cells.start)
                    .is_some_and(|level| level % 2 == 1),
                "run level and map level must agree"
            );

            let shaped = cache.shape(text, rtl, font, face);
            let mut groups: Vec<(usize, Vec<Glyph>)> = Vec::new();
            for glyph in &shaped.glyphs {
                let glyph = Glyph {
                    cluster: start + glyph.cluster,
                    ..*glyph
                };
                let cluster = glyph.cluster;
                match groups.last_mut() {
                    Some((existing, glyphs)) if *existing == cluster => glyphs.push(glyph),
                    _ => groups.push((cluster, vec![glyph])),
                }
            }

            let mut starts: Vec<usize> = groups.iter().map(|(cluster, _)| *cluster).collect();
            starts.sort_unstable();

            let mut clusters = Vec::with_capacity(groups.len());
            for (cluster, glyphs) in groups {
                let next = starts
                    .iter()
                    .copied()
                    .find(|candidate| *candidate > cluster)
                    .unwrap_or(end);
                let first = row.cell_of_byte(cluster);
                let mut last = row.cell_of_byte(next.saturating_sub(1).max(cluster));
                last += usize::from(row.widths[last].max(1) - 1);
                last = last.min(map.cols().saturating_sub(1));
                let cells = first..last + 1;
                let a = map.visual_of(cells.start);
                let b = map.visual_of(cells.end - 1);
                let slots = a.min(b)..a.max(b) + 1;
                clusters.push(Cluster {
                    cells,
                    slots,
                    glyphs,
                    text: row.text[cluster..next.min(row.text.len())].to_string(),
                });
            }
            clusters.sort_by_key(|cluster| cluster.slots.start);

            let a = map.visual_of(cells.start);
            let b = map.visual_of(cells.end - 1);
            out.push(ShapedRun {
                slots: a.min(b)..a.max(b) + 1,
                arabic,
                units_per_em: shaped.units_per_em,
                clusters,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::RowText;
    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};
    use crate::visual::VisualMap;
    use rustybuzz::{Direction, Face, UnicodeBuffer};
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    /// Some UI fonts (Segoe UI on this host) carry Arabic glyphs but no
    /// lam-alef ligature in GSUB, so they cannot exercise the cluster test.
    fn ligates_lam_alef(data: &[u8], index: u32) -> bool {
        let Some(face) = Face::from_slice(data, index) else {
            return false;
        };
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str("لا");
        buffer.set_direction(Direction::RightToLeft);
        rustybuzz::shape(&face, &[], buffer).glyph_infos().len() == 1
    }

    fn find_arabic_font() -> Option<(Vec<u8>, u32)> {
        let mut database = fontdb::Database::new();
        database.load_font_data(crate::fonts::BUNDLED_ARABIC.to_vec());
        database.load_system_fonts();
        for family in [
            "Kawkab Mono",
            "Segoe UI",
            "Arial",
            "Tahoma",
            "Noto Naskh Arabic",
            "Noto Sans Arabic",
            "DejaVu Sans",
            "FreeSerif",
        ] {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            if let Some(id) = database.query(&query) {
                if let Some((data, index)) =
                    database.with_face_data(id, |data, index| (data.to_vec(), index))
                {
                    if ligates_lam_alef(&data, index) {
                        return Some((data, index));
                    }
                }
            }
        }
        None
    }

    fn with_arabic_font<T>(f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
        let (data, index) = find_arabic_font()?;
        Some(f(&data, index))
    }

    fn shape(text: &RowText, map: &VisualMap, faces: &Faces<'_>) -> Vec<ShapedRun> {
        shape_row(text, map, faces, &mut ShapeCache::new())
    }

    #[test]
    fn arabic_row_shapes_as_one_run() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["س", "ل", "م"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            (runs.len(), runs[0].clusters.len())
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result, (1, 3));
    }

    #[test]
    fn arabic_letters_use_joined_forms() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["س", "ل", "م"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            let joined = runs[0]
                .clusters
                .iter()
                .find(|cluster| cluster.cells.start == 0)
                .expect("first cluster")
                .glyphs[0]
                .glyph_id;
            let face = Face::from_slice(data, index).expect("face");
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str("س");
            buffer.set_direction(Direction::RightToLeft);
            let isolated = rustybuzz::shape(&face, &[], buffer).glyph_infos()[0].glyph_id;
            (joined, isolated)
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_ne!(
            result.0, result.1,
            "the initial form must differ from the isolated form"
        );
    }

    #[test]
    fn lam_alef_is_one_cluster_across_two_cells() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["ل", "ا"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            runs[0].clusters[0].clone()
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.cells, 0..2);
        assert_eq!(result.slots, 0..2);
        assert!(!result.glyphs.is_empty());
    }

    #[test]
    fn combining_marks_stay_in_the_base_cluster() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["مَ"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            (runs[0].clusters.len(), runs[0].clusters[0].cells.clone())
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.0, 1);
        assert_eq!(result.1, 0..1);
    }

    #[test]
    fn mixed_direction_rows_split_at_the_run_boundary() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["a", "b", "c", "م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            let counts: Vec<usize> = runs.iter().map(|run| run.clusters.len()).collect();
            (
                runs.len(),
                counts,
                runs[0].slots.clone(),
                runs[1].slots.clone(),
            )
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.0, 2);
        assert_eq!(result.1, vec![3, 3]);
        assert_eq!(result.2, 0..3);
        assert_eq!(result.3, 3..6);
    }

    #[test]
    fn identity_maps_still_shape_arabic_with_rtl_joining() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::identity(3);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            let joined = runs[0]
                .clusters
                .iter()
                .find(|cluster| cluster.cells.start == 0)
                .expect("first cluster")
                .glyphs[0]
                .glyph_id;
            let face = Face::from_slice(data, index).expect("face");
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str("م");
            buffer.set_direction(Direction::RightToLeft);
            let isolated = rustybuzz::shape(&face, &[], buffer).glyph_infos()[0].glyph_id;
            (joined, isolated)
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_ne!(
            result.0, result.1,
            "identity maps must still use RTL joining forms"
        );
    }

    #[test]
    fn clusters_cover_every_visual_slot() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["a", "b", "c", "م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape(&text, &map, &faces);
            let mut seen = vec![false; 6];
            for run in &runs {
                for cluster in &run.clusters {
                    for slot in cluster.slots.clone() {
                        assert!(!seen[slot], "slot {slot} covered twice");
                        seen[slot] = true;
                    }
                }
            }
            seen
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert!(result.into_iter().all(|covered| covered));
    }

    #[test]
    fn style_changes_do_not_split_a_run() {
        let Some(result) = with_arabic_font(|data, index| {
            let mut first = cell("م");
            first.style.fg = crate::term::Color::Indexed(1);
            let second = cell("ر");
            let line = RowSnapshot {
                cells: vec![first, second],
                wrapped: false,
            };
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            shape(&text, &map, &faces).len()
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(
            result, 1,
            "style is applied after shaping, so one bidi run stays one shaped run"
        );
    }

    #[test]
    fn mixed_scripts_pick_a_face_per_segment() {
        let fonts = crate::fonts::FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| crate::fonts::FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        if !fonts.has_arabic() {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        }
        let faces = Faces::from_fonts(&fonts).expect("the loaded faces must parse");
        let line = row(&["a", "b", "c", "م", "ر", "ح"]);
        let text = RowText::flatten(&line);
        // One identity run: without per-segment splitting every cell would use
        // the Arabic face when bidi is disabled.
        let map = VisualMap::identity(line.cells.len());
        let runs = shape(&text, &map, &faces);
        assert_eq!(runs.len(), 2, "the script change splits the identity run");
        assert!(!runs[0].arabic);
        assert!(runs[1].arabic);
        assert_eq!(runs[0].units_per_em, faces.primary.units_per_em());
        assert_eq!(
            runs[1].units_per_em,
            faces.arabic.as_ref().expect("arabic face").units_per_em()
        );
    }

    #[test]
    fn clusters_carry_their_source_text() {
        let line = row(&["A", "✕"]);
        let text = RowText::flatten(&line);
        let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
        let Some((data, index)) = find_arabic_font() else {
            return;
        };
        let Some(face) = Face::from_slice(&data, index) else {
            return;
        };
        let runs = shape(&text, &map, &Faces::new(face, None));
        assert_eq!(runs[0].clusters[0].text, "A");
        assert_eq!(runs[0].clusters[1].text, "✕");
    }

    #[test]
    fn analyzed_even_levels_shape_left_to_right() {
        let Some(result) = with_arabic_font(|data, index| {
            // An LTR override keeps the Arabic letters at level 0: the text's
            // script must not flip an analyzed even level to RTL.
            let line = row(&["\u{202D}", "م", "ر", "ح", "\u{202C}"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let levels = map.levels().to_vec();
            let primary = Face::from_slice(data, index).expect("primary face");
            let arabic = Face::from_slice(data, index).expect("arabic face");
            let faces = Faces::new(primary, Some(arabic));
            let runs = shape(&text, &map, &faces);
            let arabic_run = runs.iter().find(|run| run.arabic).expect("Arabic segment");
            let actual: Vec<u32> = arabic_run
                .clusters
                .iter()
                .flat_map(|cluster| cluster.glyphs.iter().map(|glyph| glyph.glyph_id))
                .collect();
            let face = Face::from_slice(data, index).expect("face");
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str("مرح");
            buffer.set_direction(Direction::LeftToRight);
            let expected: Vec<u32> = rustybuzz::shape(&face, &[], buffer)
                .glyph_infos()
                .iter()
                .map(|info| info.glyph_id)
                .collect();
            (levels, actual, expected)
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert!(
            result.0[1..4].iter().all(|level| level % 2 == 0),
            "the LTR override must force even levels: {:?}",
            result.0
        );
        assert_eq!(result.1, result.2, "an even level must shape left to right");
    }
}
