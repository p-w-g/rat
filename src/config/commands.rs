use super::{
    print_config, print_config_path, set_ignored_directories, set_timeout, set_working_directory,
    unset_ignored_directories, unset_timeout, unset_working_directory,
};
use crate::cli::ParsedArgs;
use std::io;

/// Dispatches `cfg <subcommand> [args...]`, mirroring ConfigSwitch.cs's
/// `Evaluate`.
///
/// Fix: the original indexed `Instance["PayLoad"][0]` unguarded, so a bare
/// `ath cfg` (no subcommand) crashed with an uncaught KeyNotFoundException
/// instead of printing a usage message. Same issue for `cfg to` with no
/// duration - `Payload[0]` threw before ever reaching SetTimeout's own
/// (otherwise dead) empty-string check. Both now print a usage message.
pub fn evaluate(instance: &ParsedArgs) {
    let Some(command) = instance.payload.first() else {
        println!("Usage: rat cfg <path|file|here|away|ignore|heed|to|nto>");
        return;
    };
    let command = command.to_lowercase();
    let payload = &instance.payload[1..];

    match command.as_str() {
        "path" => print_config_path(),
        "file" => print_config(),
        "here" => report(set_working_directory()),
        "away" => report(unset_working_directory()),
        "ignore" => report(set_ignored_directories(payload)),
        "heed" => {
            let clear_all = instance.options.contains_key("all");
            report(unset_ignored_directories(payload, clear_all));
        }
        "to" => match payload.first() {
            Some(duration) => report(set_timeout(duration)),
            None => println!("Forgot to add duration in `rat cfg to`?"),
        },
        "nto" => report(unset_timeout()),
        _ => println!("Unknown config command: {command} - refer to help (rat help)"),
    }
}

fn report(result: io::Result<()>) {
    if let Err(e) = result {
        println!("Failed to update config: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(payload: &[&str], options: &[(&str, &[&str])]) -> ParsedArgs {
        ParsedArgs {
            payload: payload.iter().map(|s| s.to_string()).collect(),
            options: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn empty_payload_does_not_panic() {
        evaluate(&instance(&[], &[]));
    }

    #[test]
    fn to_with_no_duration_does_not_panic() {
        evaluate(&instance(&["to"], &[]));
    }

    #[test]
    fn unknown_subcommand_does_not_panic() {
        evaluate(&instance(&["bogus"], &[]));
    }
}
