use std::collections::HashSet;

/// The parsed, normalized form of `--only`/`--skip`: a plain set of tokens
/// per side, already deduplicated.
///
/// Deliberately knows nothing about CLI syntax (dashes, commas, repeated
/// flags) or how tokens are derived from a directory name - parsing lives
/// upstream (`cli::parse_instance`, this type's own constructor) and
/// matching lives downstream (`DirectoryMatcher`). This is just the shared
/// vocabulary both sides agree on: "does this set of tokens satisfy this
/// filter?", not "does this string contain that substring?".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FilterExpression {
    pub only: HashSet<String>,
    pub skip: HashSet<String>,
}

impl FilterExpression {
    pub fn new(only: Option<&[String]>, skip: Option<&[String]>) -> Self {
        Self {
            only: to_set(only),
            skip: to_set(skip),
        }
    }

    /// True when neither `--only` nor `--skip` was given, i.e. every
    /// directory satisfies this filter.
    pub fn is_empty(&self) -> bool {
        self.only.is_empty() && self.skip.is_empty()
    }
}

fn to_set(values: Option<&[String]>) -> HashSet<String> {
    values
        .map(|v| v.iter().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_when_neither_side_given() {
        assert!(FilterExpression::new(None, None).is_empty());
    }

    #[test]
    fn deduplicates_repeated_values() {
        let only = strings(&["uk", "uk", "fi"]);
        let filter = FilterExpression::new(Some(&only), None);
        assert_eq!(filter.only, set(&["uk", "fi"]));
    }

    #[test]
    fn only_and_skip_are_independent() {
        let only = strings(&["uk"]);
        let skip = strings(&["corp"]);
        let filter = FilterExpression::new(Some(&only), Some(&skip));
        assert!(!filter.is_empty());
        assert_eq!(filter.only, set(&["uk"]));
        assert_eq!(filter.skip, set(&["corp"]));
    }
}
