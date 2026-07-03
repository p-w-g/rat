pub fn show_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    r#"
Available commands:

    * help          Show help information

    * fep           Run command for nested folders in CWD.
                    takes a list of optional folders to either skip or run command in, separated by '-'
                    `rat fep <<command>> [--skip-foo-bar-baz || --only-gris-gras-gres]`

                    by default runs in current working folder or set working folder,
                    which can be temporarily overrun with `--local` flag

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
