use crate::cli::{ParsedArgs, dirs};
use crate::config::{self, Config};
use crate::exec::process;
use std::time::Duration;

/// Runs the fep command's payload in every available subdirectory in
/// parallel, mirroring FEP.cs's `RunParallelAsync`. Uses OS threads (one per
/// directory) rather than an async runtime - these are blocking child-process
/// waits, and this whole tool already decided against pulling in tokio just
/// for that (see the exec::process migration).
///
/// Fix: the original read `Instance["PayLoad"]` unguarded before doing
/// anything else, so `ath fep` (or `ath fep --local` etc.) with no command
/// crashed with an uncaught KeyNotFoundException. Guarded here with a usage
/// message instead.
/// Single-dash tokens are payload as far as cli::parse_instance is concerned
/// (see its own doc comment), so a bare `rat fep -h` would otherwise fall
/// straight through to the shell-out below and try to run the literal
/// command `-h` in every subdirectory instead of showing help. Checked here,
/// before the command is ever assembled, rather than in the parser: it's
/// fep's own payload that's ambiguous (a real wrapped command may legitimately
/// want to pass `-h` through, e.g. `rat fep grep -h pattern`), so only an
/// *entire* payload of just a help alias is treated as "show help".
const HELP_ALIASES: &[&str] = &["-h", "-help"];

/// Returns whether every directory's command completed successfully, so
/// `main` can translate that into the process's exit code. Before this,
/// `run_command`'s result was discarded (`let _ = handle.join()`) and every
/// invocation exited 0 regardless of whether the fanned-out command failed
/// everywhere - unworkable for scripting/CI, which is the tool's primary
/// use case. The usage-message early returns below deliberately still count
/// as success (nothing was asked to run, so nothing failed); a directory
/// listing failure counts as a real failure since the requested work
/// couldn't happen at all.
pub fn run_parallel(instance: &ParsedArgs) -> bool {
    if instance.payload.len() == 1 && HELP_ALIASES.contains(&instance.payload[0].as_str()) {
        println!("Usage: rat fep <<command>> [--skip-foo-bar-baz || --only-gris-gras-gres]");
        return true;
    }

    if instance.payload.is_empty() {
        println!("Usage: rat fep <<command>> [--skip-foo-bar-baz || --only-gris-gras-gres]");
        return true;
    }

    let config = config::get_config();

    let local_flag = instance.options.contains_key("local");
    let working_directory =
        dirs::assume_working_directory(local_flag, config.default_folder.as_deref());

    let ignored = config.ignored_folders.as_deref();
    let skip = instance.options.get("skip").map(Vec::as_slice);
    let only = instance.options.get("only").map(Vec::as_slice);

    let available_dirs = match dirs::available_directories(&working_directory, ignored, skip, only)
    {
        Ok(dirs) => dirs,
        Err(e) => {
            println!(
                "Couldn't read subdirectories of {}: {e}",
                working_directory.display()
            );
            return false;
        }
    };

    let command = instance.payload.join(" ");
    let sustain = instance.options.contains_key("sustain");
    let timeout = resolve_timeout(instance, &config);

    let handles: Vec<_> = available_dirs
        .into_iter()
        .map(|dir| {
            let command = command.clone();
            let dir_display = dir.display().to_string();
            let handle =
                std::thread::spawn(move || process::run_command(&command, &dir, sustain, timeout));
            (dir_display, handle)
        })
        .collect();

    let results = handles
        .into_iter()
        .map(|(dir_display, handle)| (dir_display, handle.join()))
        .collect();
    aggregate(results)
}

/// Reduces each directory's outcome (`Ok(success)`, or `Err(panic payload)`
/// if the worker thread itself panicked) into one overall success flag.
///
/// Before this existed, a panicking worker was invisible: `let _ =
/// handle.join()` discarded the `Err` with no message at all, unlike the
/// explicit "Caught exception in {dir}" path used for ordinary IO errors -
/// a directory could silently vanish from the output. Kept as a plain
/// function over `Vec<(String, thread::Result<bool>)>` rather than a
/// generic worker-pool abstraction so the panic-handling logic itself is
/// unit-testable without needing to actually crash a thread in a test.
fn aggregate(results: Vec<(String, std::thread::Result<bool>)>) -> bool {
    let mut all_succeeded = true;
    for (dir_display, result) in results {
        match result {
            Ok(success) => all_succeeded &= success,
            Err(panic_payload) => {
                all_succeeded = false;
                println!(
                    "Worker for {dir_display} panicked: {}",
                    panic_message(panic_payload.as_ref())
                );
            }
        }
    }
    all_succeeded
}

/// Best-effort extraction of a panic's message. `std::panic::set_hook`
/// already prints the full panic location/backtrace (respecting
/// `RUST_BACKTRACE`) to stderr the moment it happens, independent of this -
/// this is just a directory-scoped summary tying that panic back to which
/// subdirectory it came from, which the default hook has no way to know.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Mirrors the original's timeout precedence: an explicit `--timeout` flag
/// always wins over the configured value and is never blended with it;
/// `sustain` overriding both is handled inside exec::process::run_command.
///
/// Fix: the original did `int.Parse(Instance["timeout"][0])` once the flag
/// was present, which threw uncaught (crashing the whole run) on a missing
/// or non-numeric value. This degrades to "no timeout" with a printed
/// warning instead.
fn resolve_timeout(instance: &ParsedArgs, config: &Config) -> Option<Duration> {
    if let Some(values) = instance.options.get("timeout") {
        return match values.first().and_then(|s| s.parse::<u64>().ok()) {
            Some(secs) => Some(Duration::from_secs(secs)),
            None => {
                println!("Ignoring --timeout: no valid duration given.");
                None
            }
        };
    }

    config
        .time_out
        .and_then(|secs| u64::try_from(secs).ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn instance_with_options(options: &[(&str, &[&str])]) -> ParsedArgs {
        ParsedArgs {
            payload: vec!["echo".to_string()],
            options: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn timeout_flag_wins_over_config() {
        let instance = instance_with_options(&[("timeout", &["10"])]);
        let config = Config {
            time_out: Some(99),
            ..Default::default()
        };
        assert_eq!(
            resolve_timeout(&instance, &config),
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn falls_back_to_config_when_flag_absent() {
        let instance = instance_with_options(&[]);
        let config = Config {
            time_out: Some(45),
            ..Default::default()
        };
        assert_eq!(
            resolve_timeout(&instance, &config),
            Some(Duration::from_secs(45))
        );
    }

    #[test]
    fn none_when_neither_flag_nor_config_present() {
        let instance = instance_with_options(&[]);
        assert_eq!(resolve_timeout(&instance, &Config::default()), None);
    }

    #[test]
    fn invalid_flag_value_degrades_to_no_timeout_instead_of_panicking() {
        let mut options = HashMap::new();
        options.insert("timeout".to_string(), vec!["not-a-number".to_string()]);
        let instance = ParsedArgs {
            payload: vec!["echo".to_string()],
            options,
        };
        let config = Config {
            time_out: Some(45),
            ..Default::default()
        };
        // flag presence still takes precedence over config, even though its
        // value can't be used - it does NOT fall back to the config value.
        assert_eq!(resolve_timeout(&instance, &config), None);
    }

    fn panicked(message: &'static str) -> std::thread::Result<bool> {
        Err(Box::new(message))
    }

    #[test]
    fn aggregate_is_true_when_every_directory_succeeds() {
        let results = vec![("a".to_string(), Ok(true)), ("b".to_string(), Ok(true))];
        assert!(aggregate(results));
    }

    #[test]
    fn aggregate_is_false_when_any_directory_fails() {
        let results = vec![("a".to_string(), Ok(true)), ("b".to_string(), Ok(false))];
        assert!(!aggregate(results));
    }

    #[test]
    fn aggregate_reports_failure_on_panic_instead_of_being_silently_dropped() {
        // Regression test for H3: a panicking worker used to be discarded
        // entirely via `let _ = handle.join()`, so it neither counted as a
        // failure nor printed anything - a directory could silently vanish.
        let results = vec![
            ("dir-a".to_string(), Ok(true)),
            ("dir-b".to_string(), panicked("boom")),
        ];
        assert!(!aggregate(results));
    }

    #[test]
    fn panic_message_extracts_str_panic_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(payload.as_ref()), "boom");
    }

    #[test]
    fn panic_message_extracts_string_panic_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("boom"));
        assert_eq!(panic_message(payload.as_ref()), "boom");
    }

    #[test]
    fn panic_message_falls_back_for_unknown_payload_type() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert_eq!(
            panic_message(payload.as_ref()),
            "<non-string panic payload>"
        );
    }
}
