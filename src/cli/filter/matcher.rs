use super::expression::FilterExpression;
use super::tokenizer::DirectoryTokenizer;

/// Answers exactly one question - "does this directory satisfy this
/// filter?" - by way of directory metadata (its name's components), not
/// string search over its path.
///
/// How a name becomes tokens (`DirectoryTokenizer`) and what a filter's
/// tokens are (`FilterExpression`) are each somebody else's concern; this
/// type only combines the two.
pub struct DirectoryMatcher {
    tokenizer: DirectoryTokenizer,
}

impl DirectoryMatcher {
    pub fn new(tokenizer: DirectoryTokenizer) -> Self {
        Self { tokenizer }
    }

    /// `only` and `skip` both apply, combined with AND: a directory must
    /// have at least one token in `only` (when `only` is non-empty) AND no
    /// token in `skip` (when `skip` is non-empty). An empty side of the
    /// filter imposes no constraint on its own.
    pub fn matches(&self, directory_name: &str, filter: &FilterExpression) -> bool {
        let tokens = self.tokenizer.tokenize(directory_name);

        let satisfies_only = filter.only.is_empty() || !filter.only.is_disjoint(&tokens);
        let satisfies_skip = filter.skip.is_empty() || filter.skip.is_disjoint(&tokens);

        satisfies_only && satisfies_skip
    }
}

impl Default for DirectoryMatcher {
    fn default() -> Self {
        Self::new(DirectoryTokenizer::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTORIES: &[&str] = &[
        "uk-priv-app",
        "uk-corp-app",
        "fi-priv-app",
        "fi-corp-app",
        "nl-priv-app",
        "at-corp-app",
    ];

    fn matching(filter: &FilterExpression) -> Vec<&'static str> {
        let matcher = DirectoryMatcher::default();
        DIRECTORIES
            .iter()
            .copied()
            .filter(|name| matcher.matches(name, filter))
            .collect()
    }

    fn only(values: &[&str]) -> FilterExpression {
        let values: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        FilterExpression::new(Some(&values), None)
    }

    fn skip(values: &[&str]) -> FilterExpression {
        let values: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        FilterExpression::new(None, Some(&values))
    }

    fn only_and_skip(only_values: &[&str], skip_values: &[&str]) -> FilterExpression {
        let only: Vec<String> = only_values.iter().map(|s| s.to_string()).collect();
        let skip: Vec<String> = skip_values.iter().map(|s| s.to_string()).collect();
        FilterExpression::new(Some(&only), Some(&skip))
    }

    #[test]
    fn no_filter_matches_everything() {
        assert_eq!(matching(&FilterExpression::default()), DIRECTORIES.to_vec());
    }

    #[test]
    fn only_uk_selects_both_uk_directories() {
        assert_eq!(matching(&only(&["uk"])), vec!["uk-priv-app", "uk-corp-app"]);
    }

    #[test]
    fn only_fi_selects_both_fi_directories() {
        assert_eq!(matching(&only(&["fi"])), vec!["fi-priv-app", "fi-corp-app"]);
    }

    #[test]
    fn only_app_selects_every_app() {
        assert_eq!(matching(&only(&["app"])), DIRECTORIES.to_vec());
    }

    #[test]
    fn only_corp_selects_every_corporate_app() {
        assert_eq!(
            matching(&only(&["corp"])),
            vec!["uk-corp-app", "fi-corp-app", "at-corp-app"]
        );
    }

    #[test]
    fn skip_priv_excludes_every_private_app() {
        assert_eq!(
            matching(&skip(&["priv"])),
            vec!["uk-corp-app", "fi-corp-app", "at-corp-app"]
        );
    }

    #[test]
    fn only_uk_and_skip_corp_combine_with_and_semantics() {
        assert_eq!(
            matching(&only_and_skip(&["uk"], &["corp"])),
            vec!["uk-priv-app"]
        );
    }

    #[test]
    fn only_app_skip_fi_excludes_finnish_apps_only() {
        assert_eq!(
            matching(&only_and_skip(&["app"], &["fi"])),
            vec!["uk-priv-app", "uk-corp-app", "nl-priv-app", "at-corp-app"]
        );
    }

    #[test]
    fn only_with_multiple_values_is_or_semantics() {
        assert_eq!(
            matching(&only(&["uk", "fi"])),
            vec!["uk-priv-app", "uk-corp-app", "fi-priv-app", "fi-corp-app"]
        );
    }

    #[test]
    fn only_app_skip_multiple_values_excludes_all_of_them() {
        assert_eq!(
            matching(&only_and_skip(&["app"], &["uk", "nl"])),
            vec!["fi-priv-app", "fi-corp-app", "at-corp-app"]
        );
    }

    #[test]
    fn matching_is_scoped_to_the_directory_name_not_its_full_path() {
        // A component matcher should judge a directory by its own name, not
        // by substring search over its full path - a parent folder that
        // happens to contain a filter word must not affect the result.
        let matcher = DirectoryMatcher::default();
        let filter = only(&["repos"]);
        assert!(!matcher.matches("uk-priv-app", &filter));
    }
}
