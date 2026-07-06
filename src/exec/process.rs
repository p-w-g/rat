use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Runs `command` in `working_directory` through the platform shell,
/// mirroring TaskRunner.cs's `RunCommand`. Errors (failed spawn, failed
/// wait, ...) are caught and printed rather than propagated - same as the
/// original's catch-all around the whole operation, since a single
/// directory failing shouldn't stop the others running in parallel.
///
/// `sustain` (wait forever) always overrides `timeout`, matching the
/// original's precedence.
///
/// Returns whether the command completed successfully (spawned, ran to
/// completion within any timeout, and exited with a success status) so
/// callers can aggregate per-directory outcomes into an overall exit code
/// instead of always reporting success regardless of what actually happened.
///
/// The whole per-directory report (header + result + captured output) is
/// assembled into one string and printed with a single `println!` call.
/// Before this, each of those was a separate `println!`, and since multiple
/// directories run concurrently, one directory's header/result/output could
/// end up visually interleaved with another's between those calls -
/// `println!` only holds the stdout lock for the duration of one call, not
/// across several. A single call per directory makes each report an atomic
/// block in the output no matter how many directories are running at once.
pub fn run_command(
    command: &str,
    working_directory: &Path,
    sustain: bool,
    timeout: Option<Duration>,
) -> bool {
    let header = format!("Running '{command}' in {}", working_directory.display());

    match run_command_inner(command, working_directory, sustain, timeout) {
        Ok(outcome) => {
            println!("{header}\n{}", outcome.body);
            outcome.success
        }
        Err(e) => {
            println!(
                "{header}\nCaught exception in {} with error:\n{e}",
                working_directory.display()
            );
            false
        }
    }
}

struct Outcome {
    success: bool,
    body: String,
}

/// Shells tried, in order, to run a command through. `/bin/bash` is the
/// primary target (it's what `--word...`-style compound commands in the
/// README/help text assume), but some minimal environments - notably
/// musl/Alpine-based containers, common in CI - don't ship it at all,
/// only a POSIX `/bin/sh`. Falling back to `/bin/sh` there still lets the
/// documented, POSIX-portable use cases (`git pull`, `npm install`, `&&`
/// chains) work instead of failing outright with "No such file or
/// directory" on every single subdirectory.
#[cfg(not(target_os = "windows"))]
const SHELL_CANDIDATES: &[&str] = &["/bin/bash", "/bin/sh"];

/// cmd.exe is a core, always-present Windows component, so there's no
/// equivalent fallback concern here - kept as a one-element list so
/// `spawn_shell` doesn't need a separate single-shell code path.
#[cfg(target_os = "windows")]
const SHELL_CANDIDATES: &[&str] = &["cmd.exe"];

/// Builds the (not yet spawned) `Command` that runs `command` through
/// `shell`.
///
/// On Windows this deliberately uses `raw_arg` instead of `arg`/`args` for
/// the command text: `Command::arg` applies Rust's own Windows
/// argv-quoting convention to whatever it's given, which would re-escape
/// the quotes `exec::quoting` already built for cmd.exe's benefit (e.g.
/// doubling their embedded `"` a second time), corrupting an
/// already-correct command line. `raw_arg` appends the text to the command
/// line verbatim, which is what a `cmd.exe /c <text>` invocation needs -
/// cmd.exe (and, in turn, whatever program it invokes) does its own
/// parsing of that text, not Rust's.
#[cfg(target_os = "windows")]
fn build_shell_command(shell: &str, command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(shell);
    cmd.arg("/c");
    cmd.raw_arg(command);
    cmd
}

/// On Unix, the spawned shell is put into its own new process group (see
/// `kill_process_tree`'s doc comment for why). Note this is a deliberate
/// trade-off: it also means an interactive Ctrl-C at the terminal, which
/// signals only the terminal's foreground process group, no longer reaches
/// this shell or its descendants the way it would if they shared rat's own
/// group - the same group-isolation that lets us clean up a whole tree on
/// timeout means rat itself has to be the one to do that cleanup, rather
/// than relying on the terminal to signal everything at once.
#[cfg(not(target_os = "windows"))]
fn build_shell_command(shell: &str, command: &str) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(shell);
    cmd.arg("-c").arg(command);
    cmd.process_group(0);
    cmd
}

/// Tries each shell in `candidates`, in order, spawning `command` in
/// `working_directory` through the first one that actually exists.
/// Candidates are only skipped on `NotFound` (the shell binary itself is
/// missing) - any other spawn error (permissions, ...) is reported as-is
/// rather than masked by trying further candidates.
fn spawn_shell(
    command: &str,
    working_directory: &Path,
    candidates: &[&str],
) -> std::io::Result<std::process::Child> {
    let mut last_err = None;
    for shell in candidates {
        let result = build_shell_command(shell, command)
            .current_dir(working_directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match result {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("SHELL_CANDIDATES is never empty"))
}

fn run_command_inner(
    command: &str,
    working_directory: &Path,
    sustain: bool,
    timeout: Option<Duration>,
) -> std::io::Result<Outcome> {
    let mut child = spawn_shell(command, working_directory, SHELL_CANDIDATES)?;

    // Read stdout/stderr on their own threads as the process runs, the same
    // way the original kicked off ReadToEndAsync before waiting - otherwise
    // a chatty child can fill a pipe buffer and deadlock against our wait.
    let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
    let stdout_handle = thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout_pipe.read_to_string(&mut buf);
        buf
    });
    let stderr_handle = thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf);
        buf
    });

    let effective_timeout = if sustain { None } else { timeout };

    let exited = match effective_timeout {
        None => {
            child.wait()?;
            true
        }
        Some(duration) => wait_with_timeout(&mut child, duration)?,
    };

    if !exited {
        let duration = effective_timeout.expect("timeout branch implies Some");
        kill_process_tree(&mut child);
        let _ = child.wait();
        return Ok(Outcome {
            success: false,
            body: format!(
                "Command timed out in {} after {} seconds.",
                working_directory.display(),
                duration.as_secs()
            ),
        });
    }

    let status = child.wait()?;
    let stdout_output = stdout_handle.join().unwrap_or_default();
    let stderr_output = stderr_handle.join().unwrap_or_default();

    let outcome_line = if status.success() {
        format!(
            "Command executed successfully in {}.",
            working_directory.display()
        )
    } else {
        format!(
            "Command failed gracefully in {}.",
            working_directory.display()
        )
    };

    // Both streams are shown regardless of exit status: a command can write
    // useful diagnostics to stdout right before failing, or warnings to
    // stderr while still exiting 0 - previously only the "expected" stream
    // for each outcome was kept and the other was silently discarded,
    // losing exactly the output most useful for figuring out what happened.
    let mut body = outcome_line;
    append_stream(&mut body, "stdout:", &stdout_output);
    append_stream(&mut body, "stderr:", &stderr_output);

    Ok(Outcome {
        success: status.success(),
        body,
    })
}

/// Appends a labeled section for `content` to `body`, unless `content` is
/// empty - the common case of a command that only wrote to one stream
/// shouldn't gain a dangling empty "stderr:" label.
fn append_stream(body: &mut String, label: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    body.push('\n');
    body.push_str(label);
    body.push('\n');
    body.push_str(content);
    if !content.ends_with('\n') {
        body.push('\n');
    }
}

/// Kills `child` and, as best as the platform allows, everything it spawned
/// - not just the shell process itself.
///
/// `child.kill()` alone only terminates the direct child (`bash`/`cmd.exe`);
/// anything *that* process forked (a build tool, a nested `ping`, ...) is
/// left running, orphaned, once the shell dies. A `--timeout` that only
/// kills the wrapper defeats its own purpose for any non-trivial command,
/// since the actual work can keep consuming CPU/network/disk indefinitely
/// after rat has already reported the directory as timed out and moved on.
///
/// - Unix: `shell_command` puts the shell in its own new process group
///   (`process_group(0)`), which any descendants it forks inherit by
///   default; shelling out to `kill -KILL -<pgid>` (negative pid = "the
///   whole group") reaches all of them without touching rat's own group.
/// - Windows: `taskkill /T` walks the same parent-child tree Windows
///   already tracks for every process, so no equivalent setup is needed at
///   spawn time.
///
/// Both shell out to a standard OS utility rather than using raw
/// FFI/job-object APIs, keeping this dependency- and unsafe-free; a failed
/// kill attempt (e.g. the process already exited) is not itself an error
/// here; whatever remains is caught by the `child.wait()` right after.
#[cfg(not(target_os = "windows"))]
fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id();
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .status();
}

#[cfg(target_os = "windows")]
fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id();
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

/// std::process::Child has no built-in timed wait, so poll try_wait() at a
/// short interval until either the process exits or the timeout elapses.
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> std::io::Result<bool> {
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    let start = Instant::now();

    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    const MISSING_SHELL: &str = "definitely-not-a-real-shell-xyz.exe";
    #[cfg(not(target_os = "windows"))]
    const MISSING_SHELL: &str = "/definitely/not/a/real/shell-xyz";

    #[cfg(target_os = "windows")]
    const REAL_SHELL: &str = "cmd.exe";
    #[cfg(not(target_os = "windows"))]
    const REAL_SHELL: &str = "/bin/sh";

    #[test]
    fn falls_back_to_the_next_candidate_when_the_first_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = spawn_shell(
            &echo_command("hi"),
            dir.path(),
            &[MISSING_SHELL, REAL_SHELL],
        )
        .unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn errors_when_every_candidate_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = spawn_shell(&echo_command("hi"), dir.path(), &[MISSING_SHELL]);
        assert!(result.is_err());
    }

    #[cfg(target_os = "windows")]
    fn echo_command(text: &str) -> String {
        format!("echo {text}")
    }

    #[cfg(target_os = "windows")]
    fn failing_command_with_stderr(text: &str) -> String {
        format!("echo {text} 1>&2 & exit 1")
    }

    #[cfg(target_os = "windows")]
    fn success_writing_to_both_streams() -> String {
        "echo out-line & echo err-line 1>&2".to_string()
    }

    #[cfg(target_os = "windows")]
    fn failure_writing_to_both_streams() -> String {
        "echo out-line & echo err-line 1>&2 & exit 1".to_string()
    }

    #[cfg(target_os = "windows")]
    fn sleep_command(seconds: u32) -> String {
        // no built-in `sleep` in cmd.exe; ping is the standard workaround
        // and needs no interactive stdin, unlike `timeout`.
        format!("ping -n {} 127.0.0.1 > nul", seconds + 1)
    }

    #[cfg(not(target_os = "windows"))]
    fn echo_command(text: &str) -> String {
        format!("echo {text}")
    }

    #[cfg(not(target_os = "windows"))]
    fn failing_command_with_stderr(text: &str) -> String {
        format!("echo {text} 1>&2; exit 1")
    }

    #[cfg(not(target_os = "windows"))]
    fn success_writing_to_both_streams() -> String {
        "echo out-line && echo err-line 1>&2".to_string()
    }

    #[cfg(not(target_os = "windows"))]
    fn failure_writing_to_both_streams() -> String {
        "echo out-line && echo err-line 1>&2 && exit 1".to_string()
    }

    #[cfg(not(target_os = "windows"))]
    fn sleep_command(seconds: u32) -> String {
        format!("sleep {seconds}")
    }

    /// A command whose *direct* child (the shell) finishes almost instantly,
    /// but which itself waits on a backgrounded grandchild that sleeps
    /// briefly and then writes `marker`. This mirrors what killing only the
    /// wrapping shell used to leave behind: a grandchild process that isn't
    /// a direct child of the timed-out process. If the marker exists after
    /// waiting well past the grandchild's own sleep, it survived the kill.
    #[cfg(target_os = "windows")]
    fn background_grandchild_then_touch(marker: &std::path::Path) -> String {
        format!(
            "start /B /WAIT cmd /c \"ping -n 5 127.0.0.1 > nul && echo done > {}\"",
            marker.display()
        )
    }

    #[cfg(not(target_os = "windows"))]
    fn background_grandchild_then_touch(marker: &std::path::Path) -> String {
        format!("(sleep 0.5 && touch '{}') & wait", marker.display())
    }

    #[cfg(target_os = "windows")]
    fn grandchild_settle_time() -> Duration {
        Duration::from_secs(5)
    }

    #[cfg(not(target_os = "windows"))]
    fn grandchild_settle_time() -> Duration {
        Duration::from_millis(700)
    }

    #[test]
    fn timeout_kills_the_whole_process_tree_not_just_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");

        let success = run_command(
            &background_grandchild_then_touch(&marker),
            dir.path(),
            false,
            Some(Duration::from_millis(200)),
        );
        assert!(!success);

        // Give the grandchild more time than its own sleep needs; if
        // killing only reached the direct shell (the pre-fix behavior),
        // the grandchild keeps running in the background and eventually
        // writes the marker anyway, well after rat has already returned.
        thread::sleep(grandchild_settle_time());
        assert!(
            !marker.exists(),
            "a grandchild process survived the timeout and wrote its marker file"
        );
    }

    #[test]
    fn successful_command_reports_success() {
        let dir = tempfile::tempdir().unwrap();
        let success = run_command(&echo_command("hello"), dir.path(), false, None);
        assert!(success);
    }

    #[test]
    fn failing_command_reports_failure_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let success = run_command(
            &failing_command_with_stderr("boom"),
            dir.path(),
            false,
            None,
        );
        assert!(!success);
    }

    #[test]
    fn stderr_is_still_reported_when_the_command_succeeds() {
        // Regression test: a command can exit 0 while still writing
        // diagnostics/warnings to stderr - those must not be silently
        // dropped just because the overall command "succeeded".
        let dir = tempfile::tempdir().unwrap();
        let outcome =
            run_command_inner(&success_writing_to_both_streams(), dir.path(), false, None).unwrap();
        assert!(outcome.success);
        assert!(outcome.body.contains("out-line"));
        assert!(outcome.body.contains("err-line"));
    }

    #[test]
    fn stdout_is_still_reported_when_the_command_fails() {
        // Regression test: a command can print useful context to stdout
        // right before failing - that must not be silently dropped just
        // because the overall command "failed".
        let dir = tempfile::tempdir().unwrap();
        let outcome =
            run_command_inner(&failure_writing_to_both_streams(), dir.path(), false, None).unwrap();
        assert!(!outcome.success);
        assert!(outcome.body.contains("out-line"));
        assert!(outcome.body.contains("err-line"));
    }

    #[test]
    fn timeout_kills_long_running_command_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let success = run_command(
            &sleep_command(5),
            dir.path(),
            false,
            Some(Duration::from_millis(300)),
        );
        // Should be killed well before the 5s sleep completes.
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(!success);
    }

    #[test]
    fn sustain_overrides_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let success = run_command(
            &echo_command("quick"),
            dir.path(),
            true,
            Some(Duration::from_millis(1)),
        );
        // Command finishes almost instantly regardless of the tiny timeout,
        // proving sustain suppressed it rather than racing the timeout.
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(success);
    }
}
