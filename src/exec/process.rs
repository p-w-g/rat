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

/// Builds the `Command` that runs `command` through the platform shell.
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
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd.exe");
    cmd.arg("/c");
    cmd.raw_arg(command);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("/bin/bash");
    cmd.arg("-c").arg(command);
    cmd
}

fn run_command_inner(
    command: &str,
    working_directory: &Path,
    sustain: bool,
    timeout: Option<Duration>,
) -> std::io::Result<Outcome> {
    let mut child = shell_command(command)
        .current_dir(working_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

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
        child.kill()?;
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

    let body = if status.success() {
        format!(
            "Command executed successfully in {}.\n{stdout_output}",
            working_directory.display()
        )
    } else {
        format!(
            "Command failed gracefully in {}.\n{stderr_output}",
            working_directory.display()
        )
    };

    Ok(Outcome {
        success: status.success(),
        body,
    })
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
    fn echo_command(text: &str) -> String {
        format!("echo {text}")
    }

    #[cfg(target_os = "windows")]
    fn failing_command_with_stderr(text: &str) -> String {
        format!("echo {text} 1>&2 & exit 1")
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
    fn sleep_command(seconds: u32) -> String {
        format!("sleep {seconds}")
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
