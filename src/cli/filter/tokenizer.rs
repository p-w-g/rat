use std::collections::HashSet;

/// Splits a directory name into a set of tokens on a configurable set of
/// delimiter characters.
///
/// Isolated behind its own type so that how a name becomes tokens (`-`
/// today, maybe `_` or a user-configurable delimiter later) can change
/// without anything that consumes tokens (matcher, filters) needing to
/// know or care.
#[derive(Debug, Clone)]
pub struct DirectoryTokenizer {
    delimiters: Vec<char>,
}

impl DirectoryTokenizer {
    pub fn new(delimiters: Vec<char>) -> Self {
        Self { delimiters }
    }

    /// Splits `name` on any configured delimiter, dropping empty segments -
    /// a leading/doubled delimiter must not produce a `""` token, which
    /// would trivially satisfy every filter downstream.
    pub fn tokenize(&self, name: &str) -> HashSet<String> {
        name.split(|c: char| self.delimiters.contains(&c))
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect()
    }
}

impl Default for DirectoryTokenizer {
    /// `-` is the only delimiter today, matching the dash-joined naming
    /// convention (`uk-priv-app`) this feature is built around.
    fn default() -> Self {
        Self::new(vec!['-'])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_on_default_delimiter() {
        let tokenizer = DirectoryTokenizer::default();
        assert_eq!(
            tokenizer.tokenize("uk-priv-app"),
            set(&["uk", "priv", "app"])
        );
    }

    #[test]
    fn single_token_when_no_delimiter_present() {
        let tokenizer = DirectoryTokenizer::default();
        assert_eq!(tokenizer.tokenize("web"), set(&["web"]));
    }

    #[test]
    fn empty_segments_from_leading_or_doubled_delimiters_are_dropped() {
        let tokenizer = DirectoryTokenizer::default();
        assert_eq!(tokenizer.tokenize("--uk--priv--"), set(&["uk", "priv"]));
    }

    #[test]
    fn custom_delimiter_can_replace_the_default() {
        let tokenizer = DirectoryTokenizer::new(vec!['_']);
        assert_eq!(
            tokenizer.tokenize("uk_priv_app"),
            set(&["uk", "priv", "app"])
        );
    }

    #[test]
    fn multiple_delimiters_can_be_combined() {
        let tokenizer = DirectoryTokenizer::new(vec!['-', '_']);
        assert_eq!(
            tokenizer.tokenize("uk-priv_app"),
            set(&["uk", "priv", "app"])
        );
    }
}
