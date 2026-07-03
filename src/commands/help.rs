pub fn show_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    r#"
Available commands:

    * help          Show help information

    * fep           Run a shell command inside every immediate subdirectory
                    of the working folder, in parallel.

                    Usage: `rat fep <<command>> [flags]`
                    Example: `rat fep git pull`

                    By default runs in the current working folder, or the
                    folder set with `cfg here`; override that for one call
                    with the `--local` flag.

                    fep's own flags (all optional):

                      --local             use CWD for this run, even if a
                                          default folder is configured
                      --concurrency-4     run at most 4 directories at once
                                          (default: number of CPUs)
                      --only-a-b-c        only run in subfolders whose path
                                          contains "a", "b", or "c"
                      --skip-a-b-c        skip subfolders whose path contains
                                          "a", "b", or "c" (ignored entirely
                                          if --only is also given)
                      --sustain           wait as long as it takes, ignoring
                                          any timeout
                      --timeout-30        timeout this run after 30 seconds,
                                          overriding the configured timeout
                                          (--timeout-0 means a 0-second
                                          timeout, NOT "disabled" - use
                                          --sustain or `cfg nto` for that)

                    IMPORTANT: rat parses these flags out of <<command>>
                    itself, before your command ever runs. Any `--word...`
                    you pass that starts with local/skip/only/sustain/timeout
                    is captured by rat instead of reaching your command, and
                    any other unrecognized `--flag` is silently dropped
                    rather than forwarded. So:

                      `rat fep git merge --skip-commit`
                      -> rat reads "--skip-commit" as its own --skip flag;
                         git never sees --skip-commit at all.

                    Single-dash flags (`-m`, `-rf`, `-n`, ...) are never
                    touched by rat and always reach your command untouched,
                    e.g. `rat fep git commit -m "message"` is safe.

    * cfg (config)
    cfg path        prints out config file's path
    cfg file        prints out config file's content

    cfg here        sets current working directory as a default working directory for future
                    uses with fep, untill it gets unset or new directory is set
    cfg away        unsets default working directory and allows running fep in current working directory

    cfg ignore      adds folders to the permanently ignored list
                    `rat cfg ignore .git .idea .vscode`
    cfg heed        removes folders from the permanently ignored list
                    `rat cfg heed .git .idea .vscode`
                    or
                    `rat cfg heed --all`

    cfg to          sets timeout in seconds
                    `rat cfg to 30`
                    or disables timeout if passed 0 - same as nto
                    `rat cfg to 0`
    cfg nto         disables timeout

"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_references_the_new_binary_name() {
        assert!(help_text().contains("rat fep"));
        assert!(help_text().contains("rat cfg ignore"));
        assert!(help_text().contains("rat cfg heed"));
        assert!(help_text().contains("rat cfg to"));
        // catches leftover `ath <command>` invocations from the C# original
        // without false-positiving on "path", which legitimately contains "ath"
        assert!(!help_text().contains("`ath "));
    }
}
