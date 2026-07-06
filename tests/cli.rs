use std::path::Path;
use std::process::Command;

fn rat() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rat"))
}

/// Directories from the component-matching examples: two per country
/// ("priv"/"corp"), all sharing an "app" suffix.
const COUNTRY_APP_DIRS: &[&str] = &[
    "uk-priv-app",
    "uk-corp-app",
    "fi-priv-app",
    "fi-corp-app",
    "nl-priv-app",
    "at-corp-app",
];

fn create_dirs(root: &Path, names: &[&str]) {
    for name in names {
        std::fs::create_dir(root.join(name)).unwrap();
    }
}

/// Runs `fep echo marker` with the given extra flags and returns the subset
/// of `COUNTRY_APP_DIRS` that the run actually reported executing in.
fn run_and_collect_matches(root: &Path, extra_flags: &[&str]) -> Vec<&'static str> {
    let output = rat()
        .args(["fep", "echo", "marker", "--local"])
        .args(extra_flags)
        .current_dir(root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    COUNTRY_APP_DIRS
        .iter()
        .copied()
        .filter(|name| stdout.contains(&root.join(name).display().to_string()))
        .collect()
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

#[test]
fn fep_only_matches_a_directory_name_component() {
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let mut matched = run_and_collect_matches(dir.path(), &["--only-uk"]);
    matched.sort();
    assert_eq!(matched, vec!["uk-corp-app", "uk-priv-app"]);
}

#[test]
fn fep_only_component_shared_by_every_directory_selects_all_of_them() {
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let mut matched = run_and_collect_matches(dir.path(), &["--only-app"]);
    matched.sort();
    let mut expected = COUNTRY_APP_DIRS.to_vec();
    expected.sort();
    assert_eq!(matched, expected);
}

#[test]
fn fep_skip_excludes_a_directory_name_component() {
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let mut matched = run_and_collect_matches(dir.path(), &["--skip-priv"]);
    matched.sort();
    assert_eq!(matched, vec!["at-corp-app", "fi-corp-app", "uk-corp-app"]);
}

#[test]
fn fep_only_and_skip_combine_instead_of_skip_being_ignored() {
    // Regression test for the redesigned --only/--skip interaction: skip
    // used to be ignored entirely whenever only was also given. It now
    // narrows the only-selected set instead.
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let matched = run_and_collect_matches(dir.path(), &["--only-uk", "--skip-corp"]);
    assert_eq!(matched, vec!["uk-priv-app"]);
}

#[test]
fn fep_only_accepts_comma_separated_values_as_or() {
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let mut matched = run_and_collect_matches(dir.path(), &["--only-uk,fi"]);
    matched.sort();
    assert_eq!(
        matched,
        vec!["fi-corp-app", "fi-priv-app", "uk-corp-app", "uk-priv-app"]
    );
}

#[test]
fn fep_skip_accepts_comma_separated_values_as_or() {
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), COUNTRY_APP_DIRS);

    let mut matched = run_and_collect_matches(dir.path(), &["--only-app", "--skip-uk,nl"]);
    matched.sort();
    assert_eq!(matched, vec!["at-corp-app", "fi-corp-app", "fi-priv-app"]);
}

#[test]
fn fep_only_matches_directory_name_not_parent_path_component() {
    // A component matcher must judge each directory by its own name, not by
    // substring search over its full path - a parent folder that happens to
    // contain a filter word must not affect the result. Nest the fixture
    // directories under a parent that shares a token with the filter.
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("uk-workspace");
    std::fs::create_dir(&parent).unwrap();
    create_dirs(&parent, &["api", "web"]);

    let output = rat()
        .args(["fep", "echo", "marker", "--local", "--only-uk"])
        .current_dir(&parent)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("Running 'echo marker'"),
        "expected no directory to match a filter word that only appears in \
         the parent path, got:\n{stdout}"
    );
}

#[test]
fn fep_preserves_a_quoted_multi_word_argument() {
    // Regression test for the argument-quoting bug: `instance.payload.join("
    // ")` used to rebuild the shell command line by rejoining already-split
    // argv elements with a plain space, so a quoted argument like the
    // commit message below (one argv element because it was quoted) turned
    // into three unquoted words once handed to the sub-shell - git happily
    // interpreted "my" and "message" as pathspecs instead of part of the
    // message, and the commit silently failed.
    let workspace = tempfile::tempdir().unwrap();
    let repo = workspace.path().join("repo");
    std::fs::create_dir(&repo).unwrap();

    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("f.txt"), "x").unwrap();
    git(&["add", "f.txt"]);
    git(&[
        "-c",
        "user.email=test@example.com",
        "-c",
        "user.name=test",
        "commit",
        "-q",
        "-m",
        "initial",
    ]);
    std::fs::write(repo.join("f.txt"), "y").unwrap();
    git(&["add", "f.txt"]);

    let output = rat()
        .args([
            "fep",
            "git",
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "commit",
            "-m",
            "fix: my message",
            "--local",
        ])
        .current_dir(workspace.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "rat fep exited non-zero: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );

    let log = Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let subject = String::from_utf8_lossy(&log.stdout);
    assert_eq!(
        subject.trim(),
        "fix: my message",
        "commit message was mangled - quoting was lost when rat rebuilt the \
         command line"
    );
}

#[test]
fn fep_never_runs_inside_a_dot_git_directory() {
    // .git is always excluded, even with no `cfg ignore` configuration at
    // all - a fresh install shouldn't try to shell out inside it on the
    // very first real-world run.
    let dir = tempfile::tempdir().unwrap();
    create_dirs(dir.path(), &["repo-a", ".git"]);

    let output = rat()
        .args(["fep", "echo", "marker", "--local"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains(&dir.path().join("repo-a").display().to_string()));
    assert!(!stdout.contains(&dir.path().join(".git").display().to_string()));
}

#[test]
fn fep_reports_when_no_subdirectories_match() {
    // Regression test: a working folder with nothing to run in (empty, or
    // everything filtered out) used to print nothing at all and exit 0,
    // indistinguishable from "ran successfully everywhere" - a real risk in
    // CI/scripting, this tool's primary use case, where a misconfigured
    // --only or wrong working folder would go unnoticed.
    let dir = tempfile::tempdir().unwrap();

    let output = rat()
        .args(["fep", "echo", "marker", "--local"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No matching subdirectories"),
        "expected a message about no matching subdirectories, got:\n{stdout}"
    );
    assert!(!stdout.contains("Running 'echo marker'"));
}
