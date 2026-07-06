pub mod dirs;
pub mod filter;

use std::collections::HashMap;

const VALID_OPTIONS: &[&str] = &[
    "sustain",
    "local",
    "skip",
    "only",
    "timeout",
    "all",
    "concurrency",
    "sync",
];

#[derive(Debug, Default, PartialEq)]
pub struct ParsedArgs {
    pub payload: Vec<String>,
    pub options: HashMap<String, Vec<String>>,
}

/// Parses CLI args into positional payload strings and `--key-v1-v2-...`
/// style options, mirroring InstanceParser.cs. Two deliberate deviations
/// from the original:
///
/// - a repeated option (e.g. two `--skip-...` tokens) merges its values
///   instead of throwing (the C# `Dictionary.Add` panicked on a duplicate
///   key, crashing the whole invocation over a harmless repeated flag)
/// - this is a plain function over a fresh `ParsedArgs` rather than a
///   static mutable dictionary, so parsing twice in the same process
///   (e.g. in tests) can't leak state between calls
pub fn parse_instance(args: &[String]) -> ParsedArgs {
    let mut parsed = ParsedArgs::default();
    for arg in args {
        parse_argument(arg, &mut parsed);
    }
    parsed
}

fn parse_argument(arg: &str, parsed: &mut ParsedArgs) {
    if arg.starts_with("--") {
        // C# TrimStart('-') strips every leading dash, not just the first two.
        let trimmed = arg.trim_start_matches('-');
        let key = trimmed.split('-').next().unwrap_or("");
        if VALID_OPTIONS.contains(&key) {
            parse_option(trimmed, parsed);
        }
        // An unrecognized --flag is silently dropped: not payload, not an
        // option. That matches the original, which never had an else/error
        // branch here.
    } else {
        // Single-dash tokens (e.g. "-h") fall through here too, since the
        // check above only matches "--".
        parsed.payload.push(arg.to_string());
    }
}

fn parse_option(option: &str, parsed: &mut ParsedArgs) {
    let mut parts = option.split('-');
    let key = parts.next().unwrap_or("").to_string();
    // Each dash-separated segment may itself be a comma-separated list (e.g.
    // "--only-uk,fi" or "--only-uk,fi-nl"), so split on both and flatten.
    //
    // A leading/trailing/doubled dash or comma (e.g. "--skip-" or
    // "--only--a-b" or "--only-a,") produces empty segments here. Left
    // unfiltered, an empty string would trivially satisfy any filter built
    // from it downstream, silently turning "--skip-" into "skip every
    // directory" and "--only--a-b" into "only" not restricting anything at
    // all.
    let values: Vec<String> = parts
        .flat_map(|segment| segment.split(','))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    parsed.options.entry(key).or_default().extend(values);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_args_produce_empty_result() {
        let parsed = parse_instance(&[]);
        assert_eq!(parsed, ParsedArgs::default());
    }

    #[test]
    fn positional_args_go_to_payload_in_order() {
        let parsed = parse_instance(&args(&["build", "release"]));
        assert_eq!(parsed.payload, vec!["build", "release"]);
        assert!(parsed.options.is_empty());
    }

    #[test]
    fn bare_flag_has_no_values() {
        let parsed = parse_instance(&args(&["--local"]));
        assert_eq!(parsed.options.get("local"), Some(&vec![]));
    }

    #[test]
    fn sync_flag_is_recognized() {
        // Guards against `--sync` being silently dropped as an unrecognized
        // flag (the fate of any option not listed in VALID_OPTIONS) if it's
        // ever removed from that list without a test noticing.
        let parsed = parse_instance(&args(&["--sync"]));
        assert_eq!(parsed.options.get("sync"), Some(&vec![]));
    }

    #[test]
    fn dash_packed_values_are_split() {
        let parsed = parse_instance(&args(&["--skip-a-b-c"]));
        assert_eq!(
            parsed.options.get("skip"),
            Some(&vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn unrecognized_flag_is_silently_dropped() {
        let parsed = parse_instance(&args(&["--bogus-x"]));
        assert!(parsed.options.is_empty());
        assert!(parsed.payload.is_empty());
    }

    #[test]
    fn single_dash_token_is_payload_not_a_flag() {
        let parsed = parse_instance(&args(&["-h"]));
        assert_eq!(parsed.payload, vec!["-h"]);
        assert!(parsed.options.is_empty());
    }

    #[test]
    fn repeated_flag_merges_values_instead_of_erroring() {
        let parsed = parse_instance(&args(&["--skip-a", "--skip-b"]));
        assert_eq!(
            parsed.options.get("skip"),
            Some(&vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn trailing_dash_produces_no_values_not_an_empty_string_value() {
        // "--skip-" must behave like a no-op filter (no values), not like a
        // filter containing "" - which would match (and remove) every
        // directory via `.contains("")` downstream.
        let parsed = parse_instance(&args(&["--skip-"]));
        assert_eq!(parsed.options.get("skip"), Some(&vec![]));
    }

    #[test]
    fn doubled_dash_skips_the_empty_segment() {
        // "--only--a-b" must yield ["a", "b"], not ["", "a", "b"] - an empty
        // value would match every directory via `.contains("")`, silently
        // defeating the whole point of `--only`.
        let parsed = parse_instance(&args(&["--only--a-b"]));
        assert_eq!(
            parsed.options.get("only"),
            Some(&vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn comma_separated_values_are_split() {
        let parsed = parse_instance(&args(&["--only-uk,fi"]));
        assert_eq!(
            parsed.options.get("only"),
            Some(&vec!["uk".to_string(), "fi".to_string()])
        );
    }

    #[test]
    fn comma_and_dash_separated_values_combine() {
        let parsed = parse_instance(&args(&["--only-uk,fi-nl"]));
        assert_eq!(
            parsed.options.get("only"),
            Some(&vec!["uk".to_string(), "fi".to_string(), "nl".to_string()])
        );
    }

    #[test]
    fn trailing_comma_produces_no_extra_empty_value() {
        let parsed = parse_instance(&args(&["--only-uk,"]));
        assert_eq!(parsed.options.get("only"), Some(&vec!["uk".to_string()]));
    }

    #[test]
    fn mixed_payload_and_options() {
        let parsed = parse_instance(&args(&["fep-cmd", "--only-web-api", "--sustain"]));
        assert_eq!(parsed.payload, vec!["fep-cmd"]);
        assert_eq!(
            parsed.options.get("only"),
            Some(&vec!["web".to_string(), "api".to_string()])
        );
        assert_eq!(parsed.options.get("sustain"), Some(&vec![]));
    }
}
