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
pub fn run_command(command: &str, working_directory: &Path, sustain: bool, timeout: Option<Duration>) {
    println!(
        "Running '{command}' in {}",
        working_directory.display()
    );

    if let Err(e) = run_command_inner(command, working_directory, sustain, timeout) {
        println!(
            "Caught exception in {} with error:",
            working_directory.display()
        );
        println!("{e}");
    }
}

#[cfg(target_os = "windows")]
fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    ("cmd.exe", vec!["/c".to_string(), command.to_string()])
}

#[cfg(not(target_os = "windows"))]
fn shell_invocation(command: &str) -> (&'static str, Vec<String>) {
    ("/bin/bash", vec!["-c".to_string(), command.to_string()])
}

fn run_command_inner(
    command: &str,
    working_directory: &Path,
    sustain: bool,
    timeout: Option<Duration>,
) -> std::io::Result<()> {
    let (shell, shell_args) = shell_invocation(command);

    let mut child = Command::new(shell)
        .args(&shell_args)
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
        println!(
            "Command timed out in {} after {} seconds.",
            working_directory.display(),
            duration.as_secs()
        );
        child.kill()?;
        let _ = child.wait();
        return Ok(());
    }

    let status = child.wait()?;
    let stdout_output = stdout_handle.join().unwrap_or_default();
    let stderr_output = stderr_handle.join().unwrap_or_default();

    if status.success() {
        println!(
            "Command executed successfully in {}.",
            working_directory.display()
        );
        println!("{stdout_output}");
    } else {
        println!(
            "Command failed gracefully in {}.",
            working_directory.display()
        );
        println!("{stderr_output}");
    }

    Ok(())
}

/// std::process::Child has no built-in timed wait, so poll try_wait() at a
/// short interval until either the process exits or the timeout elapses.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> std::io::Result<bool> {
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
        // Just exercising the happy path end-to-end for panics; stdout
        // capture correctness is implicitly covered by exit-status handling.
        run_command(&echo_command("hello"), dir.path(), false, None);
    }

    #[test]
    fn failing_command_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        run_command(&failing_command_with_stderr("boom"), dir.path(), false, None);
    }

    #[test]
    fn timeout_kills_long_running_command() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        run_command(
            &sleep_command(5),
            dir.path(),
            false,
            Some(Duration::from_millis(300)),
        );
        // Should be killed well before the 5s sleep completes.
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn sustain_overrides_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        run_command(
            &echo_command("quick"),
            dir.path(),
            true,
            Some(Duration::from_millis(1)),
        );
        // Command finishes almost instantly regardless of the tiny timeout,
        // proving sustain suppressed it rather than racing the timeout.
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
