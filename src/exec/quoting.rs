/// Rebuilds a single shell command-line string from already-split argv
/// elements (`fep`'s payload), quoting only the elements that need it.
///
/// Each element of `payload` is exactly one OS-level argv word - i.e.
/// whatever the user's own shell produced after its own quote handling. An
/// element can only contain internal whitespace if the user explicitly
/// quoted it (`git commit -m "fix: my message"` arrives as the single
/// element `fix: my message`); naively rejoining elements with a plain
/// space, as a straight `payload.join(" ")` does, throws that quoting away
/// and lets the sub-shell re-split it into several words. Quoting exactly
/// the elements that contain whitespace - and leaving everything else
/// (words, single-dash flags, and shell operators like `&&`/`|` that the
/// user left unquoted on purpose) untouched - restores the original
/// argument boundaries without disturbing the shell syntax `fep` is
/// designed to let through.
///
/// This is a best-effort fix, not a general-purpose shell escaper: an
/// argument that contains shell-significant characters (`;`, `&`, `|`, ...)
/// but no whitespace is passed through unquoted, same as before. Rat has no
/// way to tell such a token apart from an intentionally unquoted shell
/// operator - the boundary information for that case was already lost by
/// the time argv reached this process. In practice this is a rare
/// construction compared to the quoted-multi-word-argument case this fixes.
pub fn build_command_line(payload: &[String]) -> String {
    payload
        .iter()
        .map(|arg| quote_if_needed(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_if_needed(arg: &str) -> String {
    if needs_quoting(arg) {
        quote(arg)
    } else {
        arg.to_string()
    }
}

fn needs_quoting(arg: &str) -> bool {
    arg.is_empty() || arg.chars().any(char::is_whitespace)
}

#[cfg(not(target_os = "windows"))]
fn quote(arg: &str) -> String {
    // POSIX single-quoting: nothing inside single quotes is special except
    // another single quote, which can't be escaped from within the quotes
    // at all - so close the quote, emit an escaped literal quote, and
    // reopen: `it's` -> `'it'"'"'s'`.
    format!("'{}'", arg.replace('\'', r#"'"'"'"#))
}

#[cfg(target_os = "windows")]
fn quote(arg: &str) -> String {
    // cmd.exe does no further word-splitting inside a quoted section; the
    // invoked program's own argv parser (the MSVCRT convention almost every
    // Windows program follows) does, where a doubled `"` inside a quoted
    // section represents one literal `"`. Windows command-line quoting is
    // notoriously inconsistent across programs, so - like the POSIX
    // branch - this covers the common case (embedded whitespace) rather
    // than claiming to be exhaustive.
    format!("\"{}\"", arg.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn words_without_whitespace_are_left_unquoted() {
        let payload = strings(&["git", "pull"]);
        assert_eq!(build_command_line(&payload), "git pull");
    }

    #[test]
    fn shell_operators_left_unquoted_still_work_as_operators() {
        // A token the user left unquoted (e.g. `&&`, its own argv element)
        // must not gain quotes, or it stops being shell syntax.
        let payload = strings(&["git", "pull", "&&", "npm", "install"]);
        assert_eq!(build_command_line(&payload), "git pull && npm install");
    }

    #[test]
    fn single_dash_flags_are_left_unquoted() {
        let payload = strings(&["git", "commit", "-m"]);
        assert_eq!(build_command_line(&payload), "git commit -m");
    }

    #[test]
    fn empty_argument_is_quoted_as_an_empty_string_not_dropped() {
        let payload = strings(&["echo", ""]);
        let result = build_command_line(&payload);
        assert!(result.ends_with("''") || result.ends_with("\"\""));
    }

    #[cfg(not(target_os = "windows"))]
    mod posix {
        use super::*;

        #[test]
        fn multi_word_argument_is_single_quoted() {
            let payload = strings(&["git", "commit", "-m", "fix: my message"]);
            assert_eq!(
                build_command_line(&payload),
                "git commit -m 'fix: my message'"
            );
        }

        #[test]
        fn embedded_single_quote_is_escaped() {
            let payload = strings(&["echo", "it's ok"]);
            assert_eq!(build_command_line(&payload), r#"echo 'it'"'"'s ok'"#);
        }
    }

    #[cfg(target_os = "windows")]
    mod windows {
        use super::*;

        #[test]
        fn multi_word_argument_is_double_quoted() {
            let payload = strings(&["git", "commit", "-m", "fix: my message"]);
            assert_eq!(
                build_command_line(&payload),
                "git commit -m \"fix: my message\""
            );
        }

        #[test]
        fn embedded_double_quote_is_doubled() {
            let payload = strings(&["echo", "say \"hi\" now"]);
            assert_eq!(build_command_line(&payload), "echo \"say \"\"hi\"\" now\"");
        }
    }
}
