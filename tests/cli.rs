use std::process::Command;

fn rat() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rat"))
}

#[test]
fn no_args_prints_usage() {
    let output = rat().output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: rat <command> [arguments]"));
}

#[test]
fn help_aliases_all_show_help() {
    for arg in ["help", "h", "-h", "--h", "-help", "--help", "HELP"] {
        let output = rat().arg(arg).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Available commands"),
            "expected help text for `rat {arg}`, got: {stdout}"
        );
    }
}

#[test]
fn unknown_command_is_reported_not_a_crash() {
    let output = rat().arg("bogus").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("Unknown command: bogus"));
}

#[test]
fn cfg_path_runs_successfully() {
    let output = rat().args(["cfg", "path"]).output().unwrap();
    assert!(output.status.success());
}

#[test]
fn cfg_with_no_subcommand_prints_usage_instead_of_crashing() {
    let output = rat().arg("cfg").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: rat cfg"));
}

#[test]
fn fep_with_no_command_prints_usage_instead_of_crashing() {
    let output = rat().arg("fep").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: rat fep"));
}

#[test]
fn fep_exits_zero_when_every_directory_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo-a")).unwrap();
    std::fs::create_dir(dir.path().join("repo-b")).unwrap();

    let output = rat()
        .args(["fep", "exit", "0", "--local"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn fep_exits_nonzero_when_a_directory_fails() {
    // Regression test for C1: previously, run_command's per-directory
    // outcome was discarded entirely (`let _ = handle.join()`), so `rat`
    // always exited 0 even when the fanned-out command failed in every
    // single directory - unworkable for CI/scripting use, which is the
    // tool's primary purpose.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo-a")).unwrap();
    std::fs::create_dir(dir.path().join("repo-b")).unwrap();

    let output = rat()
        .args(["fep", "exit", "1", "--local"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
}

#[test]
fn fep_dash_h_shows_usage_instead_of_running_as_a_command() {
    // Regression test: `-h` is a single-dash token, so cli::parse_instance
    // puts it in the payload rather than treating it as a flag. Before the
    // fix, fep would take that payload and shell out the literal command
    // `-h` in every subdirectory instead of showing help.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo-a")).unwrap();

    let output = rat()
        .args(["fep", "-h", "--local"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: rat fep"));
    assert!(
        !stdout.contains("Running"),
        "fep should not have attempted to run anything, got: {stdout}"
    );
}

#[cfg(target_os = "windows")]
fn multi_line_command() -> &'static str {
    // no built-in `sleep` in cmd.exe; a 1-count ping is the standard
    // workaround and needs no interactive stdin, unlike `timeout`. The
    // pauses widen the window during which concurrent directories' output
    // would interleave if reports weren't printed atomically.
    "echo line-one && ping -n 1 127.0.0.1 > nul && echo line-two && ping -n 1 127.0.0.1 > nul && echo line-three"
}

#[cfg(not(target_os = "windows"))]
fn multi_line_command() -> &'static str {
    "echo line-one && sleep 0.05 && echo line-two && sleep 0.05 && echo line-three"
}

#[test]
fn fep_directory_reports_are_not_interleaved() {
    // Regression test for M3: each directory's report used to be printed
    // via several separate `println!` calls (header, then result, then
    // captured output). `println!` only holds the stdout lock for one call,
    // not across several, so with directories genuinely running at once,
    // one directory's lines could end up interleaved with another's. Now
    // each directory's whole report is built into one string and printed
    // with a single `println!`, so it must appear as one contiguous block.
    let dir = tempfile::tempdir().unwrap();
    let labels = ["alpha", "beta"];
    for label in labels {
        std::fs::create_dir(dir.path().join(label)).unwrap();
    }

    let output = rat()
        .arg("fep")
        .args(multi_line_command().split_whitespace())
        .args(["--local", "--concurrency-2"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let report_blocks: Vec<&str> = stdout.split("Running '").skip(1).collect();
    assert_eq!(
        report_blocks.len(),
        labels.len(),
        "expected one report per directory, got:\n{stdout}"
    );

    for block in &report_blocks {
        let header_end = block.find('\n').expect("report has a header line");
        let header = &block[..header_end];
        let (_, this_dir) = header
            .split_once("' in ")
            .expect("header is \"<command>' in <path>\"");

        for label in labels {
            let other_dir = dir.path().join(label).display().to_string();
            if other_dir != this_dir {
                assert!(
                    !block.contains(&other_dir),
                    "report for {this_dir} unexpectedly mentions {other_dir} - reports are interleaved:\n{stdout}"
                );
            }
        }
    }
}
