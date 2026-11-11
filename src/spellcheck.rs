//! Offline spellcheck for the shell input line.

use std::collections::{HashMap, HashSet};
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

pub fn scan(row: &[Cell], start_col: usize, cursor_col: usize) -> Vec<Misspelling> {
    let end = cursor_col.min(row.len());
    if end <= start_col {
        return Vec::new();
    }

    let mut found = Vec::new();
    let mut index = 0usize;
    let mut first_token_seen = false;

    while index < end {
        let cell = &row[index];
        let ch = cell.text.chars().next().unwrap_or(' ');
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        let word_start = index;
        let mut word = String::new();
        let mut quoted = false;
        while index < end {
            let current = &row[index];
            let c = current.text.chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                break;
            }
            if c == '"' || c == '\'' || c == '`' {
                quoted = true;
            }
            word.push(c);
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
        if !is_candidate(&word) {
            continue;
        }

        let lower = word.to_ascii_lowercase();
        if is_known(&lower) {
            continue;
        }

        found.push(Misspelling {
            row: 0,
            col: word_start.max(start_col),
            len: word.len(),
        });
    }

    found
}

fn is_candidate(word: &str) -> bool {
    if word.len() < MIN_TOKEN_LEN {
        return false;
    }
    if word.starts_with('-') {
        return false;
    }
    if word.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if word
        .chars()
        .any(|c| matches!(c, '\\' | '/' | ':' | '@' | '$' | '"' | '\'' | '`'))
    {
        return false;
    }
    if !word.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if word.len() >= 4 && word.chars().all(|c| !c.is_ascii_lowercase()) {
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
    fn skips_quoted_strings() {
        let cells = row("Write-Host \"wrng wrd\"");
        assert!(scan(&cells, 0, cells.len()).is_empty());
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
}
