use super::execution_mode::ExecutionMode;
use crate::cli::filter::FilterExpression;
use crate::cli::{ParsedArgs, dirs};
use crate::config::{self, Config};
use crate::exec::{process, quoting};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
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
        match dirs::assume_working_directory(local_flag, config.default_folder.as_deref()) {
            Ok(dir) => dir,
            Err(e) => {
                println!("Couldn't determine the working directory: {e}");
                return false;
            }
        };

    let ignored = config.ignored_folders.as_deref();
    let filter = FilterExpression::new(
        instance.options.get("only").map(Vec::as_slice),
        instance.options.get("skip").map(Vec::as_slice),
    );

    let available_dirs = match dirs::available_directories(&working_directory, ignored, &filter) {
        Ok(dirs) => dirs,
        Err(e) => {
            println!(
                "Couldn't read subdirectories of {}: {e}",
                working_directory.display()
            );
            return false;
        }
    };

    if available_dirs.is_empty() {
        println!(
            "No matching subdirectories found in {} (after ignore/--only/--skip filtering) - \
             nothing to run.",
            working_directory.display()
        );
        return true;
    }

    let command = quoting::build_command_line(&instance.payload);
    let sustain = instance.options.contains_key("sustain");
    let timeout = resolve_timeout(instance, &config);
    let execution_mode = resolve_execution_mode(instance);

    let results = run_bounded(&available_dirs, execution_mode.concurrency_limit(), |dir| {
        process::run_command(&command, dir, sustain, timeout)
    });
    aggregate(results)
}

/// Determines whether this run is sequential (`--sync`) or bounded-parallel.
/// `--sync` always wins over `--concurrency`: forcing the same scheduler
/// (`run_bounded`) down to a limit of 1 keeps one code path responsible for
/// ordering/joining directories regardless of which mode is active, rather
/// than duplicating a separate "run one at a time" loop.
fn resolve_execution_mode(instance: &ParsedArgs) -> ExecutionMode {
    if instance.options.contains_key("sync") {
        if instance.options.contains_key("concurrency") {
            println!("Ignoring --concurrency: --sync forces one directory at a time.");
        }
        return ExecutionMode::Sync;
    }

    ExecutionMode::Concurrent(resolve_concurrency(instance))
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
/// A fixed pool of `max_concurrency` worker threads share one backlog: each
/// worker atomically claims the next unclaimed index into `dirs`
/// (`next_index.fetch_add`), runs it, and immediately claims another -
/// there's no "chunk boundary" where an idle worker sits waiting for
/// slower siblings before more work becomes available. `dirs` itself needs
/// no locking since it's only ever read; `next_index` is the only thing
/// workers contend on, and that's a single atomic increment.
///
/// A directory's `work` call is wrapped in `catch_unwind` so a panic ends
/// only that directory's turn, not the worker thread itself - unlike the
/// previous one-thread-per-directory design, a worker here goes on to
/// claim further directories over its lifetime, so without this a panic
/// would take an entire worker (not just one directory) out of rotation
/// for the rest of the run. `AssertUnwindSafe` is required because `F` is
/// generic and the compiler can't otherwise prove closures built from it
/// are unwind-safe; this holds for every `work` this function is actually
/// called with, since none of them leave shared state broken on panic -
/// `process::run_command` (the real `work`) already reports its own
/// per-directory errors without panicking, and the closures in this
/// module's tests only touch atomics, which tolerate being read after a
/// panicked write.
fn run_bounded<F>(
    dirs: &[PathBuf],
    max_concurrency: usize,
    work: F,
) -> Vec<(String, thread::Result<bool>)>
where
    F: Fn(&Path) -> bool + Sync,
{
    if dirs.is_empty() {
        return Vec::new();
    }

    let worker_count = max_concurrency.max(1).min(dirs.len());
    let next_index = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(dirs.len()));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                loop {
                    let i = next_index.fetch_add(1, Ordering::SeqCst);
                    let Some(dir) = dirs.get(i) else {
                        break;
                    };
                    let dir_display = dir.display().to_string();
                    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| work(dir)));
                    results.lock().unwrap().push((dir_display, outcome));
                }
            });
        }
    });

    results.into_inner().unwrap()
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
    use std::time::Instant;

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
    fn sync_flag_selects_sync_execution_mode() {
        let instance = instance_with_options(&[("sync", &[])]);
        assert_eq!(resolve_execution_mode(&instance), ExecutionMode::Sync);
    }

    #[test]
    fn sync_flag_wins_over_a_concurrency_flag() {
        let instance = instance_with_options(&[("sync", &[]), ("concurrency", &["8"])]);
        assert_eq!(resolve_execution_mode(&instance), ExecutionMode::Sync);
    }

    #[test]
    fn without_sync_resolves_to_concurrent_mode() {
        let instance = instance_with_options(&[("concurrency", &["3"])]);
        assert_eq!(
            resolve_execution_mode(&instance),
            ExecutionMode::Concurrent(3)
        );
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
        // With a shared backlog rather than fixed batches, results are no
        // longer guaranteed to come back in input order (two workers can
        // finish in either order) - match each directory's outcome by name
        // instead of asserting a fixed result sequence.
        let dirs: Vec<PathBuf> = vec![PathBuf::from("ok"), PathBuf::from("bad")];
        let results = run_bounded(&dirs, 2, |dir| dir.to_string_lossy() != "bad");

        let mut outcomes: HashMap<String, bool> = results
            .into_iter()
            .map(|(dir, result)| (dir, result.unwrap()))
            .collect();
        assert_eq!(outcomes.remove("ok"), Some(true));
        assert_eq!(outcomes.remove("bad"), Some(false));
        assert!(outcomes.is_empty());
    }

    #[test]
    fn run_bounded_isolates_a_panic_to_its_own_directory() {
        // Regression test for the persistent-worker redesign: a worker
        // thread now handles many directories over its lifetime instead of
        // exactly one, so a panic inside `work` must not take the rest of
        // that worker's future directories down with it.
        let dirs: Vec<PathBuf> = (0..6).map(|i| PathBuf::from(format!("dir-{i}"))).collect();

        let results = run_bounded(&dirs, 2, |dir| {
            if dir.to_string_lossy() == "dir-3" {
                panic!("boom");
            }
            true
        });

        assert_eq!(results.len(), 6);
        let mut succeeded = 0;
        let mut panicked = 0;
        for (_, result) in results {
            match result {
                Ok(true) => succeeded += 1,
                Ok(false) => panic!("no directory in this test returns false"),
                Err(_) => panicked += 1,
            }
        }
        assert_eq!(succeeded, 5);
        assert_eq!(panicked, 1);
    }

    #[test]
    fn run_bounded_never_spawns_more_workers_than_directories() {
        // Requesting more concurrency than there is work must not spawn
        // idle worker threads for directories that don't exist.
        let dirs: Vec<PathBuf> = vec![PathBuf::from("only-one")];
        let results = run_bounded(&dirs, 8, |_dir| true);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn run_bounded_handles_empty_input() {
        let results = run_bounded(&[], 4, |_dir| true);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn run_bounded_lets_a_free_worker_pick_up_the_next_directory_immediately() {
        // Regression test for the shared-backlog redesign: directories used
        // to be split into fixed-size chunks, and the next chunk only
        // started once every thread in the current one finished - a single
        // slow directory sharing a chunk with a fast one held up not just
        // its own chunk but every later chunk's start too. Now a free
        // worker immediately claims the next unclaimed directory, so 20
        // fast directories running alongside one slow one (with a second
        // worker free to drain them) finish in roughly the slower of "the
        // one slow directory" and "20 fast ones run back-to-back by one
        // worker" - not "slow" plus several additional chunk-rounds of fast
        // ones serialized behind it on top of that.
        //
        // With chunk size 2: [slow, fast] pairs the slow directory with one
        // fast one (~400ms for that chunk), then the remaining 19 fast
        // directories still need 10 more chunk-rounds (~15ms each) *after*
        // that ~400ms, landing around 550ms+. The shared-backlog version
        // instead has one worker occupied by the slow directory for 400ms
        // while the other worker drains all 20 fast directories
        // (~20 * 15ms = 300ms) in parallel, finishing around 400-450ms.
        let mut dirs = vec![PathBuf::from("slow")];
        dirs.extend((0..20).map(|i| PathBuf::from(format!("fast-{i}"))));

        let start = Instant::now();
        let results = run_bounded(&dirs, 2, |dir| {
            let sleep = if dir.to_string_lossy() == "slow" {
                Duration::from_millis(400)
            } else {
                Duration::from_millis(15)
            };
            thread::sleep(sleep);
            true
        });
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 21);
        assert!(
            elapsed < Duration::from_millis(500),
            "expected the fast directories to finish alongside the slow one \
             rather than serialized behind chunk boundaries, took {elapsed:?}"
        );
    }
}
