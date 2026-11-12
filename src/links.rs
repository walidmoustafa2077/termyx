//! Detection of clickable destinations in the terminal grid.
//!
//! Two sources feed the spans the underline pass paints and the click
//! handler opens: the OSC 8 runs the shell emitted, and an ASCII scan of
//! plain text for `http(s)://` URLs and absolute paths. Detection never
//! touches the filesystem; the destination is opened only on click.

use crate::term::GridSnapshot;

/// One underlined, clickable run of cells on a single row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Index into the snapshot's lines.
    pub row: usize,
    /// First logical column, inclusive.
    pub col: usize,
    /// Length in cells, including any `:line:col` suffix the scan matched.
    pub len: usize,
    /// The destination opened on click: a URL or an absolute path.
    pub uri: String,
}

/// Every link in the snapshot, ordered by row then column.
///
/// OSC 8 runs come from the shell verbatim; the plain-text scan skips the
/// ranges a run already covers so one target is never reported twice.
pub fn scan(snapshot: &GridSnapshot) -> Vec<LinkSpan> {
    let mut spans: Vec<LinkSpan> = snapshot
        .hyperlinks
        .iter()
        .map(|run| LinkSpan {
            row: run.row,
            col: run.start,
            len: run.end.saturating_sub(run.start),
            uri: run.uri.clone(),
        })
        .collect();
    scan_text(snapshot, &mut spans);
    spans.sort_by_key(|span| (span.row, span.col));
    spans
}

/// The span covering a cell, if any. `row` is a snapshot line index and
/// `col` a logical column, the coordinates `pos_at` produces.
pub fn at(spans: &[LinkSpan], row: usize, col: usize) -> Option<&LinkSpan> {
    spans
        .iter()
        .find(|span| span.row == row && col >= span.col && col < span.col + span.len)
}

/// Scans the plain text of every row, joining rows a soft wrap connects.
fn scan_text(snapshot: &GridSnapshot, out: &mut Vec<LinkSpan>) {
    let mut start = 0;
    while start < snapshot.lines.len() {
        let mut end = start;
        while snapshot.lines[end].wrapped && end + 1 < snapshot.lines.len() {
            end += 1;
        }
        scan_segment(snapshot, start, end, out);
        start = end + 1;
    }
}

/// Scans one segment of rows that reads as a single line of text.
///
/// The segment text is byte-scanned; `origin` maps every byte back to the
/// cell it came from so a match becomes row/column spans.
fn scan_segment(
    snapshot: &GridSnapshot,
    first_row: usize,
    last_row: usize,
    out: &mut Vec<LinkSpan>,
) {
    let mut text = String::new();
    // (row, logical column, cell width) per byte of `text`.
    let mut origin: Vec<(usize, usize, u8)> = Vec::new();
    for row in first_row..=last_row {
        let Some(line) = snapshot.lines.get(row) else {
            continue;
        };
        for (col, cell) in line.cells.iter().enumerate() {
            if cell.width == 0 {
                continue;
            }
            for _ in cell.text.bytes() {
                origin.push((row, col, cell.width));
            }
            text.push_str(&cell.text);
        }
    }
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if let Some(end) = match_at(bytes, cursor) {
            // `cursor` always sits on an ASCII marker or prefix byte, so the
            // slice is char-aligned even when the loop stepped mid-character.
            emit(snapshot, &origin, &text, cursor, end, out);
            cursor = end.max(cursor + 1);
        } else {
            cursor += 1;
        }
    }
}

/// A URL or absolute path starting at `at`, or `None`.
///
/// URLs are found first so a path-looking fragment inside one (the `/C:\...`
/// of a file URL) never splits the match. A path must also start its own
/// token: the slash in `src/main.rs` is not a root.
fn match_at(bytes: &[u8], at: usize) -> Option<usize> {
    if let Some(end) = match_url(bytes, at) {
        return Some(end);
    }
    if !path_start_ok(bytes, at) {
        return None;
    }
    path_prefix(bytes, at).and_then(|prefix| {
        let end = consume(bytes, at);
        (end - at >= prefix).then(|| trim_trailing(bytes, at, end))
    })
}

/// Whether a path match may begin at `at`: not glued to the token before it.
fn path_start_ok(bytes: &[u8], at: usize) -> bool {
    match at.checked_sub(1).map(|index| bytes[index]) {
        None => true,
        Some(previous) => {
            !previous.is_ascii_alphanumeric() && !matches!(previous, b'.' | b'/' | b'\\')
        }
    }
}

/// The `http://` or `https://` marker at `at` with its body consumed and
/// trailing punctuation trimmed. The scheme alone is not a destination.
fn match_url(bytes: &[u8], at: usize) -> Option<usize> {
    for marker in ["https://", "http://"] {
        let head = marker.as_bytes();
        if bytes.len() < at + head.len() || !bytes[at..at + head.len()].eq_ignore_ascii_case(head) {
            continue;
        }
        let end = trim_trailing(bytes, at, consume(bytes, at));
        if end > at + head.len() {
            return Some(end);
        }
    }
    None
}

/// Consumes from `at` until whitespace, a quote or bracket that cannot
/// appear in a target, or any non-ASCII byte.
///
/// Stopping at non-ASCII keeps Arabic text glued to a URL out of the match;
/// paths with non-ASCII names are a known v1 limit.
fn consume(bytes: &[u8], at: usize) -> usize {
    let mut end = at;
    while end < bytes.len() && !is_stop(bytes[end]) {
        end += 1;
    }
    end
}

fn is_stop(byte: u8) -> bool {
    byte <= 0x20
        || byte >= 0x7F
        || matches!(byte, b'"' | b'\'' | b'<' | b'>' | b'|' | b'`' | b'{' | b'}')
}

/// Drops trailing punctuation a target rarely ends with, keeping `)` `]`
/// `}` when they are balanced inside the match.
fn trim_trailing(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut end = end;
    while end > start {
        let byte = bytes[end - 1];
        let drop = match byte {
            b',' | b'.' | b';' | b'!' | b':' | b'\'' | b'"' | b'*' => true,
            b')' => count(&bytes[start..end], b')') > count(&bytes[start..end], b'('),
            b']' => count(&bytes[start..end], b']') > count(&bytes[start..end], b'['),
            b'}' => count(&bytes[start..end], b'}') > count(&bytes[start..end], b'{'),
            _ => false,
        };
        if !drop {
            break;
        }
        end -= 1;
    }
    end
}

fn count(bytes: &[u8], needle: u8) -> usize {
    bytes.iter().filter(|byte| **byte == needle).count()
}

/// The minimum length of an absolute-path match at `at`, if one starts
/// there. The value doubles as the shortest acceptable match, so a lone
/// `/` is never a destination.
///
/// Only absolute targets qualify: a drive letter, a UNC share, `~/`, or a
/// POSIX root. Relative paths such as `src/main.rs` are not links.
fn path_prefix(bytes: &[u8], at: usize) -> Option<usize> {
    let rest = bytes.get(at..)?;
    if rest.len() >= 3
        && rest[0].is_ascii_alphabetic()
        && rest[1] == b':'
        && (rest[2] == b'\\' || rest[2] == b'/')
    {
        return Some(3);
    }
    if rest.len() >= 3 && rest[0] == b'\\' && rest[1] == b'\\' {
        return Some(3);
    }
    if rest.starts_with(b"~/") || rest.starts_with(b"~\\") {
        return Some(2);
    }
    if rest.len() >= 2 && rest[0] == b'/' {
        return Some(2);
    }
    None
}

/// Turns one byte match into row spans, dropping it when an OSC 8 run
/// already covers any of the cells.
fn emit(
    snapshot: &GridSnapshot,
    origin: &[(usize, usize, u8)],
    text: &str,
    start: usize,
    end: usize,
    out: &mut Vec<LinkSpan>,
) {
    let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
    let mut index = start;
    while index < end {
        let (row, col, width) = origin[index];
        let (mut last_col, mut last_width) = (col, width);
        let mut next = index + 1;
        while next < end && origin[next].0 == row {
            last_col = origin[next].1;
            last_width = origin[next].2;
            next += 1;
        }
        pieces.push((row, col, last_col + usize::from(last_width.max(1)) - col));
        index = next;
    }
    if pieces
        .iter()
        .any(|(row, col, len)| covered(&snapshot.hyperlinks, *row, *col, *len))
    {
        return;
    }
    let uri = target(&text[start..end]);
    for (row, col, len) in pieces {
        out.push(LinkSpan {
            row,
            col,
            len,
            uri: uri.clone(),
        });
    }
}

/// Whether an OSC 8 run already claims any cell of the span.
fn covered(runs: &[crate::term::HyperlinkRun], row: usize, col: usize, len: usize) -> bool {
    runs.iter()
        .any(|run| run.row == row && col < run.end && run.start < col + len)
}

/// The destination for a match: paths lose a trailing `:line`/`:line:col`
/// so the file opens, while the underline still covers the whole token.
fn target(raw: &str) -> String {
    if path_prefix(raw.as_bytes(), 0).is_none() {
        return raw.to_string();
    }
    let mut cleaned = raw;
    while let Some(index) = cleaned.rfind(':') {
        let tail = &cleaned[index + 1..];
        if !tail.is_empty() && tail.len() <= 6 && tail.bytes().all(|byte| byte.is_ascii_digit()) {
            cleaned = &cleaned[..index];
        } else {
            break;
        }
    }
    if cleaned.len() >= 3 && path_prefix(cleaned.as_bytes(), 0).is_some() {
        cleaned.to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{
        Affinity, Cell, CursorShape, CursorState, GridSnapshot, HyperlinkRun, RowSnapshot, Style,
    };

    fn cell(ch: char) -> Cell {
        Cell {
            text: ch.to_string(),
            width: 1,
            style: Style::default(),
        }
    }

    fn row(text: &str) -> RowSnapshot {
        RowSnapshot {
            cells: text.chars().map(cell).collect(),
            wrapped: false,
        }
    }

    fn wrapped_row(text: &str) -> RowSnapshot {
        RowSnapshot {
            cells: text.chars().map(cell).collect(),
            wrapped: true,
        }
    }

    fn snapshot(lines: Vec<RowSnapshot>) -> GridSnapshot {
        let rows = lines.len();
        GridSnapshot {
            cols: 80,
            rows,
            lines,
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            input_start: None,
            history: 0,
            hyperlinks: Vec::new(),
        }
    }

    fn uris(spans: &[LinkSpan]) -> Vec<&str> {
        spans.iter().map(|span| span.uri.as_str()).collect()
    }

    #[test]
    fn an_https_url_is_one_span() {
        let spans = scan(&snapshot(vec![row("see https://example.com/a now")]));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].row, 0);
        assert_eq!(spans[0].col, 4);
        assert_eq!(spans[0].len, "https://example.com/a".len());
        assert_eq!(spans[0].uri, "https://example.com/a");
    }

    #[test]
    fn an_http_url_is_detected() {
        let spans = scan(&snapshot(vec![row("http://old.example/x")]));
        assert_eq!(uris(&spans), vec!["http://old.example/x"]);
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_target() {
        let spans = scan(&snapshot(vec![row("see https://x.com.")]));
        assert_eq!(uris(&spans), vec!["https://x.com"]);
        // The dot stays outside the underline.
        assert_eq!(spans[0].len, "https://x.com".len());
    }

    #[test]
    fn a_closing_paren_is_kept_when_it_is_balanced() {
        let spans = scan(&snapshot(vec![row(
            "https://en.wikipedia.org/wiki/Foo_(bar)",
        )]));
        assert_eq!(
            uris(&spans),
            vec!["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
    }

    #[test]
    fn an_unbalanced_closing_paren_is_trimmed() {
        let spans = scan(&snapshot(vec![row("(https://x.com)")]));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].col, 1);
        assert_eq!(spans[0].uri, "https://x.com");
    }

    #[test]
    fn a_drive_path_is_detected() {
        let spans = scan(&snapshot(vec![row(r"run C:\src\main.rs now")]));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].col, 4);
        assert_eq!(spans[0].uri, r"C:\src\main.rs");
    }

    #[test]
    fn a_line_and_column_suffix_is_opened_without_it() {
        let spans = scan(&snapshot(vec![row(r"C:\src\main.rs:12:5")]));
        assert_eq!(spans.len(), 1);
        // The whole token including the suffix is underlined...
        assert_eq!(spans[0].len, "C:\\src\\main.rs:12:5".len());
        // ...but the file opens without it.
        assert_eq!(spans[0].uri, r"C:\src\main.rs");
    }

    #[test]
    fn home_and_posix_paths_are_detected() {
        let spans = scan(&snapshot(vec![row("~/notes/todo.txt and /usr/bin/env")]));
        assert_eq!(uris(&spans), vec!["~/notes/todo.txt", "/usr/bin/env"]);
    }

    #[test]
    fn a_unc_path_is_detected() {
        let spans = scan(&snapshot(vec![row(r"\\server\share\file.txt")]));
        assert_eq!(uris(&spans), vec![r"\\server\share\file.txt"]);
    }

    #[test]
    fn a_relative_path_is_not_a_link() {
        let spans = scan(&snapshot(vec![row("edit src/main.rs please")]));
        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    }

    #[test]
    fn a_lone_slash_is_not_a_link() {
        let spans = scan(&snapshot(vec![row("a / b")]));
        assert!(spans.is_empty(), "unexpected spans: {spans:?}");
    }

    #[test]
    fn glued_non_ascii_text_ends_a_url() {
        let spans = scan(&snapshot(vec![row("https://x.comمرحبا")]));
        assert_eq!(uris(&spans), vec!["https://x.com"]);
    }

    #[test]
    fn a_url_split_across_wrapped_rows_is_one_link_per_row() {
        let lines = vec![wrapped_row("see https://exam"), row("ple.com end")];
        let spans = scan(&snapshot(lines));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].row, 0);
        assert_eq!(spans[0].uri, "https://example.com");
        assert_eq!(spans[1].row, 1);
        assert_eq!(spans[1].uri, "https://example.com");
        assert_eq!(spans[1].col, 0);
        assert_eq!(spans[1].len, "ple.com".len());
    }

    #[test]
    fn an_osc8_run_is_reported_verbatim() {
        let mut snapshot = snapshot(vec![row("link text here")]);
        snapshot.hyperlinks = vec![HyperlinkRun {
            row: 0,
            start: 4,
            end: 8,
            uri: "https://shell.example".into(),
        }];
        let spans = scan(&snapshot);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].col, 4);
        assert_eq!(spans[0].len, 4);
        assert_eq!(spans[0].uri, "https://shell.example");
    }

    #[test]
    fn plain_text_inside_an_osc8_run_is_not_reported_twice() {
        let mut snapshot = snapshot(vec![row("https://example.com")]);
        snapshot.hyperlinks = vec![HyperlinkRun {
            row: 0,
            start: 0,
            end: 19,
            uri: "https://shell.example".into(),
        }];
        let spans = scan(&snapshot);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].uri, "https://shell.example");
    }

    #[test]
    fn spans_are_ordered_by_row_then_column() {
        let spans = scan(&snapshot(vec![
            row("/usr/bin/env and https://a.example"),
            row(r"C:\one and C:\two"),
        ]));
        let coords: Vec<(usize, usize)> = spans.iter().map(|s| (s.row, s.col)).collect();
        assert_eq!(
            coords,
            vec![(0, 0), (0, 17), (1, 0), (1, 11)],
            "spans: {spans:?}"
        );
    }

    #[test]
    fn at_finds_the_span_under_a_cell() {
        let spans = scan(&snapshot(vec![row("see https://x.com now")]));
        assert!(at(&spans, 0, 4).is_some());
        assert!(at(&spans, 0, 4 + "https://x.com".len() - 1).is_some());
        assert!(at(&spans, 0, 4 + "https://x.com".len()).is_none());
        assert!(at(&spans, 0, 3).is_none());
        assert!(at(&spans, 1, 4).is_none());
    }

    #[test]
    fn an_empty_grid_has_no_links() {
        assert!(scan(&snapshot(vec![row("plain words only")])).is_empty());
    }
}
