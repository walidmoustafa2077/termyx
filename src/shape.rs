//! Bidi-run shaping with rustybuzz and cluster-to-cell mapping.
//!
// Consumed by the renderer in Task 6; the tests below are the only callers
// until then.
#![allow(dead_code)]

use std::ops::Range;

use rustybuzz::{BufferClusterLevel, Direction, Face, UnicodeBuffer};

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

pub fn shape_row(row: &RowText, map: &VisualMap, faces: &Faces<'_>) -> Vec<ShapedRun> {
    let mut out = Vec::new();

    for run in map.runs() {
        if run.slots.is_empty() {
            continue;
        }
        let start = row.cell_starts[run.cells.start];
        let end = row.cell_starts[run.cells.end];
        let text = &row.text[start..end];
        if text.is_empty() {
            continue;
        }

        let arabic = contains_arabic(text) && faces.arabic.is_some();
        let face = if arabic {
            faces.arabic.as_ref().expect("arabic face checked above")
        } else {
            &faces.primary
        };
        debug_assert_eq!(
            run.level % 2 == 1,
            map.levels()
                .get(run.cells.start)
                .is_some_and(|level| level % 2 == 1),
            "run level and map level must agree"
        );
        let rtl = run.level % 2 == 1 || first_strong_is_rtl(text);

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        buffer.set_cluster_level(BufferClusterLevel::MonotoneGraphemes);
        let shaped = rustybuzz::shape(face, &[], buffer);

        let infos = shaped.glyph_infos();
        let positions = shaped.glyph_positions();
        let mut groups: Vec<(usize, Vec<Glyph>)> = Vec::new();
        for (info, position) in infos.iter().zip(positions) {
            let glyph = Glyph {
                glyph_id: info.glyph_id,
                cluster: start + info.cluster as usize,
                x_advance: position.x_advance,
                x_offset: position.x_offset,
                y_offset: position.y_offset,
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
            });
        }
        clusters.sort_by_key(|cluster| cluster.slots.start);

        out.push(ShapedRun {
            slots: run.slots.clone(),
            arabic,
            units_per_em: face.units_per_em(),
            clusters,
        });
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

    #[test]
    fn arabic_row_shapes_as_one_run() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["س", "ل", "م"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            let runs = shape_row(&text, &map, &faces);
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
            shape_row(&text, &map, &faces).len()
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(
            result, 1,
            "style is applied after shaping, so one bidi run stays one shaped run"
        );
    }
}
