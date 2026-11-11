//! Offline spellcheck for the shell input line.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::term::Cell;

const WORD_LIST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shell/powershell/Termyx/assets/en_US.wordlist.txt"
));

const FREQUENCY_LIST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shell/powershell/Termyx/assets/en_US.frequency.txt"
));

const MIN_TOKEN_LEN: usize = 3;
const MAX_DISTANCE_ONE_TOKEN: usize = 6;

/// One misspelled span, positioned on the grid.
///
/// `row` is a grid line where `0` is the top of the live screen and negative
/// values are history, the same coordinate `term::Match` uses. `scan` always
/// returns `0` because it sees one row; the caller assigns the line. The
/// renderer adds `snapshot.display_offset` before indexing
/// `GridSnapshot::lines`. `col` and `len` are logical cell columns within that
/// line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub row: usize,
    pub col: usize,
    pub len: usize,
}

struct WordData {
    words: HashSet<String>,
    rank: HashMap<String, usize>,
    added: HashSet<String>,
}

fn data() -> &'static RwLock<WordData> {
    static DATA: OnceLock<RwLock<WordData>> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut words = HashSet::new();
        for line in WORD_LIST.lines() {
            let word = line.trim();
            if !word.is_empty() {
                words.insert(word.to_ascii_lowercase());
            }
        }

        let mut rank = HashMap::new();
        for (index, line) in FREQUENCY_LIST.lines().enumerate() {
            let word = line.split(' ').next().unwrap_or("").trim();
            if !word.is_empty() {
                rank.entry(word.to_ascii_lowercase()).or_insert(index);
            }
        }

        RwLock::new(WordData {
            words,
            rank,
            added: HashSet::new(),
        })
    })
}

pub fn is_known(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    let guard = data().read().expect("spellcheck lock");
    guard.words.contains(&lower) || guard.added.contains(&lower)
}

pub fn add_word(token: &str) {
    let lower = token.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    let mut guard = data().write().expect("spellcheck lock");
    guard.added.insert(lower);
}

/// The user-word file the PowerShell module also reads.
///
/// `None` when the platform has no `LOCALAPPDATA`; the caller then still adds
/// the word to this session's in-memory set.
pub fn user_words_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(base)
            .join("Termyx")
            .join("user-words.txt"),
    )
}

/// Loads every saved user word into the in-memory set.
///
/// One word per line, trimmed, with blank lines skipped. A missing file and an
/// IO error are silent: the word list is a convenience, not a prerequisite.
pub fn load_user_words(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    for line in contents.lines() {
        let word = line.trim();
        if !word.is_empty() {
            add_word(word);
        }
    }
}

pub fn scan(row: &[Cell], start_col: usize, cursor_col: usize) -> Vec<Misspelling> {
    let end = cursor_col.min(row.len());
    if end <= start_col {
        return Vec::new();
    }

    let mut found = Vec::new();
    // Task 6 hands over the whole visible row plus the column the input starts
    // at, so the prompt and the command name both live before `start_col`.
    let mut index = start_col;
    let mut first_token_seen = false;
    // Quote state survives whitespace: every token from an opening quote to its
    // closing partner is skipped, however many tokens the span covers.
    let mut quote: Option<char> = None;

    while index < end {
        let ch = cell_char(&row[index]);
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        let word_start = index;
        let mut raw = String::new();
        let mut quoted = quote.is_some();

        while index < end {
            let c = cell_char(&row[index]);
            if c.is_whitespace() {
                break;
            }
            match quote {
                Some(open) => {
                    quoted = true;
                    if c == open {
                        quote = None;
                    }
                }
                None => {
                    if is_quote(c) && opens_a_quote(&raw) {
                        quote = Some(c);
                        quoted = true;
                    } else {
                        raw.push(c);
                    }
                }
            }
            index += 1;
        }

        if index == word_start {
            index += 1;
            continue;
        }

        if !first_token_seen {
            first_token_seen = true;
            continue;
        }
        if quoted {
            continue;
        }

        let leading = raw.len() - raw.trim_start_matches(is_trim_char).len();
        let word = raw.trim_matches(is_trim_char);
        let trailing = raw.len() - leading - word.len();
        let span_start = word_start + leading;
        // Report the cells the word occupies, not its byte length: `cafe` with
        // an accent is one cell more than three, and is rejected anyway.
        let span_len = index - span_start - trailing;

        // Never underline anything that begins before the input area.
        if span_start < start_col {
            continue;
        }
        if !is_candidate(word) {
            continue;
        }

        let lower = word.to_ascii_lowercase();
        if is_known(&lower) {
            continue;
        }

        found.push(Misspelling {
            row: 0,
            col: span_start,
            len: span_len,
        });
    }

    found
}

fn cell_char(cell: &Cell) -> char {
    cell.text.chars().next().unwrap_or(' ')
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

// A quote opens a quoted span at the start of a token or after punctuation such
// as the `(` in `Write-Host('wrng')`. Inside a word, as in `don't`, it is a
// literal apostrophe.
fn opens_a_quote(prefix: &str) -> bool {
    match prefix.chars().next_back() {
        Some(c) => !c.is_ascii_alphanumeric(),
        None => true,
    }
}

// Characters that may belong to a word. The class check in `is_candidate`
// rejects everything else, including accented letters.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '\'' | '_' | '-')
}

// Punctuation trimmed off both ends of a token so that `hello,` reports
// `hello`. Non-ASCII characters are never trimmed: they must reach
// `is_candidate` and be rejected there.
fn is_trim_char(c: char) -> bool {
    c.is_ascii_punctuation() && !matches!(c, '\'' | '_' | '-')
}

fn is_candidate(word: &str) -> bool {
    if word.len() < MIN_TOKEN_LEN {
        return false;
    }
    if word.starts_with('-') {
        return false;
    }
    // The class check also rejects `\ / : @ $`, quotes, and every non-ASCII
    // character, including accented letters.
    if !word.chars().all(is_word_char) {
        return false;
    }
    if word.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    // Dotted names are file paths or host names, whose trailing dot was already
    // trimmed: any dot left here is interior, as in `foo.txt`.
    if word.contains('.') {
        return false;
    }
    if !word.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Acronyms and environment-variable names are skipped at every length.
    if word.chars().all(|c| !c.is_ascii_lowercase()) {
        return false;
    }
    true
}

pub fn suggestions(token: &str, limit: usize) -> Vec<String> {
    let lower = token.to_ascii_lowercase();
    if limit == 0 || lower.len() < 2 {
        return Vec::new();
    }

    let guard = data().read().expect("spellcheck lock");
    let mut one_edit: Vec<&String> = guard
        .words
        .iter()
        .filter(|word| {
            word.len().saturating_sub(lower.len()) <= 1
                && lower.len().saturating_sub(word.len()) <= 1
        })
        .filter(|word| distance(&lower, word, 1) == 1)
        .collect();

    let candidates: Vec<&String> = if one_edit.is_empty() && lower.len() <= MAX_DISTANCE_ONE_TOKEN {
        guard
            .words
            .iter()
            .filter(|word| {
                word.len().saturating_sub(lower.len()) <= 2
                    && lower.len().saturating_sub(word.len()) <= 2
            })
            .filter(|word| distance(&lower, word, 2) == 2)
            .collect()
    } else {
        std::mem::take(&mut one_edit)
    };

    // A shorter candidate never wins while a same-length or longer one exists,
    // then the lowest frequency rank decides. Words absent from the frequency
    // list sort last but remain usable when they are the only candidate.
    let mut ranked: Vec<(bool, usize, &String)> = candidates
        .into_iter()
        .map(|word| {
            let shorter = word.len() < lower.len();
            let rank = guard.rank.get(word).copied().unwrap_or(usize::MAX / 2);
            (shorter, rank, word)
        })
        .collect();

    ranked.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, word)| word.clone())
        .collect()
}

fn distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    let mut before_previous = vec![0usize; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut value = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(before_previous[j - 2] + 1);
            }
            current[j] = value;
            row_min = row_min.min(value);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build cells the way the existing render tests do (see
    // `single_cell_snapshot` in `src/render/mod.rs:2552`). `Style` implements
    // `Default`, so the default style is what those tests use too.
    fn row(text: &str) -> Vec<crate::term::Cell> {
        text.chars()
            .map(|c| crate::term::Cell {
                text: c.to_string(),
                width: 1,
                style: Default::default(),
            })
            .collect()
    }

    #[test]
    fn skips_the_command_name() {
        let cells = row("Get-ChildItem wrng");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].col, 14);
        assert_eq!(found[0].len, 4);
    }

    #[test]
    fn finds_nothing_before_the_input_start() {
        let cells = row("PS> Get-ChildItem wrng");
        let found = scan(&cells, 4, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].col, found[0].len), (18, 4));

        // The command name is skipped, and the first checked token is not
        // clamped back onto the input start.
        let cells = row("dev box> hello wrng");
        let found = scan(&cells, 9, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].col, found[0].len), (15, 4));
    }

    #[test]
    fn skips_the_command_name_after_a_prompt() {
        let prompt = "PS D:\\Development\\termyx> ";
        let cells = row(&format!("{prompt}Write-Host wrng"));
        let found = scan(&cells, prompt.len(), cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].col, found[0].len), (prompt.len() + 11, 4));
    }

    #[test]
    fn skips_quoted_strings() {
        let cells = row("Write-Host \"wrng wrd\"");
        assert!(scan(&cells, 0, cells.len()).is_empty());
    }

    #[test]
    fn skips_every_token_in_a_quoted_span() {
        let cells = row("Write-Host \"a b wrng c d\"");
        assert!(scan(&cells, 0, cells.len()).is_empty());
    }

    #[test]
    fn ignores_trailing_punctuation() {
        for line in [
            "Write-Host hello,",
            "Write-Host hello.",
            "Write-Host (hello)",
        ] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }

        let cells = row("Write-Host wrng,");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].col, found[0].len), (11, 4));
    }

    #[test]
    fn skips_dotted_names_and_scheme_less_urls() {
        for line in ["Write-Host www.wrng.example", "Write-Host foo.txt"] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }
    }

    #[test]
    fn ignores_non_ascii_tokens() {
        let cells = row("Write-Host caf\u{e9}");
        assert!(scan(&cells, 0, cells.len()).is_empty());
    }

    #[test]
    fn skips_short_all_caps_tokens() {
        let cells = row("Write-Host ZQX");
        assert!(scan(&cells, 0, cells.len()).is_empty());

        let cells = row("Write-Host Wrng");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].col, found[0].len), (11, 4));
    }

    #[test]
    fn skips_paths_urls_flags_and_numbers() {
        for line in [
            "cd C:\\temp\\wrng",
            "cd C:wrng",
            "Write-Host https://wrng.example",
            "Write-Host -wrng",
            "Write-Host 1234",
        ] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }
    }

    #[test]
    fn skips_short_and_uppercase_tokens() {
        for line in ["Write-Host ab", "Write-Host WRNG"] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }
    }

    #[test]
    fn finds_several_misspellings() {
        let cells = row("Write-Host wrng and wrrd");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].col, found[0].len), (11, 4));
        assert_eq!((found[1].col, found[1].len), (20, 4));
    }

    #[test]
    fn stops_at_the_cursor() {
        let cells = row("Write-Host wrng");
        assert!(scan(&cells, 0, 12).is_empty());
    }

    #[test]
    fn knows_real_words() {
        assert!(is_known("hello"));
        assert!(is_known("Hello"));
        assert!(!is_known("wrng"));
    }

    #[test]
    fn suggests_before_underlining() {
        assert_eq!(
            suggestions("teh", 5).first().map(String::as_str),
            Some("the")
        );
        assert_eq!(
            suggestions("recieve", 5).first().map(String::as_str),
            Some("receive")
        );
    }

    #[test]
    fn respects_the_limit() {
        assert!(suggestions("adress", 1).len() <= 1);
    }

    #[test]
    fn added_word_stops_being_unknown() {
        add_word("zqxjv");
        assert!(is_known("zqxjv"));
    }

    fn temp_words(tag: &str) -> (PathBuf, PathBuf) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("termyx-load-{tag}-{}-{stamp}", std::process::id()));
        let path = dir.join("user-words.txt");
        (dir, path)
    }

    #[test]
    fn loads_every_saved_user_word() {
        let (dir, path) = temp_words("words");
        std::fs::create_dir_all(&dir).expect("the directory is created");
        std::fs::write(&path, "zqloadone\nzqloadtwo\n").expect("the file is written");

        load_user_words(&path);

        assert!(is_known("zqloadone"));
        assert!(is_known("zqloadtwo"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_a_missing_file_is_a_no_op() {
        let (dir, path) = temp_words("missing");
        load_user_words(&path);
        assert!(!is_known("zqmissingword"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_skips_blank_lines() {
        let (dir, path) = temp_words("blank");
        std::fs::create_dir_all(&dir).expect("the directory is created");
        std::fs::write(&path, "  \n zqblankword \n\n").expect("the file is written");

        load_user_words(&path);

        assert!(is_known("zqblankword"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_words_path_names_the_shared_file() {
        let Some(path) = user_words_path() else {
            return;
        };
        assert!(path.ends_with("Termyx/user-words.txt"));
    }
}
