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
pub fn run_parallel(instance: &ParsedArgs) {
    if instance.payload.is_empty() {
        println!("Usage: rat fep <<command>> [--skip-foo-bar-baz || --only-gris-gras-gres]");
        return;
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
            return;
        }
    };

    let command = instance.payload.join(" ");
    let sustain = instance.options.contains_key("sustain");
    let timeout = resolve_timeout(instance, &config);

    let handles: Vec<_> = available_dirs
        .into_iter()
        .map(|dir| {
            let command = command.clone();
            std::thread::spawn(move || process::run_command(&command, &dir, sustain, timeout))
        })
        .collect();

    for handle in handles {
        let _ = handle.join();
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
}
