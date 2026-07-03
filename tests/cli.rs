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
