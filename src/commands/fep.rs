use crate::cli::{ParsedArgs, dirs};
use crate::config::{self, Config};
use crate::exec::process;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Used when `--concurrency` isn't given and the number of available CPUs
/// can't be determined.
const DEFAULT_MAX_CONCURRENCY: usize = 8;

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
    let max_concurrency = resolve_concurrency(instance);

    let results = run_bounded(&available_dirs, max_concurrency, |dir| {
        process::run_command(&command, dir, sustain, timeout)
    });
    aggregate(results)
}

/// Determines how many directories may run at once. An explicit
/// `--concurrency-N` flag always wins; otherwise falls back to the number
/// of available CPUs (a reasonable default for this kind of fan-out work),
/// or `DEFAULT_MAX_CONCURRENCY` if that can't be determined.
///
/// Fix: previously there was no limit at all - every available directory
/// got its own OS thread (each of which itself spawns a child process plus
/// two pipe-reader threads) simultaneously. A workspace with a few hundred
/// subdirectories could spawn well over a thousand threads/processes in one
/// invocation, with no backpressure.
fn resolve_concurrency(instance: &ParsedArgs) -> usize {
    if let Some(values) = instance.options.get("concurrency") {
        match values.first().and_then(|s| s.parse::<usize>().ok()) {
            Some(n) if n > 0 => return n,
            _ => {
                println!("Ignoring --concurrency: no valid positive value given, using default.")
            }
        }
    }

    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(DEFAULT_MAX_CONCURRENCY)
}

/// Runs `work` once per directory, at most `max_concurrency` at a time, and
/// returns each directory's display path paired with its outcome.
///
/// Directories are processed in fixed-size chunks: the next chunk only
/// starts once every thread in the current one has finished. That's simpler
/// to reason about (and to test deterministically) than a persistent
/// worker-pool/queue, at the cost of some throughput if runtimes are very
/// uneven within a chunk - an acceptable trade since correctness/safety, not
/// peak throughput, is what bounding concurrency is for here.
///
/// Uses `thread::scope` rather than wrapping `work` in an `Arc`: each
/// chunk's scope blocks until every thread spawned inside it has finished,
/// so borrowing `work` and `dir` for the scope's duration is sound without
/// any shared-ownership bookkeeping.
fn run_bounded<F>(
    dirs: &[PathBuf],
    max_concurrency: usize,
    work: F,
) -> Vec<(String, thread::Result<bool>)>
where
    F: Fn(&Path) -> bool + Sync,
{
    let mut results = Vec::with_capacity(dirs.len());
    for chunk in dirs.chunks(max_concurrency.max(1)) {
        thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|dir| {
                    let dir_display = dir.display().to_string();
                    (dir_display, scope.spawn(|| work(dir)))
                })
                .collect();

            for (dir_display, handle) in handles {
                results.push((dir_display, handle.join()));
            }
        });
    }
    results
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

    fn default_concurrency() -> usize {
        thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(DEFAULT_MAX_CONCURRENCY)
    }

    #[test]
    fn resolve_concurrency_uses_explicit_flag_value() {
        let instance = instance_with_options(&[("concurrency", &["3"])]);
        assert_eq!(resolve_concurrency(&instance), 3);
    }

    #[test]
    fn resolve_concurrency_ignores_zero_and_falls_back_to_default() {
        let instance = instance_with_options(&[("concurrency", &["0"])]);
        assert_eq!(resolve_concurrency(&instance), default_concurrency());
    }

    #[test]
    fn resolve_concurrency_ignores_invalid_value_and_falls_back_to_default() {
        let instance = instance_with_options(&[("concurrency", &["not-a-number"])]);
        assert_eq!(resolve_concurrency(&instance), default_concurrency());
    }

    #[test]
    fn resolve_concurrency_falls_back_to_default_when_flag_absent() {
        let instance = instance_with_options(&[]);
        assert_eq!(resolve_concurrency(&instance), default_concurrency());
    }

    #[test]
    fn run_bounded_never_exceeds_the_configured_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dirs: Vec<PathBuf> = (0..6).map(|i| PathBuf::from(format!("dir-{i}"))).collect();
        let current = AtomicUsize::new(0);
        let max_seen = AtomicUsize::new(0);

        let results = run_bounded(&dirs, 2, |_dir| {
            let in_flight = current.fetch_add(1, Ordering::SeqCst) + 1;
            max_seen.fetch_max(in_flight, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(50));
            current.fetch_sub(1, Ordering::SeqCst);
            true
        });

        assert_eq!(results.len(), 6);
        assert!(
            max_seen.load(Ordering::SeqCst) <= 2,
            "expected at most 2 concurrent workers, saw {}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn run_bounded_serializes_when_limit_is_one() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dirs: Vec<PathBuf> = (0..4).map(|i| PathBuf::from(format!("dir-{i}"))).collect();
        let current = AtomicUsize::new(0);
        let max_seen = AtomicUsize::new(0);

        let results = run_bounded(&dirs, 1, |_dir| {
            let in_flight = current.fetch_add(1, Ordering::SeqCst) + 1;
            max_seen.fetch_max(in_flight, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            current.fetch_sub(1, Ordering::SeqCst);
            true
        });

        assert_eq!(results.len(), 4);
        assert_eq!(max_seen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn run_bounded_propagates_directory_results() {
        let dirs: Vec<PathBuf> = vec![PathBuf::from("ok"), PathBuf::from("bad")];
        let results = run_bounded(&dirs, 2, |dir| dir.to_string_lossy() != "bad");

        let outcomes: Vec<bool> = results
            .into_iter()
            .map(|(_, result)| result.unwrap())
            .collect();
        assert_eq!(outcomes, vec![true, false]);
    }
}
