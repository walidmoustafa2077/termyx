//! Find-in-buffer state: the query being typed and the matches it produced.
//!
//! The search itself lives on [`crate::term::Terminal`]; this module owns the
//! editor state that sits over it while the find bar is open.

use crate::term::Match;

#[derive(Debug, Default)]
pub struct FindState {
    pub query: String,
    pub matches: Vec<Match>,
    pub current: usize,
}

impl FindState {
    pub fn push_str(&mut self, text: &str) {
        self.query.push_str(text);
    }

    pub fn backspace(&mut self) {
        self.query.pop();
    }

    pub fn set_matches(&mut self, matches: Vec<Match>) {
        self.matches = matches;
        self.current = 0;
    }

    pub fn current(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }

    pub fn next(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.current()
    }

    pub fn previous(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::FindState;
    use crate::term::Match;

    fn matches() -> Vec<Match> {
        (0..3)
            .map(|line| Match {
                line,
                start: 0,
                end: 1,
            })
            .collect()
    }

    #[test]
    fn next_wraps_from_the_last_match_to_the_first() {
        let mut find = FindState::default();
        find.set_matches(matches());
        assert_eq!(
            find.current(),
            Some(Match {
                line: 0,
                start: 0,
                end: 1
            })
        );
        assert_eq!(
            find.next(),
            Some(Match {
                line: 1,
                start: 0,
                end: 1
            })
        );
        assert_eq!(
            find.next(),
            Some(Match {
                line: 2,
                start: 0,
                end: 1
            })
        );
        assert_eq!(
            find.next(),
            Some(Match {
                line: 0,
                start: 0,
                end: 1
            })
        );
    }

    #[test]
    fn previous_wraps_the_other_way() {
        let mut find = FindState::default();
        find.set_matches(matches());
        assert_eq!(
            find.previous(),
            Some(Match {
                line: 2,
                start: 0,
                end: 1
            })
        );
        assert_eq!(
            find.previous(),
            Some(Match {
                line: 1,
                start: 0,
                end: 1
            })
        );
        assert_eq!(
            find.previous(),
            Some(Match {
                line: 0,
                start: 0,
                end: 1
            })
        );
    }

    #[test]
    fn an_empty_match_list_yields_none() {
        let mut find = FindState::default();
        find.set_matches(Vec::new());
        assert_eq!(find.current(), None);
        assert_eq!(find.next(), None);
        assert_eq!(find.previous(), None);
    }
}
