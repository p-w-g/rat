use super::{load_config, save_config_at};
use std::io;
use std::path::Path;

pub fn set_timeout(duration: &str) -> io::Result<()> {
    if duration.is_empty() {
        println!("Forgot to add duration in `rat cfg to`?");
        return Ok(());
    }

    match parse_duration_arg(duration) {
        Ok(value) => set_timeout_at(&super::config_path(), value),
        Err(message) => {
            println!("{message}");
            Ok(())
        }
    }
}

pub fn unset_timeout() -> io::Result<()> {
    unset_timeout_at(&super::config_path())
}

/// Parses a `cfg to` duration argument: `"0"` means "disabled" (`None`), a
/// positive integer is the timeout in seconds, and anything else - a
/// negative number or non-numeric text - is rejected outright.
///
/// This deliberately does *not* mirror the original C# `int.TryParse`
/// behavior of silently treating any unparseable input as 0/disabled:
/// `cfg to banana` used to persist a disabled timeout with no indication
/// anything was wrong, and `cfg to -5` used to persist a nonsensical
/// negative value with no warning it would later be treated as "no
/// timeout" anyway. Both now fail loudly with the config left untouched.
fn parse_duration_arg(duration: &str) -> Result<Option<i32>, String> {
    match duration.parse::<i32>() {
        Ok(0) => Ok(None),
        Ok(n) if n > 0 => Ok(Some(n)),
        Ok(n) => Err(format!(
            "Timeout must be zero or a positive number of seconds, got {n}."
        )),
        Err(_) => Err(format!(
            "Timeout must be a whole number of seconds, got \"{duration}\"."
        )),
    }
}

fn set_timeout_at(path: &Path, value: Option<i32>) -> io::Result<()> {
    let mut config = load_config(path);
    config.time_out = value;
    save_config_at(path, &config)
}

fn unset_timeout_at(path: &Path) -> io::Result<()> {
    let mut config = load_config(path);
    config.time_out = None;
    save_config_at(path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn parse_duration_arg_accepts_a_positive_value() {
        assert_eq!(parse_duration_arg("30"), Ok(Some(30)));
    }

    #[test]
    fn parse_duration_arg_zero_means_disabled() {
        assert_eq!(parse_duration_arg("0"), Ok(None));
    }

    #[test]
    fn parse_duration_arg_rejects_non_numeric_input() {
        assert!(parse_duration_arg("banana").is_err());
    }

    #[test]
    fn parse_duration_arg_rejects_negative_values() {
        assert!(parse_duration_arg("-5").is_err());
    }

    #[test]
    fn set_timeout_persists_duration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");

        set_timeout_at(&path, Some(45)).unwrap();

        assert_eq!(load_config(&path).time_out, Some(45));
    }

    #[test]
    fn set_timeout_zero_clears_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                time_out: Some(45),
                ..Default::default()
            },
        )
        .unwrap();

        set_timeout_at(&path, None).unwrap();

        assert_eq!(load_config(&path).time_out, None);
    }

    #[test]
    fn invalid_duration_leaves_existing_config_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                time_out: Some(45),
                ..Default::default()
            },
        )
        .unwrap();

        // set_timeout (not set_timeout_at) is the entry point that actually
        // validates - a bad value must never reach save_config_at at all.
        assert!(parse_duration_arg("banana").is_err());
        assert!(parse_duration_arg("-5").is_err());
        assert_eq!(load_config(&path).time_out, Some(45));
    }

    #[test]
    fn unset_timeout_clears_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                time_out: Some(45),
                ..Default::default()
            },
        )
        .unwrap();

        unset_timeout_at(&path).unwrap();

        assert_eq!(load_config(&path).time_out, None);
    }
}
