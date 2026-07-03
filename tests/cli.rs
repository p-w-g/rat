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
